//! The Epsilon agent runtime.
//!
//! Ports pi's two-level loop (steering inner / follow-up outer), adds three named
//! improvements at defined seams: a bounded parallel tool pool, fail-all-on-truncation
//! (from pi itself), and the doom-loop brake (opencode + DSH).
//!
//! Everything the loop does is observable through [`AgentEvent`]s on the broadcast
//! channel; frontends are reducers over that stream.

mod exec;
mod gate;
mod registry;

#[doc(hidden)]
pub mod testing;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

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
}

impl Default for LoopLimits {
    fn default() -> Self {
        Self {
            max_parallel_tool_calls: 8,
            doom_loop_threshold: 3,
            steering_mode: QueueMode::OneAtATime,
            followup_mode: QueueMode::All,
            max_turns: 64,
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
        })
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

    /// Current transcript snapshot (all roles, including harness-only messages).
    pub fn history(&self) -> Vec<Message> {
        self.history.lock().map(|h| h.clone()).unwrap_or_default()
    }

    fn push_message(&self, m: &Message) {
        if let Ok(mut h) = self.history.lock() {
            h.push(m.clone());
        }
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

        if let Ok(mut s) = self.doom_streaks.lock() {
            s.clear();
        }
        self.push_message(&prompt);
        self.emit(AgentEvent::AgentStart);

        let outcome = self.run_inner(prompt).await;

        self.emit(AgentEvent::AgentEnd {
            messages: outcome
                .as_ref()
                .map(|o| o.produced.clone())
                .unwrap_or_default(),
        });
        self.running.store(false, Ordering::SeqCst);
        outcome
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
                if turn_index >= self.limits.max_turns {
                    reason = reason.merge_sticky(TurnEndReason::BudgetExhausted);
                    break 'outer;
                }

                // Drain point: inject queued messages before the next model call.
                for m in pending.drain(..) {
                    self.push_message(&m);
                    produced.push(m.clone());
                }

                self.emit(AgentEvent::TurnStart { turn_index });

                // ---- one provider turn --------------------------------------------
                let assistant = match self.stream_one_turn(&mut usage_total).await {
                    Ok(a) => a,
                    Err(e) => return Err(e),
                };
                let m = Message::Assistant(assistant.clone());
                produced.push(m);

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

    async fn stream_one_turn(
        self: &Arc<Self>,
        usage_total: &mut Usage,
    ) -> Result<AssistantMessage, RunError> {
        let history = self.history();
        let ctx = epsilon_llm::LlmContext {
            system_prompt: String::new(), // prompt tier lands in M7; neutral for now
            messages: Self::convert_to_llm(&history),
            tools: self.tools.descriptors(),
        };

        let model = epsilon_llm::ModelRef::new("faux-1");
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
                    if folder.apply(other).is_some()
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
