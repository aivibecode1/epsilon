//! The Epsilon agent runtime.
//!
//! Ports pi's two-level loop (steering inner / follow-up outer), adds three named
//! improvements at defined seams: a bounded parallel tool pool, fail-all-on-truncation
//! (from pi itself), and the doom-loop brake (opencode + DSH).
//!
//! Everything the loop does is observable through [`AgentEvent`]s on the broadcast
//! channel; frontends are reducers over that stream.

pub mod agents;
// M196: public because the TUI formats token counts in the compaction row too,
// and a private module would mean a SECOND formatter. One number, one
// rendering -- or a header reading `12k` beside a row reading `12000`, and a
// reader left to guess whether those are the same figure.
pub mod cache_stats;
mod compact;
pub mod config;
pub mod exec;
mod gate;
mod guards;
pub mod ignore;
pub mod paths;
pub mod prompt;
pub mod references;
mod registry;
pub mod skills;
#[doc(hidden)]
pub mod test_support;
mod untrusted;
mod verify;
/// Walking a workspace to list content rather than build output.
///
/// Lives beside [`ignore`] because the two are one answer. `epsilon_tui::at_files`
/// predicted this move in its own module doc: *"If a third consumer of this walk ever
/// appears, the walk belongs in core"* — and the desktop file tree was the third.
pub mod walk;

#[doc(hidden)]
pub mod testing;

// The loop's own modules. What moved here used to live at the crate root, and every
// path below still resolves there: the re-exports keep `epsilon_core::Agent`,
// `epsilon_core::RunError` and friends exactly where every caller found them, so the
// split added no edit anywhere else in the workspace.
mod agent;
mod goal;
mod queue;
mod run_;
mod session;
mod tasks;
mod util;

pub(crate) use queue::PendingQueue;
pub(crate) use tasks::TaskEntry;
#[cfg(feature = "real")]
pub use util::REAL_PATH_ENABLED;
pub use util::{
    CompactionOutcome, LoopLimits, RunError, RunOutcome, human_age, human_age_ms,
    looks_like_auth_failure, retry_notice_text,
};
pub(crate) use util::{
    RunGuard, clamp_tail_tokens, contain_hook, contain_panic, estimate_one,
    looks_like_context_overflow, now_ms, store_lock_failure, today_string,
};

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::broadcast;

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::message::{Message, Usage};

pub use gate::{Authorization, AutoAllow, AutoDeny, PermissionGate};
pub use registry::ToolRegistry;
// Session-log surface for frontends (pi search/scanning.ts parity). Re-exported here
// so epsilon-tui/server never need a direct epsilon-session dependency edge.
pub use epsilon_session::{
    SearchOptions, SessionSearchHit, SessionStore, StoreError, search_session,
};

/// How many queued messages a drain point releases.
///
// `QueueMode` now lives in `epsilon-contracts` -- see that file for why the architecture
// law pushed it there -- and is re-exported here, so `epsilon_core::QueueMode` resolves
// exactly as it did before and no caller had to change.
pub use epsilon_contracts::QueueMode;

#[cfg(test)]
mod auth_classifier_tests {
    use super::looks_like_auth_failure;

    #[test]
    fn the_reported_failure_is_recognised() {
        // The body verbatim off the reporter's screen, 2026-10-04.
        assert!(looks_like_auth_failure(
            "HTTP 401 (Auth): {\"error\":{\"message\":\"Authentication Fails, Your api key: \
             ****6a81 is invalid\",\"type\":\"authentication_error\",\
             \"code\":\"invalid_request_error\"}}"
        ));
    }

    #[test]
    fn the_other_spellings_are_recognised_too() {
        for m in [
            "401 Unauthorized",
            "Authentication failed",
            "invalid api key",
            "invalid_api_key",
            "authentication_error",
        ] {
            assert!(looks_like_auth_failure(m), "not recognised: {m}");
        }
    }

    #[test]
    fn every_auth_failure_the_retry_policy_recognises_is_reported_too() {
        // The two classifiers answer different questions about the same failure, and the
        // pairing is not decoration. `epsilon_llm::classify` decides a failure is never
        // retried; `looks_like_auth_failure` decides a reader is told the model was never
        // reached. When they disagree, the retry policy correctly gives up while the front
        // end says nothing — which is the 2026-10-04 report's own shape reproduced with a
        // different status code.
        //
        // 403 is the common one: a key that is valid but has no access to the model it
        // named. A user whose plan lapsed or whose tier excludes that model hits it, and
        // before this nothing on screen said the model was never reached — they were told
        // a provider error happened and left to work out whether anything was generated.
        for m in [
            "HTTP 403 (Auth): forbidden",
            "HTTP 403: {\"error\":{\"message\":\"You are not authorized to access model X\"}}",
            "403 Forbidden",
            "forbidden",
        ] {
            assert_eq!(
                epsilon_llm::classify(m),
                epsilon_llm::FailureKind::Auth,
                "precondition: the retry policy calls this Auth — {m}"
            );
            assert!(
                looks_like_auth_failure(m),
                "the front ends must say the model was never reached too — {m}"
            );
        }
    }

    #[test]
    fn unrelated_failures_are_not_mistaken_for_auth() {
        // A false positive here would claim "your model was never reached" about a run that
        // did reach it, which is worse than saying nothing: the reader would go re-check a key
        // that is fine.
        //
        // Every case is a **provider error body**, because that is the only thing this is ever
        // handed (`err`, the message about a call that produced no reply). A tighter rule than
        // `contains` was tried for the status-code needles and rejected: any delimiter-based
        // version of it fires on `{"input": 403}` exactly as often as the loose one does, so
        // the "usage figure is not a status" guard cannot be written without a context this
        // function does not have. The exposure is recorded in the doc comment instead.
        for m in [
            "429 rate limit exceeded, retry after 20s",
            "500 internal server error",
            "connection reset by peer",
            "context window exceeded",
            "the model returned an empty response",
            "request failed: model returned no text",
        ] {
            assert!(!looks_like_auth_failure(m), "wrongly classified: {m}");
        }
    }
}

#[cfg(test)]
mod cwd_note_tests {
    use super::Agent;

    /// Empty recording stays silent; the same directory stays silent (even
    /// spelled with a trailing separator); a different live directory names
    /// both; a gone directory says so. RED first: no such helper.
    #[test]
    fn recorded_vs_running_names_mismatches_only() {
        let tmp = tempfile::tempdir().expect("tmp");
        let here = tmp.path();
        let elsewhere = tempfile::tempdir().expect("elsewhere");
        let gone = here.join("gone-zzz");

        assert!(Agent::cwd_mismatch_note("", here).is_none());
        assert!(Agent::cwd_mismatch_note("   ", here).is_none());
        assert!(Agent::cwd_mismatch_note(&here.to_string_lossy(), here).is_none());

        let trailing = format!("{}{}", here.display(), std::path::MAIN_SEPARATOR);
        assert!(
            Agent::cwd_mismatch_note(&trailing, here).is_none(),
            "a trailing separator is not a mismatch"
        );

        let note = Agent::cwd_mismatch_note(&elsewhere.path().to_string_lossy(), here)
            .expect("a different live directory must be named");
        assert!(
            note.contains(&elsewhere.path().to_string_lossy().into_owned())
                && note.contains(&here.to_string_lossy().into_owned()),
            "names both directories: {note:?}"
        );

        let gone_note = Agent::cwd_mismatch_note(&gone.to_string_lossy(), here)
            .expect("a gone directory must be named");
        assert!(
            gone_note.contains("no longer exists"),
            "says the directory is gone: {gone_note:?}"
        );
    }
}

#[cfg(test)]
mod overflow_classifier_tests {
    use super::looks_like_context_overflow;

    #[test]
    fn every_spelling_the_providers_use_is_recognised() {
        // Each of these is a real provider body shape. The underscore forms are
        // the ones that were missing, and their absence meant a genuine overflow
        // failed the run instead of compacting once and retrying.
        for message in [
            "request_too_large",
            "Error: request_too_large: prompt exceeds the model's maximum",
            "prompt is too long: 210000 tokens > 200000 maximum",
            "prompt_too_long",
            "This model's maximum context length is 128000 tokens",
            "context_length_exceeded",
            "context_window_exceeded",
            "Input is too long for requested model",
            "Request too large for this model",
            "too many tokens in request",
            "please reduce the length of the messages",
        ] {
            assert!(
                looks_like_context_overflow(message),
                "not recognised as an overflow: {message:?}"
            );
        }
    }

    #[test]
    fn unrelated_failures_are_not_mistaken_for_overflow() {
        // A false positive costs a pointless compaction that throws away context,
        // so the classifier has to be specific as well as complete.
        for message in [
            "401 Unauthorized: invalid api key",
            "rate limit exceeded, retry after 20s",
            "connection reset by peer",
            "tool call arguments were truncated",
            "the model returned an empty response",
        ] {
            assert!(
                !looks_like_context_overflow(message),
                "wrongly classified as an overflow: {message:?}"
            );
        }
    }

    #[test]
    fn classification_ignores_case() {
        assert!(looks_like_context_overflow("REQUEST_TOO_LARGE"));
        assert!(looks_like_context_overflow("Prompt Is Too Long"));
    }
}

/// The agent: owns the transcript, the queues, and the event bus.
pub struct Agent {
    limits: LoopLimits,
    stream_fn: RwLock<Arc<dyn epsilon_llm::StreamFn>>,
    tools: Arc<ToolRegistry>,
    gate: Arc<dyn PermissionGate>,
    /// Interception hooks around tool execution (pi's `beforeToolCall` /
    /// `afterToolCall`). Swappable after construction so extension hosts can
    /// register without rebuilding the agent.
    tool_hooks: RwLock<Arc<dyn epsilon_contracts::tool::ToolHooks>>,
    /// Turn-boundary hooks (pi's `transformContext` / `convertToLlm` /
    /// `prepareNextTurn` / `shouldStopAfterTurn`). Swappable after construction
    /// so extension hosts can register without rebuilding the agent.
    turn_hooks: RwLock<Arc<dyn epsilon_contracts::turn::TurnHooks>>,

    history: Mutex<Vec<Message>>,
    observation_scope: Mutex<Arc<()>>,
    /// Bumped whenever runtime history is replaced wholesale: a committed compaction
    /// (mid-run pressure or overflow recovery) or a restored transcript. A run-scoped
    /// `prepare_next_turn` replacement records the epoch it was installed at; when the
    /// two differ its base no longer exists, so the replacement is dropped and the next
    /// request rebuilds from the current source (CORE-02 — a stale replacement would
    /// stop reading the store's shadowed view and re-send a request a compaction just
    /// invalidated).
    history_epoch: std::sync::atomic::AtomicU64,
    steering: Mutex<PendingQueue>,
    followups: Mutex<PendingQueue>,
    /// Provider usage summed over every run this agent has served, since construction.
    ///
    /// **The run loop's own `usage_total` is deliberately run-scoped** (CORE-02 keeps a
    /// run's turn context, model and transcript out of the session), and this is the
    /// session-level total the run loop cannot provide. It exists because the only
    /// cumulative accounting in the workspace lived in the TUI's `UiState` — a *view*.
    ///
    /// That was the actual reason pi's `get_session_stats` could not be answered: not
    /// missing arithmetic, but a number that only existed somewhere a view happened to
    /// be. `RpcMode` has no `UiState`, so it could not read it even in principle.
    ///
    /// Folded in once per completed run, from the run's own total, so the two cannot
    /// drift. A lock rather than an atomic because `Usage` is a five-field struct and a
    /// lock is not measurably slower at one write per run.
    session_usage: Mutex<Usage>,
    /// The same usage, split by the model that actually produced it.
    ///
    /// **Why a map and not just the flat total.** Price is a property of the model that
    /// consumed the tokens, so a session that switched models cannot be priced from a
    /// single row — summing the flat total and pricing it at the *current* model
    /// under-bills every response the previous model produced. This is the same reason
    /// `UiState` has `usage_by_model`, arrived at independently.
    session_usage_by_model: Mutex<std::collections::BTreeMap<String, Usage>>,

    events_tx: broadcast::Sender<Arc<AgentEvent>>,
    running: AtomicBool,
    /// Run-scoped doom-loop counters; cleared when a run starts (see exec.rs).
    doom_streaks: Mutex<HashMap<String, usize>>,
    /// Declared workspace root; path arguments resolving outside it get gated under the
    /// external_dir capability instead of their tool name.
    pub(crate) workspace_root: std::path::PathBuf,
    /// Durable mirror of this agent's transcript; compaction entries land here and the
    /// provider context switches to the store's shadowed model view when attached.
    session_store: Mutex<Option<Arc<Mutex<SessionStore>>>>,
    /// The durable write that failed on this attached store, if one did (F07).
    ///
    /// Set by [`Agent::durable_write_failed`] and read by [`Agent::try_context_view`], which
    /// refuses to serve the store's projection afterwards: a projection missing a message the
    /// loop believes it recorded is exactly the divergence F07 is about, and the alternative —
    /// quietly carrying on, or quietly falling back to runtime history — is the "silently
    /// alternate authoritative sources" the ledger forbids. Cleared when a store is attached:
    /// the request path's source is a different file, and this latch is a statement about one.
    durable_write_failure: Mutex<Option<String>>,
    /// Set when an overflow-triggered compaction already retried this run.
    overflow_retried: AtomicBool,
    /// Whether this process may continue the session goal on its own.
    ///
    /// **Never persisted, and false when the process starts.** A goal survives a restart; the
    /// permission to keep working on it unattended does not, because the person who armed it is
    /// not necessarily the person who opens the session next. DSH's rule, and the reason
    /// [`GoalView`] carries activation separately from the durable snapshot.
    goal_armed: AtomicBool,
    /// Set when a pressure-triggered compaction pass failed this run (provider error,
    /// empty answer, append failure). One attempt per run: a summarizer that just
    /// failed will not succeed on the next turn, and each retry is a paid call.
    compact_suppressed: AtomicBool,
    /// Consecutive pressure-triggered compactions that committed yet reclaimed less than
    /// 10% of the estimated context (ADR-048 anti-thrash guard). Session-scoped, never
    /// run-scoped: a per-run reset is the thrash this exists to stop. Updated only at the
    /// pressure call site (`run_.rs`); read by the pressure gate; cleared by a reducing
    /// commit, a manual `/compact` commit, or a session resume.
    nonreducing_streak: std::sync::atomic::AtomicUsize,
    /// Discovered agent-type defs for `task` routing (M50: pi `agents.ts` parity).
    /// Interior-mutable so composition code can attach discoveries after `Arc<Agent>`.
    agents: Mutex<Vec<crate::agents::AgentDef>>,
    /// Delegation depth of this agent: 0 for the top-level session, +1 per
    /// `task` hop. Children at the cap cannot delegate further (opencode
    /// `subagent_depth` parity — unbounded nesting would recurse the loop).
    /// Interior-mutable so `run_task` can stamp the child after construction.
    task_depth: Mutex<usize>,
    /// Role brief for delegated children (M50: agent-def system prompt).
    /// Rendered FIRST in the CONTEXT tier so role survives cache discipline.
    agent_brief: Mutex<String>,
    /// Workspace-supplied replacement/extension of the STABLE tier's identity block
    /// (`crate::prompt::SystemOverlay`; pi `SYSTEM.md` + `APPEND_SYSTEM.md`).
    /// Frontend-attached, because the frontend owns the trust decision that guards it.
    system_overlay: Mutex<crate::prompt::SystemOverlay>,
    /// What the last session resume had to repair (M75), so a frontend can tell the user what
    /// their session was missing. `None` when the session opened clean or was never attached.
    last_repair: Mutex<Option<epsilon_session::repair::RepairReport>>,
    /// Directories a tool call has touched this process (M73), for subdirectory hints.
    ///
    /// A set, not a list: the hint it produces is the same whichever call discovered the
    /// directory, and the prompt must not change shape because a call was retried.
    touched_dirs: Mutex<std::collections::HashSet<std::path::PathBuf>>,
    /// M53: delegated tasks this session tracks, in dispatch order. Children
    /// stay alive between calls, so `task_id` can resume one and the tool
    /// layer can report status. Bounded by [`TASK_REGISTRY_CAP`].
    tasks: Arc<Mutex<Vec<TaskEntry>>>,
    /// M53: completion notices from background children. Senders live in the
    /// detached run tasks; the receiving side drains into the steering queue
    /// at every drain point (see the private `drain_steering`).
    task_notices_tx: tokio::sync::mpsc::UnboundedSender<Message>,
    task_notices_rx: Mutex<tokio::sync::mpsc::UnboundedReceiver<Message>>,
    /// Discovered skills (catalog in CONTEXT tier; bodies load on demand via skill tool).
    /// Interior-mutable so composition code can attach discoveries after `Arc<Agent>`.
    skills: Mutex<Vec<crate::skills::Skill>>,
    /// Dedicated auxiliary stream for summarization (isolated from the main loop's
    /// provider/script; pi isolates aux calls for cache reasons, we ALSO isolate scripts).
    summarizer: Mutex<Option<Arc<dyn epsilon_llm::StreamFn>>>,
    /// Model identifier sent to the provider and rendered in the VOLATILE prompt tier
    /// ("faux-1" until overridden — real providers route on this).
    model_id: Mutex<String>,
    /// Session thinking level (pi's session.thinkingLevel); rides every run.
    thinking_level: Mutex<epsilon_contracts::ThinkingLevel>,
    /// Set when an abort arrives while no run is in flight (M54): a run that
    /// starts *after* the stop request (a child between spawn and first poll)
    /// must still honour it, so the flag is consumed at the next run's start.
    abort_armed: std::sync::atomic::AtomicBool,
    /// Token for the ACTIVE run; cancelled by [`Agent::abort_current_run`]. Tools
    /// receive clones through their `ToolContext` so long-running work bails
    /// cooperatively (M12: closes the never-cancelled-token gap, ADR-010).
    run_token: Mutex<Option<tokio_util::sync::CancellationToken>>,
    /// Live context-window override, refreshed by [`Agent::set_model_id`] from the
    /// catalog (hot-swaps included). Seeded from `limits.context_window_tokens`
    /// at construction; the compaction trigger reads this, never `limits`.
    model_window_tokens: Mutex<Option<usize>>,
    /// What the provider billed for the last completed request, plus how much of
    /// the history that request covered. Makes the compaction trigger a measurement
    /// instead of a guess (pi `compaction.ts`); cleared whenever the history it
    /// describes is rewritten.
    usage_anchor: Mutex<Option<crate::compact::UsageAnchor>>,
    /// Consecutive-identical-result tracker for the stuck-run guard (`crate::guards`).
    /// Run-scoped like `doom_streaks`: a fresh run starts with a clean slate.
    progress: Mutex<crate::guards::ProgressGuard>,
    /// Cache-miss attribution across this agent's calls (`crate::cache_stats`). Session-scoped:
    /// the prompt cache outlives a run, so the evidence for or against it does too.
    cache: Mutex<crate::cache_stats::CacheStats>,
}
