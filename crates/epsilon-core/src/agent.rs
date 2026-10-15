//! Building an [`Agent`] and everything a front end sets on it afterwards.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::message::Usage;

use tokio::sync::broadcast;

use crate::{Agent, LoopLimits, PendingQueue, PermissionGate, ToolRegistry};

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
        // M53: background-task completion notices (see the `tasks` field docs).
        let (task_notices_tx, task_notices_rx) = tokio::sync::mpsc::unbounded_channel();
        let steering_queue = PendingQueue::new(limits.steering_mode);
        let followup_queue = PendingQueue::new(limits.followup_mode);
        let window = limits.context_window_tokens;
        Arc::new(Self {
            limits,
            stream_fn: RwLock::new(stream_fn),
            tools,
            gate,
            tool_hooks: RwLock::new(Arc::new(epsilon_contracts::tool::NoopToolHooks)),
            turn_hooks: RwLock::new(Arc::new(epsilon_contracts::turn::NoopTurnHooks)),
            history: Mutex::new(Vec::new()),
            observation_scope: Mutex::new(Arc::new(())),
            history_epoch: std::sync::atomic::AtomicU64::new(0),
            session_usage: Mutex::new(Usage::default()),
            session_usage_by_model: Mutex::new(std::collections::BTreeMap::new()),
            steering: Mutex::new(steering_queue),
            followups: Mutex::new(followup_queue),
            events_tx,
            running: AtomicBool::new(false),
            doom_streaks: Mutex::new(HashMap::new()),
            workspace_root,
            session_store: Mutex::new(None),
            durable_write_failure: Mutex::new(None),
            overflow_retried: AtomicBool::new(false),
            // A goal is disarmed for a process that has not read it yet: see the field's docs.
            goal_armed: AtomicBool::new(false),
            compact_suppressed: AtomicBool::new(false),
            agents: Mutex::new(Vec::new()),
            task_depth: Mutex::new(0),
            agent_brief: Mutex::new(String::new()),
            system_overlay: Mutex::new(crate::prompt::SystemOverlay::default()),
            touched_dirs: Mutex::new(Default::default()),
            last_repair: Mutex::new(None),
            tasks: Arc::new(Mutex::new(Vec::new())),
            task_notices_tx: task_notices_tx.clone(),
            task_notices_rx: Mutex::new(task_notices_rx),
            summarizer: Mutex::new(None),
            skills: Mutex::new(Vec::new()),
            model_id: Mutex::new("faux-1".into()),
            thinking_level: Mutex::new(epsilon_contracts::ThinkingLevel::Off),
            abort_armed: std::sync::atomic::AtomicBool::new(false),
            run_token: Mutex::new(None),
            model_window_tokens: Mutex::new(window),
            usage_anchor: Mutex::new(None),
            progress: Mutex::new(crate::guards::ProgressGuard::new()),
            cache: Mutex::new(crate::cache_stats::CacheStats::default()),
        })
    }

    /// Override the model identifier (M10): providers route via the catalog's wire
    /// column and resolve auth from env vars at call time
    /// per family from standard env vars; the id also renders into the VOLATILE tier.
    ///
    /// Also refreshes the live context-window override from the catalog, so the
    /// compaction trigger tracks hot-swaps (`/model`, `try_hot_swap_to_real`).
    /// Unknown ids (custom providers) clear the override — the fixed threshold
    /// keeps working, and no stale window survives a model change.
    ///
    /// **And records the change in the session, so a resume adopts it.**
    ///
    /// This is here rather than in the front ends because a session is a *shared* file and
    /// every front end writes to it: on 2026-10-04 the owner reported that switching models
    /// "keeps reverting to another", and the cause was that `resumed_model_to_use` prefers the
    /// **session's** model over the saved config — while **nothing** recorded a switch, so the
    /// session could only ever report whatever its last assistant message ran on. Fixing that
    /// in the TUI's `apply_model` left two RPC paths (`rpc_mode.rs:660`, `:722`) switching a
    /// live session without recording it, so a stdio-RPC client got the same defect straight
    /// back. **One writer, in the place every writer already calls.**
    ///
    /// A session store is optional — a bare `--tui` launch has none — so the absence of one is
    /// `Ok(())`, not an error. A store that cannot be locked or appended is likewise not this
    /// function's failure: the switch has already been applied and is visible in the status bar,
    /// so refusing would undo a choice the reader can see take effect. The front end decides
    /// whether to say so.
    pub fn set_model_id(&self, model_id: impl Into<String>) {
        let model_id = model_id.into();
        let window =
            epsilon_llm::catalog::lookup_model(&model_id).map(|m| m.context_window as usize);
        if let Ok(mut slot) = self.model_id.lock() {
            *slot = model_id.clone();
        }
        if let Ok(mut slot) = self.model_window_tokens.lock() {
            *slot = window;
        }
        self.record_model_change(&model_id);
    }

    /// Switch the gate's permission preset mid-session (ADR-032).
    ///
    /// Returns what the gate reported: `false` means no preset changed (an
    /// unknown name, or a fixed-posture gate with nothing to switch). The
    /// workspace scopes the workspace-write allows; parked asks keep the
    /// policy that raised them (fail closed beats seamless).
    pub fn set_preset(&self, preset: &str, workspace: &std::path::Path) -> bool {
        self.gate.set_preset(preset, workspace)
    }

    /// The preset name last applied (what bare `/preset` reports).
    pub fn current_preset(&self) -> String {
        self.gate.current_preset()
    }

    /// Append a `model_change` entry to the attached session, if there is one.
    ///
    /// Called by [`Agent::set_model_id`]; separate only so the test that drives a front end's
    /// switch does not have to reach through a lock. The provider comes from the catalog
    /// because `context_settings` keeps the pair verbatim, and a model the catalog does not know
    /// has no provider to record — pi writes the bare id for those.
    ///
    /// **A failure here is reported, not discarded (M311).** This line used to be
    /// `let _ = guard.append(..)`, and M303's own doc said *"the front end decides whether to say
    /// so"* — which no front end could do, because nothing told one. The switch has already been
    /// applied and is visible in the status bar, so the fix is not to fail the switch: it is to
    /// say what did not happen, which is the same shape as [`Self::persist`] two hundred lines
    /// below and for the same reason — *an error nobody prints is not a report*. Every front end
    /// already renders [`AgentEvent::Notice`] (`subscribe`), so this needs no new channel.
    fn record_model_change(&self, model_id: &str) {
        let Some(store) = self.store() else { return };
        let provider = epsilon_llm::catalog::lookup_model(model_id)
            .map(|m| m.provider.to_string())
            .unwrap_or_default();
        let outcome: Result<(), String> = match store.lock() {
            Ok(mut guard) => guard
                .append(epsilon_contracts::session_entry::EntryKind::ModelChange {
                    provider,
                    model_id: model_id.to_string(),
                })
                .map(|_| ())
                .map_err(|e| e.to_string()),
            // A poisoned lock is the same failure seen from the other side: the session's memory
            // of this switch did not get written.
            Err(_) => Err("the session store's lock is poisoned".to_string()),
        };
        if let Err(e) = outcome {
            self.emit(AgentEvent::Notice {
                text: format!(
                    "model switched, but this session could not record it ({e}) — resuming this \
                     session may come back on its previous model"
                ),
            });
        }
    }

    /// The model identifier the next request will carry (frontends render it; the RPC
    /// and HTTP `get_state` surfaces report it). Set through [`Agent::set_model_id`].
    pub fn model_id(&self) -> String {
        self.model_id.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Hot-swap the provider stream without a restart (M32d).
    ///
    /// Wired from `crates/epsilon-tui` after `epsilon auth login` saves a key
    /// so the *next* prompt hits the real API like pi/opencode. Real-only.
    pub fn try_hot_swap_to_real(&self) -> bool {
        #[cfg(feature = "real")]
        {
            let cur = self.model_id.lock().map(|s| s.clone()).unwrap_or_default();
            if !cur.starts_with("faux") && epsilon_llm::catalog::credentials_ready(&cur) {
                return false;
            }
            let real = self.retrying_real_stream();
            if let Ok(mut slot) = self.stream_fn.write() {
                *slot = real;
                return true;
            }
            false
        }
        #[cfg(not(feature = "real"))]
        {
            false
        }
    }

    /// Set the thinking/reasoning effort for subsequent runs (pi's per-session
    /// `thinkingLevel`; provider wires map it to native parameters).
    ///
    /// **And records the change in the session, so a resume comes back on it** —
    /// the one-writer pattern [`Self::set_model_id`] uses, for the same reason:
    /// a session file is shared by every front end, so the write belongs where
    /// all of them already call. The reader (`SessionStore::context_settings`)
    /// has existed since the port; until now nothing in production fed it, so a
    /// level set in one run was re-set by hand in the next, every launch.
    ///
    /// A store is optional — a bare `--tui` launch has none, and a subagent
    /// never attaches one — so its absence is `Ok(())`, not an error, exactly as
    /// in [`Self::set_model_id`]. A store that cannot be locked or appended is
    /// reported rather than discarded (M311): the level is already applied and
    /// visible in the status bar, so refusing would undo a choice the reader can
    /// see take effect, while saying nothing would leave the next resume
    /// silently on the previous level.
    pub fn set_thinking_level(&self, level: epsilon_contracts::ThinkingLevel) {
        if let Ok(mut slot) = self.thinking_level.lock() {
            *slot = level;
        }
        self.record_thinking_level_change(level);
    }

    /// Current thinking level (session state, like the model id).
    pub fn thinking_level(&self) -> epsilon_contracts::ThinkingLevel {
        self.thinking_level.lock().map(|l| *l).unwrap_or_default()
    }

    /// Append a `thinking_level_change` entry to the attached session, if there is one.
    ///
    /// Called by [`Agent::set_thinking_level`]; separate only so a front end's
    /// switch test can drive the record without reaching through a lock, which
    /// is the whole reason [`Self::record_model_change`] is separate too.
    fn record_thinking_level_change(&self, level: epsilon_contracts::ThinkingLevel) {
        let Some(store) = self.store() else { return };
        let outcome: Result<(), String> = match store.lock() {
            Ok(mut guard) => guard
                .append(
                    epsilon_contracts::session_entry::EntryKind::ThinkingLevelChange {
                        level: level.as_str().to_string(),
                    },
                )
                .map(|_| ())
                .map_err(|e| e.to_string()),
            // A poisoned lock is the same failure seen from the other side: the
            // session's memory of this level did not get written.
            Err(_) => Err("the session store's lock is poisoned".to_string()),
        };
        if let Err(e) = outcome {
            self.emit(AgentEvent::Notice {
                text: format!(
                    "thinking level set, but this session could not record it ({e}) — resuming \
                     this session may come back on its previous level"
                ),
            });
        }
    }

    /// Attach discovered skills for the CONTEXT-tier catalog (M11 wiring).
    pub fn set_skills(&self, skills: Vec<crate::skills::Skill>) {
        if let Ok(mut slot) = self.skills.lock() {
            *slot = skills;
        }
    }

    /// Attach discovered agent-type defs for `task` routing (M50 wiring).
    pub fn set_agents(&self, agents: Vec<crate::agents::AgentDef>) {
        if let Ok(mut slot) = self.agents.lock() {
            *slot = agents;
        }
    }

    /// Role brief prepended to the CONTEXT tier (M50: the agent-def body for
    /// `task` children — pi's agent file body past frontmatter). Empty for
    /// top-level sessions; set once on the child before its first run.
    pub fn set_agent_brief(&self, brief: String) {
        if let Ok(mut slot) = self.agent_brief.lock() {
            *slot = brief;
        }
    }

    /// Attach the workspace's identity-tier overlays (`.epsilon/SYSTEM.md` and
    /// `.epsilon/APPEND_SYSTEM.md`, or the global pair).
    ///
    /// Discovery stays with the frontend because it owns the trust decision, exactly as it
    /// does for skills: an untrusted checkout must not be able to replace the harness's own
    /// contract. Setting this once at composition time is what makes a written `SYSTEM.md`
    /// something other than a file the user was asked to trust for no reason.
    pub fn set_system_overlay(&self, overlay: crate::prompt::SystemOverlay) {
        if let Ok(mut slot) = self.system_overlay.lock() {
            *slot = overlay;
        }
    }

    pub(crate) fn system_overlay(&self) -> crate::prompt::SystemOverlay {
        self.system_overlay
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub(crate) fn agent_list(&self) -> Vec<crate::agents::AgentDef> {
        self.agents.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub(crate) fn task_depth_slot(&self) -> Result<std::sync::MutexGuard<'_, usize>, String> {
        self.task_depth
            .lock()
            .map_err(|_| "task depth poisoned".to_string())
    }

    /// The discovered skills with their descriptions and paths.
    ///
    /// A count badge only needs the skill names, but pi's `get_commands` returns
    /// `RpcSlashCommand` carrying a description and a `sourceInfo` path
    /// (`rpc-types.ts:130-141`), so a client building a command palette needs the whole
    /// row. Public because that is now a real caller rather than a future one.
    pub fn skills(&self) -> Vec<crate::skills::Skill> {
        self.skill_list()
    }

    pub(crate) fn skill_list(&self) -> Vec<crate::skills::Skill> {
        self.skills.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Provide a dedicated auxiliary provider for summarization calls.
    pub fn set_summarizer(&self, s: Arc<dyn epsilon_llm::StreamFn>) {
        if let Ok(mut slot) = self.summarizer.lock() {
            *slot = Some(s);
        }
    }

    pub(crate) fn summarizer_stream(&self) -> Arc<dyn epsilon_llm::StreamFn> {
        self.summarizer
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .unwrap_or_else(|| {
                self.stream_fn
                    .read()
                    .map(|g| Arc::clone(&*g))
                    .unwrap_or_else(|e| Arc::clone(&*e.into_inner()))
            })
    }

    /// Subscribe to the lifecycle event stream (TUI, server SSE, telemetry all consume this).
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<AgentEvent>> {
        self.events_tx.subscribe()
    }

    /// Register interception hooks around tool execution (extension hosts,
    /// compaction summarizers, tests). Replaces the previous hooks.
    pub fn set_tool_hooks(&self, hooks: Arc<dyn epsilon_contracts::tool::ToolHooks>) {
        if let Ok(mut slot) = self.tool_hooks.write() {
            *slot = hooks;
        }
    }

    /// Current hooks (cloned `Arc`, cheap). The execution pipeline calls this
    /// per tool call so a host can swap hooks mid-run.
    pub(crate) fn tool_hooks(&self) -> Arc<dyn epsilon_contracts::tool::ToolHooks> {
        self.tool_hooks
            .read()
            .map(|h| Arc::clone(&*h))
            .unwrap_or_else(|_| Arc::new(epsilon_contracts::tool::NoopToolHooks))
    }

    /// Register turn-boundary hooks (pi's `AgentLoopConfig`: `transformContext`,
    /// `convertToLlm`, `prepareNextTurn`, `shouldStopAfterTurn`). Replaces the
    /// previous hooks. Unregistered seams stay at their no-op defaults, so the loop
    /// behaves exactly as it did before this hook point existed.
    pub fn set_turn_hooks(&self, hooks: Arc<dyn epsilon_contracts::turn::TurnHooks>) {
        if let Ok(mut slot) = self.turn_hooks.write() {
            *slot = hooks;
        }
    }

    /// Current turn hooks (cloned `Arc`, cheap). The loop reads this once per
    /// turn boundary and once per request build, so a host can swap hooks mid-run.
    pub(crate) fn turn_hooks(&self) -> Arc<dyn epsilon_contracts::turn::TurnHooks> {
        self.turn_hooks
            .read()
            .map(|h| Arc::clone(&*h))
            .unwrap_or_else(|_| Arc::new(epsilon_contracts::turn::NoopTurnHooks))
    }

    /// The workspace this agent was built for: where relative paths resolve, and what the
    /// `@`-reference expander confines to (M74). Frontends need it to build user messages
    /// through the shared constructor rather than each rolling their own.
    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    pub(crate) fn emit(&self, ev: AgentEvent) {
        let _ = self.events_tx.send(Arc::new(ev));
    }
}
