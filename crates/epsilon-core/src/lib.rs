//! The Epsilon agent runtime.
//!
//! Ports pi's two-level loop (steering inner / follow-up outer), adds three named
//! improvements at defined seams: a bounded parallel tool pool, fail-all-on-truncation
//! (from pi itself), and the doom-loop brake (opencode + DSH).
//!
//! Everything the loop does is observable through [`AgentEvent`]s on the broadcast
//! channel; frontends are reducers over that stream.

mod compact;
pub mod config;
mod exec;
mod gate;
pub mod prompt;
mod registry;
pub mod skills;

#[doc(hidden)]
pub mod testing;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use epsilon_contracts::content::UserContent;
use epsilon_contracts::session_entry::EntryKind;
use epsilon_session::SessionStore;

/// Token estimate for one message (delegates to compact's heuristic).
fn estimate_one(m: &Message) -> usize {
    crate::compact::estimate_messages(std::slice::from_ref(m))
}

fn clamp_tail_tokens(v: usize) -> usize {
    v.clamp(
        crate::compact::MIN_TAIL_TOKENS,
        crate::compact::MAX_TAIL_TOKENS,
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// ISO-ish date via civil-from-days (Hinnant); no chrono dependency needed.
fn today_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

use epsilon_contracts::agent_event::{AgentEvent, TurnEndReason};
use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
use epsilon_contracts::stream::{StreamEvent, StreamFolder};

pub use gate::{Authorization, AutoAllow, AutoDeny, PermissionGate};
pub use registry::ToolRegistry;

/// How many queued messages a drain point releases.
///
/// Ported verbatim from pi's `QueueMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    /// Release everything queued.
    All,
    /// Release only the oldest; the rest wait for the next drain point.
    OneAtATime,
}

#[derive(Debug)]
struct PendingQueue {
    inner: VecDeque<Message>,
    mode: QueueMode,
}

impl PendingQueue {
    fn new(mode: QueueMode) -> Self {
        Self {
            inner: VecDeque::new(),
            mode,
        }
    }

    fn push(&mut self, m: Message) {
        self.inner.push_back(m);
    }

    fn drain(&mut self) -> Vec<Message> {
        match self.mode {
            QueueMode::All => self.inner.drain(..).collect(),
            QueueMode::OneAtATime => self.inner.pop_front().into_iter().collect(),
        }
    }
}

/// Loop tuning knobs (defaults from the reference audit; see ADR-002/010).
#[derive(Debug, Clone)]
pub struct LoopLimits {
    /// Upper bound on concurrently executing parallel-safe tool calls (DSH's pool cap).
    pub max_parallel_tool_calls: usize,
    /// Consecutive identical calls before the doom-loop gate asks (opencode: 3).
    pub doom_loop_threshold: usize,
    /// Steering queue drain behavior.
    pub steering_mode: QueueMode,
    /// Follow-up queue drain behavior.
    pub followup_mode: QueueMode,
    /// Maximum assistant turns in ONE run (budget guard, Hermes-pattern).
    pub max_turns: usize,
    /// Compact when estimated tokens exceed this before a provider call.
    pub compaction_threshold_tokens: usize,
    /// Recent-message budget protected from pruning/clamping during compaction.
    pub compaction_tail_tokens: usize,
}

impl Default for LoopLimits {
    fn default() -> Self {
        Self {
            max_parallel_tool_calls: 8,
            doom_loop_threshold: 3,
            steering_mode: QueueMode::OneAtATime,
            followup_mode: QueueMode::All,
            max_turns: 64,
            compaction_threshold_tokens: 120_000,
            compaction_tail_tokens: 12_000,
        }
    }
}

/// Outcome of one completed run.
#[derive(Debug)]
pub struct RunOutcome {
    /// Messages produced during the run (assistant + tool results + injected steering),
    /// excluding the original prompt.
    pub produced: Vec<Message>,
    /// Sticky terminal reason (max_tokens beats later clean steps, DSH rule).
    pub reason: TurnEndReason,
    /// Total provider usage across the run.
    pub usage: Usage,
}

/// Heuristic overflow classifier (DSH's CONTEXT_WINDOW_EXCEEDED, message-shaped).
fn looks_like_context_overflow(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("context window")
        || m.contains("context_length")
        || m.contains("maximum context")
        || m.contains("too many tokens")
        || m.contains("request too large")
        || m.contains("context_window_exceeded")
}

/// Errors a run can surface to its caller.
#[derive(Debug)]
pub enum RunError {
    /// Another run is active on this agent.
    AlreadyRunning,
    /// Internal invariant violation (provider failures arrive in-band instead).
    Internal(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::AlreadyRunning => write!(f, "another run is already active"),
            RunError::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for RunError {}

/// The agent: owns the transcript, the queues, and the event bus.
pub struct Agent {
    limits: LoopLimits,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    tools: Arc<ToolRegistry>,
    gate: Arc<dyn PermissionGate>,

    history: Mutex<Vec<Message>>,
    steering: Mutex<PendingQueue>,
    followups: Mutex<PendingQueue>,

    events_tx: broadcast::Sender<Arc<AgentEvent>>,
    running: AtomicBool,
    /// Run-scoped doom-loop counters; cleared when a run starts (see exec.rs).
    doom_streaks: Mutex<HashMap<String, usize>>,
    /// Declared workspace root; path arguments resolving outside it get gated under the
    /// xternal_dir capability instead of their tool name.
    pub(crate) workspace_root: std::path::PathBuf,
    /// Durable mirror of this agent's transcript; compaction entries land here and the
    /// provider context switches to the store's shadowed model view when attached.
    session_store: Mutex<Option<Arc<Mutex<SessionStore>>>>,
    /// Set when an overflow-triggered compaction already retried this run.
    overflow_retried: AtomicBool,
    /// Discovered skills (catalog in CONTEXT tier; bodies load on demand via skill tool).
    /// Interior-mutable so composition code can attach discoveries after Arc<Agent>.
    skills: Mutex<Vec<crate::skills::Skill>>,
    /// Dedicated auxiliary stream for summarization (isolated from the main loop's
    /// provider/script; pi isolates aux calls for cache reasons, we ALSO isolate scripts).
    summarizer: Mutex<Option<Arc<dyn epsilon_llm::StreamFn>>>,
    /// Model identifier sent to the provider and rendered in the VOLATILE prompt tier
    /// ("faux-1" until overridden — real providers route on this).
    model_id: Mutex<String>,
    /// Token for the ACTIVE run; cancelled by [`Agent::abort_current_run`]. Tools
    /// receive clones through their `ToolContext` so long-running work bails
    /// cooperatively (M12: closes the never-cancelled-token gap, ADR-010).
    run_token: Mutex<Option<tokio_util::sync::CancellationToken>>,
}

impl Agent {
    /// Create an agent. Wrap raw providers in [`epsilon_llm::PanicGuard`] before passing.
    pub fn new(
        limits: LoopLimits,
        stream_fn: Arc<dyn epsilon_llm::StreamFn>,
        tools: Arc<ToolRegistry>,
        gate: Arc<dyn PermissionGate>,
        workspace_root: std::path::PathBuf,
    ) -> Arc<Self> {
        let (events_tx, _) = broadcast::channel(1024);
        let steering_queue = PendingQueue::new(limits.steering_mode);
        let followup_queue = PendingQueue::new(limits.followup_mode);
        Arc::new(Self {
            limits,
            stream_fn,
            tools,
            gate,
            history: Mutex::new(Vec::new()),
            steering: Mutex::new(steering_queue),
            followups: Mutex::new(followup_queue),
            events_tx,
            running: AtomicBool::new(false),
            doom_streaks: Mutex::new(HashMap::new()),
            workspace_root,
            session_store: Mutex::new(None),
            overflow_retried: AtomicBool::new(false),
            summarizer: Mutex::new(None),
            skills: Mutex::new(Vec::new()),
            model_id: Mutex::new("faux-1".into()),
            run_token: Mutex::new(None),
        })
    }

    /// Override the model identifier (M10): providers route via the catalog's wire
    /// column and resolve auth from env vars at call time
    /// per family from standard env vars; the id also renders into the VOLATILE tier.
    pub fn set_model_id(&self, model_id: impl Into<String>) {
        if let Ok(mut slot) = self.model_id.lock() {
            *slot = model_id.into();
        }
    }

    /// Attach discovered skills for the CONTEXT-tier catalog (M11 wiring).
    pub fn set_skills(&self, skills: Vec<crate::skills::Skill>) {
        if let Ok(mut slot) = self.skills.lock() {
            *slot = skills;
        }
    }

    fn skill_list(&self) -> Vec<crate::skills::Skill> {
        self.skills.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Names of discovered skills (for `/skills` and diagnostics).
    pub fn skill_names(&self) -> Vec<String> {
        self.skill_list().into_iter().map(|s| s.name).collect()
    }

    /// Seed the transcript from a previous session (M12 `--resume`).
    ///
    /// Messages are restored into runtime history WITHOUT re-persisting (they already
    /// live in the durable mirror); subsequent messages append normally. Call before
    /// the first `run()`.
    pub fn restore_history(&self, messages: Vec<Message>) {
        if let Ok(mut h) = self.history.lock() {
            *h = messages;
        }
    }

    /// Number of restored transcript entries (diagnostics after `restore_history`).
    pub fn restored_len(&self) -> usize {
        self.history.lock().map(|h| h.len()).unwrap_or(0)
    }

    /// Provide a dedicated auxiliary provider for summarization calls.
    pub fn set_summarizer(&self, s: Arc<dyn epsilon_llm::StreamFn>) {
        if let Ok(mut slot) = self.summarizer.lock() {
            *slot = Some(s);
        }
    }

    fn summarizer_stream(&self) -> Arc<dyn epsilon_llm::StreamFn> {
        self.summarizer
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .unwrap_or_else(|| Arc::clone(&self.stream_fn))
    }

    /// Subscribe to the lifecycle event stream (TUI, server SSE, telemetry all consume this).
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<AgentEvent>> {
        self.events_tx.subscribe()
    }

    /// Queue a mid-run interjection; drained at the next drain point (after current turn).
    pub fn steer(&self, m: Message) {
        if let Ok(mut q) = self.steering.lock() {
            q.push(m);
        }
    }

    /// Queue work for after the agent would otherwise stop.
    pub fn follow_up(&self, m: Message) {
        if let Ok(mut q) = self.followups.lock() {
            q.push(m);
        }
    }
    /// Attach a durable session mirror. Once attached:
    /// - every transcript message is appended to the JSONL tree,
    /// - compaction entries are persisted,
    /// - the provider context comes from the store's SHADOWED model view, so summaries
    ///   shrink what the model sees while the human log stays complete.
    pub fn attach_session_store(&self, store: Arc<Mutex<SessionStore>>) {
        if let Ok(mut slot) = self.session_store.lock() {
            *slot = Some(store);
        }
    }

    fn store(&self) -> Option<Arc<Mutex<SessionStore>>> {
        self.session_store.lock().ok().and_then(|s| s.clone())
    }

    /// Test/inspection handle to the attached durable mirror.
    #[doc(hidden)]
    pub fn store_handle_for_tests(&self) -> Arc<Mutex<SessionStore>> {
        self.store().expect("no session store attached")
    }

    /// Append a message to the durable mirror when one is attached.
    fn persist(&self, m: &Message) {
        if let Some(store) = self.store()
            && let Ok(mut st) = store.lock()
        {
            let _ = st.append(EntryKind::Message {
                message: m.clone(),
                api_request_sha256: None,
            });
        }
    }

    /// Remove the most recently pushed message (overflow-retry discards failed attempts).
    fn pop_last_message(&self) {
        if let Ok(mut h) = self.history.lock() {
            h.pop();
        }
        if let Ok(store) = self.session_store.lock()
            && let Some(st) = store.as_ref()
            && let Ok(mut s) = st.lock()
        {
            let _ = s.pop_last();
        }
    }

    /// Current transcript snapshot (all roles, including harness-only messages).
    pub fn history(&self) -> Vec<Message> {
        self.history.lock().map(|h| h.clone()).unwrap_or_default()
    }

    fn push_message(&self, m: &Message) {
        if let Ok(mut h) = self.history.lock() {
            h.push(m.clone());
        }
        self.persist(m);
    }

    fn emit(&self, ev: AgentEvent) {
        let _ = self.events_tx.send(Arc::new(ev));
    }

    /// Provider-visible conversion (pi's `convertToLlm` default): drop harness-only messages.
    fn convert_to_llm(messages: &[Message]) -> Vec<Message> {
        messages
            .iter()
            .filter(|m| m.is_llm_visible())
            .cloned()
            .collect()
    }

    /// Run one prompt to completion.
    ///
    /// Returns `AlreadyRunning` rather than queueing — concurrent runs on one agent are a
    /// caller bug, not a workload to silently serialize.
    pub async fn run(self: &Arc<Self>, prompt: Message) -> Result<RunOutcome, RunError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(RunError::AlreadyRunning);
        }

        // Fresh run-scoped token (M12): tools observe it via ToolContext; abort ends
        // the run cooperatively instead of only via process exit.
        let token = tokio_util::sync::CancellationToken::new();
        if let Ok(mut slot) = self.run_token.lock() {
            *slot = Some(token.clone());
        }

        if let Ok(mut s) = self.doom_streaks.lock() {
            s.clear();
        }
        self.push_message(&prompt);
        self.emit(AgentEvent::AgentStart);

        let outcome = self.run_inner(prompt).await;

        if let Ok(mut slot) = self.run_token.lock() {
            *slot = None;
        }
        self.emit(AgentEvent::AgentEnd {
            messages: outcome
                .as_ref()
                .map(|o| o.produced.clone())
                .unwrap_or_default(),
        });
        self.running.store(false, Ordering::SeqCst);
        outcome
    }

    /// Cancel the active run's token: in-flight tools see cancellation and the loop
    /// stops at the next checkpoint with `TurnEndReason::Aborted`.
    ///
    /// Returns false when no run is active. Idempotent.
    pub fn abort_current_run(&self) -> bool {
        if let Ok(slot) = self.run_token.lock()
            && let Some(token) = slot.as_ref()
        {
            token.cancel();
            return true;
        }
        false
    }

    /// Clone of the active run's token for `ToolContext` distribution.
    fn current_run_token(&self) -> Option<tokio_util::sync::CancellationToken> {
        self.run_token
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
    }

    async fn run_inner(self: &Arc<Self>, _prompt: Message) -> Result<RunOutcome, RunError> {
        let mut produced: Vec<Message> = Vec::new();
        let mut usage_total = Usage::default();
        let mut reason = TurnEndReason::Completed;
        let mut turn_index: usize = 0;

        // Prefilled steering covers "user typed while we were starting".
        let mut pending = self.drain_steering();

        'outer: loop {
            let mut has_more_tools = true;

            while has_more_tools || !pending.is_empty() {
                // Abort checkpoint (M12): a cancelled token ends the run cooperatively.
                if let Some(token) = self.current_run_token()
                    && token.is_cancelled()
                {
                    reason = reason.merge_sticky(TurnEndReason::Aborted);
                    break 'outer;
                }

                if turn_index >= self.limits.max_turns {
                    reason = reason.merge_sticky(TurnEndReason::BudgetExhausted);
                    break 'outer;
                }

                // Drain point: inject queued messages before the next model call.
                for m in pending.drain(..) {
                    self.push_message(&m);
                    produced.push(m.clone());
                }

                // ---- compaction pressure check (ADR-007 trigger 1) ----------------
                if self.compaction_due() {
                    self.compact_now(&mut produced).await?;
                }

                self.emit(AgentEvent::TurnStart { turn_index });

                // ---- one provider turn --------------------------------------------
                let assistant = match self.stream_one_turn(&mut usage_total).await {
                    Ok(a) => a,
                    Err(e) => return Err(e),
                };
                let m = Message::Assistant(assistant.clone());
                produced.push(m);

                // ---- overflow recovery (ADR-007 trigger #2): compact once, retry ----
                if assistant.stop_reason == StopReason::Error
                    && assistant
                        .error_message
                        .as_deref()
                        .is_some_and(looks_like_context_overflow)
                    && !self.overflow_retried.swap(true, Ordering::SeqCst)
                {
                    // Discard the failed attempt, shrink, and retry this turn.
                    self.pop_last_message();
                    produced.pop();
                    self.compact_now(&mut produced).await?;
                    continue;
                }

                if assistant.stop_reason.is_failure() {
                    reason = reason.merge_sticky(match assistant.stop_reason {
                        StopReason::Aborted => TurnEndReason::Aborted,
                        _ => TurnEndReason::Error(
                            assistant.error_message.clone().unwrap_or_default(),
                        ),
                    });
                    self.emit(AgentEvent::TurnEnd {
                        turn_index,
                        message: Box::new(Message::Assistant(assistant)),
                        tool_results: Vec::new(),
                    });
                    break 'outer;
                }

                // ---- tool batch -----------------------------------------------------
                let call_count = assistant.tool_calls().count();
                let mut results: Vec<epsilon_contracts::message::ToolResultMessage> = Vec::new();
                has_more_tools = false;

                if call_count > 0 {
                    let batch = if assistant.stop_reason == StopReason::MaxTokens {
                        // pi's fail-all rule: truncated arguments are unsafe to execute.
                        exec::fail_all_truncated(&assistant)
                    } else {
                        exec::execute_batch(self, &assistant).await?
                    };
                    reason = reason.merge_sticky(batch.reason);
                    has_more_tools = !batch.terminate;
                    results = batch.results;
                }

                for r in &results {
                    let m = Message::ToolResult(r.clone());
                    // pi emits full message lifecycle for tool results too — frontends
                    // render from these events, not by scraping internals.
                    self.emit(AgentEvent::MessageStart {
                        message: Box::new(m.clone()),
                    });
                    self.push_message(&m);
                    produced.push(m);
                    self.emit(AgentEvent::MessageEnd {
                        message: Box::new(Message::ToolResult(r.clone())),
                    });
                }

                self.emit(AgentEvent::TurnEnd {
                    turn_index,
                    message: Box::new(Message::Assistant(assistant)),
                    tool_results: results,
                });
                turn_index += 1;

                pending = self.drain_steering();
            }

            // The agent would stop here; honor queued follow-ups before exiting.
            let followups = self.drain_followups();
            if followups.is_empty() {
                break;
            }
            pending = followups;
        }

        Ok(RunOutcome {
            produced,
            reason,
            usage: usage_total,
        })
    }

    fn drain_steering(&self) -> Vec<Message> {
        self.steering
            .lock()
            .map(|mut q| q.drain())
            .unwrap_or_default()
    }

    fn drain_followups(&self) -> Vec<Message> {
        self.followups
            .lock()
            .map(|mut q| q.drain())
            .unwrap_or_default()
    }

    /// Pressure trigger (ADR-007 #1): estimated context exceeds the threshold.
    fn compaction_due(&self) -> bool {
        let tokens = crate::compact::estimate_messages(&self.history());
        tokens > self.limits.compaction_threshold_tokens
    }

    /// Run one full compaction pass: prune → summarize (isolated call, amend-mode when a
    /// prior summary exists) → replace runtime history → persist entry + emit events.
    pub async fn compact_now(
        self: &Arc<Self>,
        produced: &mut Vec<Message>,
    ) -> Result<(), RunError> {
        use crate::compact;
        self.emit(AgentEvent::CompactionStarted);

        let history = self.history();
        let tail = clamp_tail_tokens(self.limits.compaction_tail_tokens);
        let (pruned_hist, pruned_count) = compact::prune_old_tool_results(&history, tail);

        // Split into [compactable | protected tail] by token budget from the END.
        let mut acc = 0usize;
        let mut split = pruned_hist.len();
        for i in (0..pruned_hist.len()).rev() {
            acc += estimate_one(&pruned_hist[i]);
            if acc >= tail {
                split = i;
                break;
            }
            split = i;
        }
        let (middle, kept_tail): (&[Message], &[Message]) = if split >= pruned_hist.len() {
            (&[], &pruned_hist[..])
        } else {
            (&pruned_hist[..split], &pruned_hist[split..])
        };

        // Amend mode when a previous summary is already in the middle range.
        let prior = middle.iter().find_map(|m| match m {
            Message::User(u)
                if u.content.iter().any(|c| {
                    matches!(c,
                    UserContent::Text { text } if text.starts_with("<compaction_summary>"))
                }) =>
            {
                u.content.iter().find_map(|c| match c {
                    UserContent::Text { text } => Some(text.clone()),
                    _ => None,
                })
            }
            _ => None,
        });

        // Isolated summarization call (fresh request; no tools).
        let sctx = compact::summary_context(middle, prior.as_deref());
        let model_id = self
            .model_id
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "faux-1".into());
        let model = epsilon_llm::ModelRef::new(model_id);
        let mut stream = self.summarizer_stream().stream(&model, sctx).await;
        let answer = loop {
            match stream.next().await {
                Some(StreamEvent::Done { message }) => break message,
                Some(_) => continue,
                None => {
                    return Err(RunError::Internal("summarizer stream ended early".into()));
                }
            }
        };
        let summary = compact::summary_text(&answer);

        // New runtime history: summary-as-context-message + retained tail.
        let mut new_history = Vec::with_capacity(kept_tail.len() + 1);
        new_history.push(Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: now_ms(),
            content: vec![UserContent::Text {
                text: format!("<compaction_summary>\n{summary}\n</compaction_summary>"),
            }],
        }));
        new_history.extend(kept_tail.iter().cloned());

        let estimate_messages_before: usize = crate::compact::estimate_messages(&history);
        // Persist a compaction entry so the durable mirror's MODEL VIEW shadows the range.
        if let Some(store) = self.store()
            && let Ok(mut st) = store.lock()
        {
            let _ = st.append(EntryKind::Compaction {
                summary: summary.clone(),
                tokens_before: estimate_messages_before as u64,
                retained_tail: kept_tail.to_vec(),
                details: Some(serde_json::json!({ "pruned_tool_results": pruned_count })),
                usage: None,
            });
        }

        *self
            .history
            .lock()
            .map_err(|_| RunError::Internal("history poisoned".into()))? = new_history;
        produced.push(Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: now_ms(),
            content: vec![UserContent::Text {
                text: "[context compacted]".into(),
            }],
        }));
        self.emit(AgentEvent::CompactionFinished);
        Ok(())
    }

    async fn stream_one_turn(
        self: &Arc<Self>,
        usage_total: &mut Usage,
    ) -> Result<AssistantMessage, RunError> {
        // Provider-visible view: store shadowed view when attached, else runtime history.
        let history: Vec<Message> = match self.store() {
            Some(store) => {
                let st = store
                    .lock()
                    .map_err(|_| RunError::Internal("store poisoned".into()))?;
                st.model_view()
                    .map_err(|e| RunError::Internal(format!("session view: {e}")))?
                    .into_iter()
                    .map(|v| v.into_message())
                    .collect()
            }
            None => self.history(),
        };

        // ---- three-tier system prompt (ADR-008) ------------------------------------
        let tool_lines: Vec<prompt::ToolLine> = self
            .tools
            .descriptors()
            .iter()
            .map(|d| prompt::ToolLine {
                name: d.name.clone(),
                snippet: d.description.lines().next().unwrap_or("").to_string(),
            })
            .collect();
        let model_id = self
            .model_id
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "faux-1".into());
        let system_prompt = prompt::build(
            &prompt::Stable { tools: &tool_lines },
            &prompt::ContextTier {
                instructions: prompt::load_project_instructions(&self.workspace_root),
                skills_catalog: Some(crate::skills::catalog(&self.skill_list())),
            },
            &prompt::Volatile {
                cwd: &self.workspace_root,
                model_id: &model_id,
                today: today_string(),
            },
        );

        let ctx = epsilon_llm::LlmContext {
            system_prompt,
            messages: Self::convert_to_llm(&history),
            tools: self.tools.descriptors(),
        };

        let model = epsilon_llm::ModelRef::new(model_id);
        let mut stream = self.stream_fn.stream(&model, ctx).await;
        let mut folder = StreamFolder::default();

        while let Some(ev) = stream.next().await {
            match ev {
                StreamEvent::Done { message } => {
                    *usage_total = usage_total.combine(message.usage);
                    self.push_message(&Message::Assistant(message.clone()));
                    self.emit(AgentEvent::MessageEnd {
                        message: Box::new(Message::Assistant(message.clone())),
                    });
                    return Ok(message);
                }
                other => {
                    // Fold for live partials; Done handled above.
                    //
                    // Broadcast discipline: argument-fragment deltas are pure transport
                    // noise (hundreds per call on big payloads) — they'd overflow slow
                    // subscribers' rings for zero UI value, so they fold WITHOUT being
                    // broadcast. Visible content still streams live.
                    if matches!(other, StreamEvent::ToolCallArgumentsDelta { .. }) {
                        let _ = folder.apply(other);
                    } else if folder.apply(other).is_some()
                        && let Some(partial) = folder.partial()
                    {
                        self.emit(AgentEvent::MessageUpdate {
                            message: Box::new(Message::Assistant(partial.clone())),
                        });
                    }
                }
            }
        }
        Err(RunError::Internal(
            "provider stream ended without Done".into(),
        ))
    }
}
