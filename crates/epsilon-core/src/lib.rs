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

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::broadcast;

use epsilon_contracts::content::UserContent;
use epsilon_contracts::goal::{
    DEFAULT_MAX_GOAL_ROUNDS, GoalChange, GoalError, GoalOp, GoalRef, GoalSnapshot, GoalStatus,
    GoalView,
};
use epsilon_contracts::session_entry::EntryKind;

/// Compile-time marker: this build carries the real-provider path (M52).
///
/// `epsilon-cli` binds it under its own `real` feature, so a build that turns
/// on the network layer without turning on this crate's real code (which
/// would leave `Agent::try_hot_swap_to_real` stubbed while the CLI advertises
/// real providers) fails to compile instead of degrading silently.
#[cfg(feature = "real")]
pub const REAL_PATH_ENABLED: () = ();

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

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A duration as something a person reads at a glance: `42s`, `7m`, `3h`, `12d`.
///
/// Two units at each scale, not three: a reader comparing two sessions wants to know
/// "just now" or "a while ago", and a finer scale is noise between them.
///
/// It lived in `epsilon-cli`'s doctor and came here when the desktop's sessions pane
/// needed the same answer — which is the same situation as the workspace walk at M236,
/// one layer down: one question, two consumers, and a second copy that would drift.
pub fn human_age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// The same answer for an epoch-millisecond timestamp, or `None` when it carries nothing.
///
/// `None` rather than `"0s"`: a listing whose timestamps are all zero is a filesystem that
/// said nothing, and printing `0s` against every row would be a measurement.
pub fn human_age_ms(now: u64, then_ms: u64) -> Option<String> {
    if then_ms == 0 {
        return None;
    }
    // A timestamp in the future is a clock change or a file written by a machine whose
    // clock disagrees; saturating keeps it at "0s" rather than wrapping into "18250d".
    Some(human_age(now.saturating_sub(then_ms) / 1000))
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

    /// Restore drained messages to the front (abnormal-exit requeue, plan 002).
    fn restore(&mut self, msgs: Vec<Message>) {
        for m in msgs.into_iter().rev() {
            self.inner.push_front(m);
        }
    }

    fn drain(&mut self) -> Vec<Message> {
        match self.mode {
            QueueMode::All => self.inner.drain(..).collect(),
            QueueMode::OneAtATime => self.inner.pop_front().into_iter().collect(),
        }
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    /// Drain all queued messages regardless of mode (used by abort restore).
    fn drain_all(&mut self) -> Vec<Message> {
        self.inner.drain(..).collect()
    }

    fn set_mode(&mut self, mode: QueueMode) {
        self.mode = mode;
    }

    fn mode(&self) -> QueueMode {
        self.mode
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
    /// Used only when the active model's context window is unknown; a known
    /// window takes over as `window - reserve` (pi `shouldCompact`).
    pub compaction_threshold_tokens: usize,
    /// Recent-message budget protected from pruning/clamping during compaction.
    pub compaction_tail_tokens: usize,
    /// Known context window of the active model in tokens. `None` (custom
    /// providers, faux/test models) keeps the fixed-threshold behavior above.
    /// Prefer [`LoopLimits::for_model`] over setting this by hand.
    pub context_window_tokens: Option<usize>,
    /// Tokens held back from a known window before compacting (pi `reserveTokens`).
    pub compaction_reserve_tokens: usize,
    /// Master switch for automatic compaction (pi's `enabled` gate). `compact_now`
    /// and `/compact` still work when this is false.
    pub compaction_enabled: bool,
    /// Consecutive identical tool RESULTS before the stuck-run guard speaks up
    /// (`guards::ProgressGuard`). The doom-loop brake counts identical *calls*; four different
    /// commands that fail the same way are invisible to it. `0` disables the guard.
    pub no_progress_threshold: usize,
    /// Empty or degenerate final turns tolerated before the run is failed (Hermes
    /// `empty_response_guard`). Each tolerated turn earns one nudge message.
    pub empty_turn_retries: usize,
    /// Stop-time nudges allowed when a run ends with edits no successful check has covered
    /// (`crate::verify::VerificationLog`). `0` disables the nudge; the log still records.
    pub verification_nudges: usize,
}

impl Default for LoopLimits {
    fn default() -> Self {
        Self {
            max_parallel_tool_calls: 8,
            doom_loop_threshold: 3,
            steering_mode: QueueMode::OneAtATime,
            // pi agent.ts:231-232 constructs both queues `one-at-a-time`; a queued
            // follow-up waits for the stop boundary created by the previous one so
            // its request sees that reply (pi parity). Explicit `All` batches them.
            followup_mode: QueueMode::OneAtATime,
            max_turns: 64,
            compaction_threshold_tokens: 120_000,
            compaction_tail_tokens: 12_000,
            context_window_tokens: None,
            compaction_reserve_tokens: 16_384,
            compaction_enabled: true,
            no_progress_threshold: 3,
            empty_turn_retries: 1,
            verification_nudges: 2,
        }
    }
}

impl LoopLimits {
    /// Limits with the active model's context window resolved from the bundled
    /// catalog, so the compaction trigger goes window-relative (pi
    /// `shouldCompact`). Unknown ids (custom providers, faux/test models) keep
    /// `context_window_tokens: None` — the fixed threshold keeps working.
    pub fn for_model(model_id: &str) -> Self {
        let mut limits = Self::default();
        if let Some(info) = epsilon_llm::catalog::lookup_model(model_id) {
            limits.context_window_tokens = Some(info.context_window as usize);
        }
        limits
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

/// What one compaction pass actually did.
///
/// A pass replaces runtime history and appends the durable entry only when the
/// summarization call produced a usable summary. The failure shapes stay
/// distinguishable so the loop can report them honestly and stop retrying
/// instead of treating "attempted" as "compacted".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionOutcome {
    /// Context replaced; the durable entry (when a store is attached) was appended.
    Committed,
    /// Summarization produced no usable result; nothing was committed.
    Failed(String),
    /// Cancellation won while summarizing; nothing was committed.
    Cancelled,
}

/// Heuristic overflow classifier (DSH's CONTEXT_WINDOW_EXCEEDED, message-shaped).
/// One-line description of a scheduled retry (M60) — the text a user reads when
/// the harness pauses instead of failing. Pure, so the notice text is covered by
/// tests without sleeping through a real backoff.
pub fn retry_notice_text(n: &epsilon_llm::RetryNotice) -> String {
    let kind = match n.kind {
        epsilon_llm::FailureKind::RateLimit => " (rate limited)",
        epsilon_llm::FailureKind::Server => " (server error)",
        epsilon_llm::FailureKind::Network => " (network)",
        _ => "",
    };
    format!(
        "{} — retrying {} in {:.1}s (attempt {}){}",
        n.detail,
        n.model,
        n.delay.as_secs_f64(),
        n.attempt + 2,
        kind
    )
}

/// Does this provider error mean **the request was rejected before the model saw it**?
///
/// # Why this classifier exists
///
/// On 2026-10-04 the owner reported "the output of the model is not shown". The renderer was
/// fine: the stored key was rejected with **HTTP 401**, nothing was generated, and the window
/// showed a provider error — but nothing anywhere said *which* model had been configured, or
/// that it had never been reached. `Run complete.` in that session was the **offline demo
/// script's** own line (`epsilon-runtime`'s `default_steps`), so the answer on screen came from
/// a model the user never chose.
///
/// The question this answers is therefore not "is it fatal" but "did anything generate a
/// reply", because that is what decides whether the transcript needs an answer at all. It is
/// deliberately narrow: a rate limit and a truncated tool call are failures too, and claiming
/// those never reached the model would be a lie a reader could not act on.
///
/// Public because the TUI needs it to word its own message: the classifier is a fact about
/// provider errors, and a front end that had to re-derive it would get a second answer.
pub fn looks_like_auth_failure(msg: &str) -> bool {
    const NEEDLES: [&str; 6] = [
        "401",
        "unauthorized",
        "authentication",
        "invalid api key",
        "invalid_api_key",
        "authentication_error",
    ];
    let m = msg.to_ascii_lowercase();
    NEEDLES.iter().any(|needle| m.contains(needle))
}

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
    fn unrelated_failures_are_not_mistaken_for_auth() {
        // A false positive here would claim "your model was never reached" about a run that
        // did reach it, which is worse than saying nothing: the reader would go re-check a key
        // that is fine.
        for m in [
            "429 rate limit exceeded, retry after 20s",
            "500 internal server error",
            "connection reset by peer",
            "context window exceeded",
            "the model returned an empty response",
        ] {
            assert!(!looks_like_auth_failure(m), "wrongly classified: {m}");
        }
    }
}

/// Does this provider error mean "your prompt did not fit"?
///
/// The message we classify is the provider's body verbatim, so the classifier has
/// to know every spelling the providers actually use. The underscore forms are
/// the ones that mattered: Anthropic answers `request_too_large`, and an earlier
/// version of this list carried only the spaced spelling — so a real overflow
/// failed the run instead of compacting once and retrying, which is the whole
/// point of the branch this feeds.
fn looks_like_context_overflow(msg: &str) -> bool {
    const NEEDLES: [&str; 12] = [
        "context window",
        "context_window",
        "context_length",
        "maximum context",
        "too many tokens",
        "request too large",
        "request_too_large",
        "prompt is too long",
        "prompt_too_long",
        "input is too long",
        "exceeds the maximum",
        "reduce the length",
    ];
    let m = msg.to_ascii_lowercase();
    NEEDLES.iter().any(|needle| m.contains(needle))
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

/// Run a turn-hook call to completion, containing a panic the way the provider seam's
/// `PanicGuard` contains a panicking stream (CORE-02).
///
/// The trait contract says a hook must not panic, but the loop cannot depend on a host
/// keeping it: an unwind out of `run_inner` would skip the cleanup that resets `running`,
/// after which every later run returns `RunError::AlreadyRunning`. On a panic the caller
/// gets `fallback` — the seam's no-op default (the input messages, `None`, `false`).
///
async fn contain_hook<T>(fut: impl std::future::Future<Output = T>, fallback: T) -> T {
    contain_panic(fut, move |_| fallback).await
}

/// [`contain_hook`]'s payload-aware form (M93): the same containment, but the panic's own
/// text reaches the fallback, so a contained failure can report what happened.
///
/// Used by both task bodies (detached and foreground), where the fallback is the terminal
/// `Failed` record F18 requires: an entry left `Running` after its run unwound would be a
/// lie, and a silently contained panic would not be a report.
///
/// Written against std only (no `futures-util` dependency in this crate): `pin!` + `poll_fn`
/// put the panic boundary exactly around `poll`, so a future that unwinds mid-await is caught
/// too, and a panicked future is never polled again.
async fn contain_panic<T>(
    fut: impl std::future::Future<Output = T>,
    on_panic: impl FnOnce(String) -> T,
) -> T {
    let mut fut = std::pin::pin!(fut);
    let mut panic: Option<Box<dyn std::any::Any + Send>> = None;
    let out = std::future::poll_fn(|cx| {
        if panic.is_some() {
            // The future panicked in an earlier poll: it must not be polled again, so the
            // containment settles immediately.
            return std::task::Poll::Ready(None);
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fut.as_mut().poll(cx))) {
            Ok(std::task::Poll::Ready(v)) => std::task::Poll::Ready(Some(v)),
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Err(payload) => {
                panic = Some(payload);
                std::task::Poll::Ready(None)
            }
        }
    })
    .await;
    match out {
        Some(v) => v,
        // A missing payload means the containment never saw a panic; `default` keeps this
        // branch panic-free as well.
        None => on_panic(panic.as_deref().map(panic_text).unwrap_or_default()),
    }
}

/// Printable form of a panic payload without assuming its type (the provider seam's
/// `PanicGuard` uses the same rule).
fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "opaque panic payload".to_string()
    }
}

/// The `RunError` for a session-store mutex poisoned by a panic (F07).
///
/// One constructor for the three write paths (`persist`, `pop_last_message`, compaction's
/// append), so a poisoned lock reads the same wherever it is reported rather than becoming a
/// per-call-site sentence no test can pin.
fn store_lock_failure() -> RunError {
    RunError::Internal("session store poisoned".into())
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

/// Clears the run-scoped state [`Agent::run`] sets when that run ends — however it ends (F07).
///
/// A plain local with a `Drop` rather than two cleanup statements at the end of `run()`: the
/// function now has early `?` returns, and the cleanup they used to skip is what keeps the agent
/// runnable at all.
struct RunGuard<'a> {
    agent: &'a Agent,
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.agent.run_token.lock() {
            *slot = None;
        }
        self.agent.running.store(false, Ordering::SeqCst);
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
    /// Discovered agent-type defs for `task` routing (M50: pi `agents.ts` parity).
    /// Interior-mutable so composition code can attach discoveries after Arc<Agent>.
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
    /// Interior-mutable so composition code can attach discoveries after Arc<Agent>.
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
    pub fn set_thinking_level(&self, level: epsilon_contracts::ThinkingLevel) {
        if let Ok(mut slot) = self.thinking_level.lock() {
            *slot = level;
        }
    }

    /// Current thinking level (session state, like the model id).
    pub fn thinking_level(&self) -> epsilon_contracts::ThinkingLevel {
        self.thinking_level.lock().map(|l| *l).unwrap_or_default()
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

    fn system_overlay(&self) -> crate::prompt::SystemOverlay {
        self.system_overlay
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    fn agent_list(&self) -> Vec<crate::agents::AgentDef> {
        self.agents.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn task_depth_slot(&self) -> Result<std::sync::MutexGuard<'_, usize>, String> {
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

    fn skill_list(&self) -> Vec<crate::skills::Skill> {
        self.skills.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Seed the transcript from a previous session (M12 `--resume`).
    ///
    /// Messages are restored into runtime history WITHOUT re-persisting (they already
    /// live in the durable mirror); subsequent messages append normally. Call before
    /// the first `run()`.
    pub fn restore_history(&self, messages: Vec<Message>) {
        *self
            .observation_scope
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Arc::new(());
        if let Ok(mut h) = self.history.lock() {
            *h = messages;
        }
        // Restored history did not come from a request this process made, so any
        // anchor held for the previous transcript is meaningless here.
        self.clear_usage_anchor();
        // ...and so is any run-scoped replacement built on top of it (CORE-02).
        self.bump_history_epoch();
    }

    /// Re-derive the model view from the store's *current* leaf and restore it into
    /// runtime history. Returns the number of messages now visible.
    ///
    /// This is what makes a session-tree jump a navigation rather than an
    /// inspection. `/tree` has always moved the store's leaf
    /// (`store.navigate_to`) and printed the resulting branch — which is why the
    /// audit called it a viewer: the file changed, the conversation did not. The
    /// user was left reading a transcript that no longer described the session.
    ///
    /// Deliberately **not** [`Self::live_resume`]. That reopens the file, which
    /// means dropping and re-acquiring the cross-process write lease — a jump
    /// within one session does not need that, and doing it would briefly release
    /// the lease on a file we are the only writer for, which is precisely the
    /// window another epsilon is waiting to steal (see the lease's dead-holder
    /// rule). It also runs `repair_on_open`, which is for a file that has been
    /// closed by someone else, not for one we are mid-conversation with.
    ///
    /// The leaf was already moved by the caller, so this only reads.
    ///
    /// Fails without touching runtime history if the store is absent or locked: a
    /// half-restored transcript is worse than the stale one the user was reading,
    /// because it is neither the old state nor the new one.
    pub fn reproject_current_branch(&self) -> Result<usize, String> {
        let Some(handle) = self.store() else {
            return Err("no session store attached".into());
        };
        // Derive the view under the store's lock, then release it before touching
        // `self.history` — two different locks, and holding both at once is how a
        // future caller deadlocks this.
        let messages: Vec<Message> = {
            let st = handle.lock().map_err(|_| "store lock".to_string())?;
            st.model_view()
                .map_err(|e| format!("read the session tree: {e}"))?
                .into_iter()
                .map(|v| v.into_message())
                .collect()
        };
        let count = messages.len();
        self.restore_history(messages);
        Ok(count)
    }

    /// Live in-TUI session switch (pi parity: `/resume` + `/import` without restart).
    ///
    /// Opens `path` as a [`SessionStore`], repairs dangling tool calls (same as CLI
    /// `--resume`), derives the shadowed model view, restores it into runtime
    /// history, and re-attaches the new store. The TUI then rehydrates the visible
    /// transcript from [`Self::history`]. Ponytail: reuse existing repair + model_view.
    pub fn live_resume(&self, path: &std::path::Path) -> Result<usize, String> {
        // A session you are resuming has to **exist**. `SessionStore::open` creates what
        // is missing, so without this a mistyped path silently produces an empty session —
        // and the next thing written goes into that new file, while the reader believes
        // they are continuing the conversation they named. That is data loss dressed as
        // success, and it is the one failure mode here that keeps no trace.
        if !path.is_file() {
            return Err(format!(
                "no session at {} — `resume` continues a conversation that exists; \
                 use a new session to start one",
                path.display()
            ));
        }
        // M61: the store owns a cross-process write lease, so re-resuming the
        // session this process already holds would conflict with ourselves.
        // Release first — and keep the old handle so a failed open (locked by
        // ANOTHER process, or corrupt) leaves the current session attached.
        let previous = self.store();
        let already_holding = previous
            .as_ref()
            .is_some_and(|h| h.lock().map(|s| s.path() == path).unwrap_or(false));
        // When we already hold this file the old store must be gone before the
        // reopen — including the clone kept for the rollback path, which would
        // otherwise keep the Arc (and its lease) alive through the open.
        let restore_target = if already_holding {
            self.detach_session_store();
            // The rollback clone would keep the store — and its lease — alive
            // through the reopen, so it must go too.
            drop(previous);
            None
        } else {
            previous
        };
        let opened = SessionStore::open(path);
        let store = match opened {
            Ok(store) => store,
            Err(e) => {
                if let Some(previous) = restore_target {
                    self.attach_session_store(previous);
                }
                return Err(format!("open {}: {e}", path.display()));
            }
        };
        // M169: the store is attached BEFORE anything else can fail. Above, the old one was
        // detached and dropped (to release its lease) when this is the file we already hold,
        // so an early return here used to leave the agent with no store at all - and every
        // TUI command that reads the session would then panic rather than describe the state.
        // Attaching first means a failed repair still costs the user their command, not the
        // whole session store.
        let handle = Arc::new(Mutex::new(store));
        self.attach_session_store(Arc::clone(&handle));
        // The attach above swaps in the new handle, dropping the previous store and with it
        // the previous lease. We keep our own clone so we can lock while repairing.
        let repaired = {
            let mut st = handle.lock().map_err(|_| "store lock".to_string())?;
            epsilon_session::repair::repair_on_open(&mut st)
        };
        match repaired {
            Ok(report) => {
                if !report.kinds().is_empty() {
                    // The kinds, not just a count: "a call had no result" and "the session ended
                    // mid-run" are different things to have lost (M75).
                    for line in report.summary() {
                        eprintln!("repaired {}: {line}", path.display());
                    }
                    if let Ok(mut slot) = self.last_repair.lock() {
                        *slot = Some(report);
                    }
                }
            }
            Err(e) => return Err(format!("repair {}: {e}", path.display())),
        }
        let msgs: Vec<Message> = {
            let st = handle.lock().map_err(|_| "store lock".to_string())?;
            st.model_view()
                .map_err(|e| format!("read {}: {e}", path.display()))?
                .into_iter()
                .map(|v| v.into_message())
                .collect()
        };
        let count = msgs.len();
        self.restore_history(msgs);
        Ok(count)
    }

    /// Name a recorded-vs-running directory mismatch, if there is one.
    ///
    /// Resuming (or switching into) a session recorded in another directory
    /// silently reinterprets every relative path against the running one —
    /// and a recorded directory that no longer exists makes it certain. The
    /// launch stays where it is (moving the process is not on offer); this
    /// names the fact so the reader, not the tooling, decides what it means.
    /// Empty recording (headerless v1 files) stays silent: no information,
    /// no noise.
    pub fn cwd_mismatch_note(recorded: &str, current: &std::path::Path) -> Option<String> {
        let recorded = recorded.trim();
        if recorded.is_empty() {
            return None;
        }
        if Self::same_directory(std::path::Path::new(recorded), current) {
            return None;
        }
        if !std::path::Path::new(recorded).is_dir() {
            return Some(format!(
                "session was recorded in {recorded}, which no longer exists — tools run in {}",
                current.display()
            ));
        }
        Some(format!(
            "session was recorded in {recorded} — tools run in {}",
            current.display()
        ))
    }

    /// Read `path`'s recorded cwd and name a mismatch against `current`.
    ///
    /// The one call both front ends make after a resume lands: headerless or
    /// unreadable headers stay silent (no information, no noise), everything
    /// else goes through [`Self::cwd_mismatch_note`].
    pub fn resume_cwd_note(path: &std::path::Path, current: &std::path::Path) -> Option<String> {
        let recorded = epsilon_session::discover::read_session_header(path)?.cwd;
        Self::cwd_mismatch_note(&recorded, current)
    }

    /// Whether two paths name the same directory.
    ///
    /// Canonicalized comparison when both exist (which settles separators,
    /// case, and symlinks on every platform); a normalized string fallback
    /// when they do not, so a gone directory still compares sanely.
    fn same_directory(a: &std::path::Path, b: &std::path::Path) -> bool {
        if let (Ok(ca), Ok(cb)) = (a.canonicalize(), b.canonicalize()) {
            return ca == cb;
        }
        fn norm(p: &std::path::Path) -> String {
            let mut s = p.to_string_lossy().into_owned();
            while s.len() > 1 && (s.ends_with('/') || s.ends_with('\\')) {
                s.pop();
            }
            #[cfg(windows)]
            {
                s.to_ascii_lowercase()
            }
            #[cfg(not(windows))]
            {
                s
            }
        }
        norm(a) == norm(b)
    }

    /// Whether a run is currently active (for in-TUI gate on live resume).
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
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

    /// Queue a mid-run interjection; drained at the next drain point (after current turn).
    pub fn steer(&self, m: Message) {
        if let Ok(mut q) = self.steering.lock() {
            q.push(m);
        }
    }

    // M30: queue visibility — the TUI reads this to show a count badge in the
    // status bar so the user knows their interjections are queued, not lost.
    /// Number of steering messages waiting to be delivered after the current turn.
    pub fn steer_queue_len(&self) -> usize {
        self.steering.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// How much of the steering queue releases at a drain point.
    ///
    /// pi's `setSteeringMode` (`rpc-mode.ts`). Readable as well as settable because a
    /// client that changes a setting needs to be able to see what is in effect: an
    /// unreadable setting is one the client cannot confirm and can only guess at.
    pub fn steering_mode(&self) -> QueueMode {
        self.steering
            .lock()
            .map(|q| q.mode())
            .unwrap_or(QueueMode::OneAtATime)
    }

    /// Set how much of the steering queue releases at a drain point.
    ///
    /// **Takes effect at the next drain, and does not disturb what is already queued.**
    /// The mode is a property of the *drain*, not of the queue's contents, so switching
    /// from `OneAtATime` to `All` with three messages waiting releases all three at the
    /// next drain point and switching the other way releases one. The alternative —
    /// draining on the spot — would deliver messages the user never asked to be released
    /// now, in the middle of a turn.
    pub fn set_steering_mode(&self, mode: QueueMode) {
        if let Ok(mut q) = self.steering.lock() {
            q.set_mode(mode);
        }
    }

    /// How much of the follow-up queue releases at a drain point (pi's
    /// `setFollowUpMode`).
    pub fn follow_up_mode(&self) -> QueueMode {
        self.followups
            .lock()
            .map(|q| q.mode())
            .unwrap_or(QueueMode::OneAtATime)
    }

    /// Set how much of the follow-up queue releases at a drain point.
    ///
    /// The same mid-queue semantics as [`Agent::set_steering_mode`], and the same reason:
    /// the mode belongs to the drain, so changing it must not itself deliver anything.
    pub fn set_follow_up_mode(&self, mode: QueueMode) {
        if let Ok(mut q) = self.followups.lock() {
            q.set_mode(mode);
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
        // A new store is a different file, so a failure recorded against the old one no longer
        // describes what requests would be built from (F07). The epoch bump below is the same
        // idea for run-scoped replacements.
        if let Ok(mut slot) = self.durable_write_failure.lock() {
            *slot = None;
        }
        // The provider view's source just changed (CORE-02): a run-scoped replacement
        // built against the previous source must not outlive it, or the rest of the run
        // would keep reading a pre-swap view — the same class of staleness a committed
        // compaction creates, handled by the epoch check before each request.
        self.bump_history_epoch();
    }

    /// Drop the attached durable mirror, releasing its write lease (M61). The
    /// agent keeps running — a caller that releases the store has decided this
    /// transcript is no longer written by this process.
    pub fn detach_session_store(&self) {
        if let Ok(mut slot) = self.session_store.lock() {
            *slot = None;
        }
        // Same reason as `attach_session_store`: the request path's source changes here.
        self.bump_history_epoch();
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

    fn store(&self) -> Option<Arc<Mutex<SessionStore>>> {
        self.session_store.lock().ok().and_then(|s| s.clone())
    }

    /// Test/inspection handle to the attached durable mirror.
    #[doc(hidden)]
    pub fn store_handle_for_tests(&self) -> Arc<Mutex<SessionStore>> {
        self.store().expect("no session store attached")
    }

    /// The attached durable mirror, when there is one.
    ///
    /// The non-panicking counterpart of [`Agent::store_handle_for_tests`], for frontends: an
    /// agent with no session file (a plain `--tui` run, a `task` child before its store exists)
    /// is a normal state, and a command like `/session` has to describe it rather than crash.
    pub fn session_store(&self) -> Option<Arc<Mutex<SessionStore>>> {
        self.store()
    }

    /// Path of the attached session file, when one is attached (M57: the TUI's
    /// session browser marks the current file — it must not panic on an agent
    /// built without a durable mirror).
    pub fn session_path(&self) -> Option<std::path::PathBuf> {
        let store = self.store()?;
        store.lock().ok().map(|st| st.path().to_path_buf())
    }

    /// The attached session's own id, from its header (`SessionStore::header`).
    ///
    /// `None` without a durable mirror, and `None` for a store whose header could not be
    /// read — the same "absent rather than fiction" rule the rest of this snapshot keeps.
    /// A client identifies a conversation by this, not by [`Agent::session_path`]: a fork
    /// moves the conversation to a new file, and a rename moves the file under the
    /// conversation.
    pub fn session_id(&self) -> Option<String> {
        let store = self.store()?;
        let guard = store.lock().ok()?;
        guard.header().map(|h| h.id.clone())
    }

    /// The session's display name, as `/name` or RPC `set_session_name` last set it.
    ///
    /// Read from the newest `Custom { "session_name" }` entry, because the name can be
    /// written anywhere in the file and the newest one wins — the same rule pi's session
    /// listing uses. `None` when no name was ever set, which is not an error.
    pub fn session_name(&self) -> Option<String> {
        let store = self.store()?;
        let guard = store.lock().ok()?;
        guard.session_name()
    }

    // ---- the session goal (ADR-006 / DSH's goal domain) -------------------------------
    //
    // One long-running objective per session, persisted in the session log and nowhere else.
    // Everything a reader needs is `goal()`; everything a writer needs is `goal_apply`, and
    // the two differences from DSH's service are deliberate and named:
    //
    //   * there is no async surface, because the store is a `std::sync` mutex over file I/O
    //     and awaiting while holding it is how a run deadlocks;
    //   * activation is one flag on the Agent rather than a per-session cache, because
    //     Epsilon has one live conversation per Agent.

    /// The current goal, with whether this process may continue it on its own.
    ///
    /// `None` covers three situations that a caller must not conflate, which is why the
    /// *status* is in the view and not in this return: no goal was ever set, the goal was
    /// cleared, and there is no durable mirror to read one from (a `task` child before its
    /// store exists). A frontend that needs to tell them apart asks
    /// [`Agent::session_store`].
    pub fn goal(&self) -> Option<GoalView> {
        let store = self.store()?;
        let goal = store.lock().ok()?.goal().ok()??;
        Some(GoalView {
            goal,
            armed: self.goal_armed(),
        })
    }

    /// Whether automatic continuation is armed in this process.
    ///
    /// Never persisted: a goal that survives a restart does not get to keep going by itself,
    /// because the person who started it is not necessarily the person looking at it now.
    pub fn goal_armed(&self) -> bool {
        self.goal_armed.load(Ordering::SeqCst)
    }

    /// Arm or disarm automatic continuation without touching the durable state.
    ///
    /// This is the `disarm()` half of DSH's activation model: it writes no revision and emits
    /// no mutation, because "may this process continue" is not part of what a goal *is*.
    pub fn set_goal_armed(&self, armed: bool) {
        self.goal_armed.store(armed, Ordering::SeqCst);
    }

    /// Apply one goal operation, with compare-and-set.
    ///
    /// `expect` is the revision the caller read: `None` means "whatever is current" and is for a
    /// frontend acting on what the user is looking at; `Some` is for a caller holding an older
    /// view, which is every model tool call.
    ///
    /// The rules, each one DSH's and each one load-bearing:
    ///
    /// * **One current goal.** A create is refused while an unfinished goal holds the slot
    ///   (`GoalError::AlreadyCurrent`); a completed one may be replaced, because the alternative
    ///   is making the user clear a goal they just finished.
    /// * **Resume clears the blocker** and refuses when the round cap is spent — rearming an
    ///   exhausted goal would only fire the guard again.
    /// * **Create and resume arm; pause, complete, blocked and clear disarm.** Completion is the
    ///   only phase that is terminal, and blocking is the only one that carries a reason.
    pub fn goal_apply(
        &self,
        expect: Option<GoalRef>,
        op: GoalOp,
    ) -> Result<Option<GoalView>, GoalError> {
        let store = self.store().ok_or(GoalError::NoStore)?;
        let mut guard = store.lock().map_err(|_| GoalError::NoStore)?;
        let current = guard
            .goal()
            .map_err(|e| GoalError::Invalid(e.to_string()))?;
        let previous_revision = current.as_ref().map(|g| g.revision);
        let is_create = matches!(op, GoalOp::Create { .. });

        // Compare-and-set first, so a stale caller is told the goal moved rather than being
        // handed a refusal about the operation it asked for.
        if let Some(expected) = &expect {
            match current.as_ref() {
                Some(actual)
                    if expected.id == actual.id && expected.revision == actual.revision => {}
                Some(actual) => {
                    return Err(GoalError::Stale {
                        expected: expected.clone(),
                        actual: actual.as_ref(),
                    });
                }
                None => return Err(GoalError::NoGoal),
            }
        }

        let now = now_ms();
        let next: GoalSnapshot = match (&op, current) {
            (
                GoalOp::Create {
                    objective,
                    max_rounds,
                },
                None,
            ) => new_goal(objective, *max_rounds, now)?,
            (GoalOp::Create { .. }, Some(existing)) if !existing.status.is_terminal() => {
                return Err(GoalError::AlreadyCurrent {
                    current: existing.as_ref(),
                });
            }
            // Replacing a completed goal takes a new identity, so the finished one stays
            // addressable in the log and a stale ref to it can never match the new goal.
            (
                GoalOp::Create {
                    objective,
                    max_rounds,
                },
                Some(_),
            ) => new_goal(objective, *max_rounds, now)?,
            (_, None) => return Err(GoalError::NoGoal),
            (
                GoalOp::Edit {
                    objective,
                    max_rounds,
                    gates,
                },
                Some(mut g),
            ) => {
                if objective.is_none() && max_rounds.is_none() && gates.is_none() {
                    return Err(GoalError::Invalid(
                        "edit needs an objective, a round cap, a gate list, or some combination"
                            .into(),
                    ));
                }
                if let Some(objective) = objective {
                    let objective = objective.trim();
                    if objective.is_empty() {
                        return Err(GoalError::Invalid(
                            "an edit cannot blank the objective; clear the goal instead".into(),
                        ));
                    }
                    g.objective = objective.to_string();
                }
                if let Some(max_rounds) = max_rounds {
                    if *max_rounds == 0 {
                        return Err(GoalError::Invalid("max_rounds must be at least 1".into()));
                    }
                    g.max_rounds = *max_rounds;
                }
                if let Some(gates) = gates {
                    // An empty gate list is a **request**, not a no-op: it is how a user
                    // takes the rule back off a goal, and a rule that cannot be removed
                    // is a rule the user is stuck with.
                    if let Some(bad) = gates.iter().find(|g| !g.is_runnable()) {
                        let _ = bad;
                        return Err(GoalError::Invalid(
                            "a gate needs a command; one with an empty command could never pass \
                             and would block completion for ever"
                                .into(),
                        ));
                    }
                    g.gates.clone_from(gates);
                }
                g
            }
            (GoalOp::Pause, Some(mut g)) => {
                g.status = GoalStatus::Paused;
                g
            }
            (GoalOp::Resume, Some(mut g)) => {
                if g.exhausted() {
                    return Err(GoalError::RoundsExhausted {
                        rounds_started: g.rounds_started,
                        max_rounds: g.max_rounds,
                    });
                }
                g.status = GoalStatus::Active;
                // Resume clears a former blocker: the phase and the reason behind it are one
                // fact, and a goal that is active again while still carrying a blocker would
                // show a reason nobody can act on.
                g.blocker = None;
                g
            }
            (GoalOp::Complete, Some(mut g)) => {
                g.status = GoalStatus::Complete;
                g.blocker = None;
                g
            }
            (GoalOp::Blocked { blocker }, Some(mut g)) => {
                if blocker.code.trim().is_empty() || blocker.reason.trim().is_empty() {
                    return Err(GoalError::Invalid(
                        "a blocker needs a code and a reason; a blocked goal nobody can explain \
                         is indistinguishable from one that stalled"
                            .into(),
                    ));
                }
                g.status = GoalStatus::Blocked;
                g.blocker = Some(blocker.clone());
                g
            }
            (GoalOp::Clear, Some(g)) => {
                let change = GoalChange::Cleared {
                    id: g.id.clone(),
                    revision: g.revision + 1,
                };
                guard
                    .append_goal(change)
                    .map_err(|e| GoalError::Invalid(e.to_string()))?;
                self.set_goal_armed(false);
                // `None` is the outcome, not a failure: the caller asked for the goal to be
                // gone, and it is gone.
                return Ok(None);
            }
        };

        let mut next = next;
        if !is_create {
            // Monotonic by one, which is what makes the compare-and-set ref meaningful: a
            // revision that jumped would let a stale caller's ref match a later state.
            next.revision = previous_revision.unwrap_or(0) + 1;
        }
        next.updated_at_ms = now;
        guard
            .append_goal(GoalChange::Set { goal: next.clone() })
            .map_err(|e| GoalError::Invalid(e.to_string()))?;
        // Arming is the caller-visible half of the same decision, and it happens only after the
        // durable write succeeded: a process that armed itself on a failed append would continue
        // a goal the next reader cannot see.
        match op {
            GoalOp::Create { .. } | GoalOp::Resume => self.set_goal_armed(true),
            GoalOp::Pause | GoalOp::Complete | GoalOp::Blocked { .. } => self.set_goal_armed(false),
            GoalOp::Edit { .. } | GoalOp::Clear => {}
        }
        Ok(Some(GoalView {
            goal: next,
            armed: self.goal_armed(),
        }))
    }

    /// Spend one round against the current goal, durably, and return the prompt to send.
    ///
    /// The round is counted when it is *sent*, not when it succeeds: a round that crashed on the
    /// way is a round that was spent, and a counter that only advanced on success would let a
    /// loop that fails instantly run for ever.
    ///
    /// `None` means this process must not continue — no goal, not active, not armed, or the cap
    /// is reached — which is the same condition [`GoalView::is_continuable`] states, evaluated
    /// against the log at the moment the decision is made rather than against a view read
    /// earlier.
    pub fn goal_begin_round(&self) -> Result<Option<String>, GoalError> {
        let Some(view) = self.goal() else {
            return Ok(None);
        };
        if !view.is_continuable() {
            return Ok(None);
        }
        let mut goal = view.goal;
        goal.rounds_started += 1;
        goal.revision += 1;
        goal.updated_at_ms = now_ms();
        let store = self.store().ok_or(GoalError::NoGoal)?;
        store
            .lock()
            .map_err(|_| GoalError::NoGoal)?
            .append_goal(GoalChange::Set { goal: goal.clone() })
            .map_err(|e| GoalError::Invalid(e.to_string()))?;
        Ok(Some(goal_round_prompt(&goal)))
    }

    /// Build the live state snapshot both wire surfaces serve.
    ///
    /// **Why this is a method and not a free function in each frontend.** `rpc_mode.rs`
    /// and `epsilon-server` each grew their own `SessionState { … }` literal, and they
    /// were identical — until a field was added and only one of them learned it. That is
    /// D29's mechanism applied to a *shared* struct: the payload exists in one place
    /// precisely so the two transports cannot disagree, and two builders put that back
    /// on a knife edge. One constructor, two callers, no third spelling.
    ///
    /// It lives here rather than in `epsilon-contracts` because it needs an `Agent`, and
    /// contracts may not depend on core — so this is the only crate that can hold it.
    /// The alternative, a `SessionState::from(&Agent)` in contracts, is forbidden by the
    /// architecture law and would be the `cli -> server` edge in disguise.
    pub fn session_state(&self) -> epsilon_contracts::SessionState {
        epsilon_contracts::SessionState {
            model: self.model_id(),
            thinking_level: self.thinking_level(),
            is_streaming: self.is_running(),
            steering_mode: self.steering_mode(),
            follow_up_mode: self.follow_up_mode(),
            message_count: self.history().len(),
            pending_message_count: self.steer_queue_len(),
            session_file: self.session_path().map(|p| p.display().to_string()),
            session_id: self.session_id(),
            session_name: self.session_name(),
            estimated_context_tokens: self.estimated_context_tokens(),
            // Additive to pi's payload (M206): a goal is this harness's own concept, and a client
            // that renders one needs the phase *and* the activation, because an `active` goal in a
            // fresh process is disarmed and will not continue by itself.
            goal: self
                .goal()
                .map(|view| epsilon_contracts::session_state::SessionGoal {
                    id: view.goal.id,
                    revision: view.goal.revision,
                    objective: view.goal.objective,
                    status: view.goal.status,
                    rounds_started: view.goal.rounds_started,
                    max_rounds: view.goal.max_rounds,
                    armed: view.armed,
                    blocker: view.goal.blocker,
                }),
        }
    }

    /// Search the attached session log (pi `search/scanning.ts` parity).
    ///
    /// Case-insensitive substring scan over user/assistant/tool-result text, newest
    /// hits first; `None` when no durable mirror is attached. Locking note: the
    /// store mutex is held only for the scan duration and the scan never writes,
    /// so a concurrent run can proceed on other locks.
    pub fn search_history(
        &self,
        query: &str,
        opts: &epsilon_session::SearchOptions,
    ) -> Option<Result<Vec<epsilon_session::SessionSearchHit>, epsilon_session::StoreError>> {
        let store = self.store()?;
        let guard = store.lock().ok()?;
        Some(epsilon_session::search_session(&guard, query, opts))
    }

    /// Append a message to the durable mirror when one is attached.
    ///
    /// Fallible, and deliberately loud (F07). The provider request reads the store's
    /// *projection*, so a durable append that fails while runtime history keeps the message
    /// leaves the two views describing different conversations: the assistant believes the turn
    /// was recorded and the next request is built from a transcript that does not contain it.
    /// The old code discarded the result (`let _ = st.append(..)`), which is exactly the silent
    /// divergence F07 names.
    ///
    /// On failure this emits an [`AgentEvent::Notice`] — every frontend renders notices, and the
    /// `Err` alone would reach only the callers that look at it (the TUI discards `run()`'s
    /// result, so an error nobody prints is not a report) — and returns [`RunError::Internal`]
    /// for the caller to propagate. Callers that can propagate stop the run; see `run()`'s
    /// prompt append and `run_inner`'s call sites.
    fn persist(&self, m: &Message) -> Result<(), RunError> {
        let Some(store) = self.store() else {
            return Ok(()); // no durable mirror attached: nothing can diverge
        };
        let mut st = store.lock().map_err(|_| store_lock_failure())?;
        st.append(EntryKind::Message {
            message: m.clone(),
            api_request_sha256: None,
        })
        .map_err(|e| self.durable_write_failed(format!("append: {e}")))?;
        Ok(())
    }

    /// Record a failed durable write: announce it, remember it, and return the run error.
    ///
    /// The notice is what makes the failure visible — the TUI prints notices and throws away the
    /// `Err`, so a silent `Err` would be the same invisible failure in a different shape. The
    /// sentence names the consequence the user needs ("the session may be missing a turn") and
    /// what to do next, rather than only the operation that failed.
    ///
    /// The latch closes the second half of F07. Stopping the run is only possible on the paths
    /// that return a `Result`, and four of them cannot (the abort/EOF salvage arms return the
    /// message, not an error). A later request on this agent must not be built from a store view
    /// that is now missing a message the loop believes it recorded, so [`Agent::try_context_view`]
    /// asks this latch first and fails closed. The next provider request therefore never runs
    /// against a transcript that disagrees with the loop's own record.
    fn durable_write_failed(&self, what: String) -> RunError {
        let detail = format!(
            "the session file could not record this turn ({what}). The turn is in this \
             session's memory but not on disk: a resume, or the next request, may be missing it. \
             Check the disk and the session file, then retry the turn."
        );
        self.emit(AgentEvent::Notice {
            text: detail.clone(),
        });
        if let Ok(mut slot) = self.durable_write_failure.lock() {
            *slot = Some(what);
        }
        RunError::Internal(detail)
    }

    /// Remove the most recently pushed message (overflow-retry discards failed attempts).
    ///
    /// Both views must lose the message together for the same reason [`Agent::persist`] exists:
    /// the store's projection is what the next request is built from, so a pop that fails on the
    /// store while runtime history drops the message leaves the store *ahead* — still carrying
    /// the attempt the loop just discarded as oversized, which the retry would send again.
    fn pop_last_message(&self) -> Result<(), RunError> {
        if let Ok(mut h) = self.history.lock() {
            h.pop();
        }
        if let Ok(store) = self.session_store.lock()
            && let Some(st) = store.as_ref()
        {
            let mut s = st.lock().map_err(|_| store_lock_failure())?;
            s.pop_last()
                .map_err(|e| self.durable_write_failed(format!("pop last: {e}")))?;
        }
        Ok(())
    }

    /// Current transcript snapshot (all roles, including harness-only messages).
    ///
    /// Returns an owned `Vec` because most callers want to keep or filter it, and
    /// a deep clone of a long session is not free. For callers that only need to
    /// *look*, prefer [`Self::with_history`]: this is a full clone of every
    /// message, tool arguments and all, and one such call per drawn frame is
    /// enough to make an input-latency problem out of a rendering problem.
    pub fn history(&self) -> Vec<Message> {
        self.history.lock().map(|h| h.clone()).unwrap_or_default()
    }

    /// Run `f` over a borrowed view of the transcript, without cloning it.
    ///
    /// This exists for the per-frame callers — the TUI's context mirror, and any
    /// other read that happens on a redraw. Holding the lock for the duration of
    /// `f` is what makes the borrow sound; `f` therefore cannot itself call back
    /// into a method that takes the same lock, which is the one real constraint
    /// and is a good one, because that would be a deadlock rather than a slow
    /// path.
    pub fn with_history<T>(&self, f: impl FnOnce(&[Message]) -> T) -> T {
        match self.history.lock() {
            Ok(h) => f(&h),
            // A poisoned lock still holds readable data; `history()` degrades to
            // empty in the same situation, and an empty transcript would make the
            // context gauge read `0/164k`, which is a lie of the same kind this
            // function exists to prevent.
            Err(poisoned) => f(&poisoned.into_inner()),
        }
    }

    fn push_message(&self, m: &Message) -> Result<(), RunError> {
        if let Ok(mut h) = self.history.lock() {
            h.push(m.clone());
        }
        self.persist(m)
    }

    /// What the session resume had to repair, if anything (M75): the *kinds*, so a frontend can
    /// say what was missing rather than printing a bare count.
    pub fn repair_report(&self) -> Option<epsilon_session::repair::RepairReport> {
        self.last_repair.lock().ok().and_then(|r| r.clone())
    }

    /// The workspace this agent was built for: where relative paths resolve, and what the
    /// `@`-reference expander confines to (M74). Frontends need it to build user messages
    /// through the shared constructor rather than each rolling their own.
    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    /// Remember that a tool call acted on `path`, so its directory's instructions can be
    /// loaded (M73). Cheap by construction: it records a directory and reads nothing — the
    /// prompt asks [`prompt::load_subdirectory_hints`] at assembly time.
    pub fn note_touched(&self, path: &std::path::Path) {
        let Some(dir) = path.parent() else { return };
        if !dir.starts_with(&self.workspace_root) || dir == self.workspace_root {
            return; // outside the workspace, or the root — already loaded as project context
        }
        if let Ok(mut set) = self.touched_dirs.lock() {
            set.insert(dir.to_path_buf());
        }
    }

    /// Directories touched so far, sorted: the hint loader must not see a walk order.
    fn touched_dirs(&self) -> Vec<std::path::PathBuf> {
        let mut v: Vec<_> = self
            .touched_dirs
            .lock()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    /// Give every unanswered tool call a result that says what happened.
    ///
    /// A call with no result is not a cosmetic problem: providers validate tool-call pairing, so a
    /// transcript that carries one is rejected outright, and the failure arrives on the *next*
    /// request, far from its cause. The synthesized result is an error and says it was never
    /// executed — a pretend success would be worse than the invalid transcript.
    ///
    /// Returns how many calls were closed.
    fn close_dangling_tool_calls(&self) -> Result<usize, RunError> {
        let mut pending: Vec<(String, String)> = Vec::new();
        for m in self.history() {
            match m {
                Message::Assistant(a) => {
                    for (id, name, _) in a.tool_calls() {
                        pending.push((id.to_string(), name.to_string()));
                    }
                }
                Message::ToolResult(t) => pending.retain(|(id, _)| id != &t.tool_call_id),
                _ => {}
            }
        }
        if pending.is_empty() {
            return Ok(0);
        }
        let closed = pending.len();
        for (id, name) in pending {
            let m = Message::ToolResult(epsilon_contracts::message::ToolResultMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: now_ms(),
                tool_call_id: id,
                tool_name: name,
                content: vec![epsilon_contracts::content::ContentBlock::text(
                    "[harness] this call was never executed: the previous run ended before its \
                     result arrived. Re-issue it if the work is still needed.",
                )],
                details: serde_json::json!({"interrupted": true}),
                usage: None,
                is_error: true,
            });
            self.push_message(&m)?;
        }
        Ok(closed)
    }

    fn emit(&self, ev: AgentEvent) {
        let _ = self.events_tx.send(Arc::new(ev));
    }

    /// The real provider stream, wrapped in ADR-010's retry policy (M60).
    ///
    /// The policy existed — classification, backoff, a tested executor — but
    /// nothing ever wrapped a production stream with it, so a transient 429/503
    /// or a stalled socket failed the turn outright. Scheduled waits surface as
    /// [`AgentEvent::Notice`] so the pause is explained instead of looking hung.
    #[cfg(feature = "real")]
    pub fn retrying_real_stream(&self) -> Arc<dyn epsilon_llm::StreamFn> {
        let tx = self.events_tx.clone();
        Arc::new(
            epsilon_llm::Retrying::new(Arc::new(epsilon_llm::PanicGuard(
                epsilon_llm::DirectProvider::new(),
            )))
            .with_max_attempts(epsilon_llm::RETRY_MAX_ATTEMPTS)
            .with_on_retry(move |n| {
                let _ = tx.send(Arc::new(AgentEvent::Notice {
                    text: retry_notice_text(n),
                }));
            }),
        )
    }

    /// The run's context view, shared by the turn hooks and the request builder
    /// (CORE-02): the durable mirror's shadowed model view when one is attached,
    /// else runtime history. A `prepare_next_turn` hook sees exactly the messages
    /// the next provider request would start from.
    ///
    /// Fallible on purpose, and the ONE read both callers share: a poisoned store lock or a
    /// failed `model_view()` is `RunError::Internal` for the request path, and for the hook
    /// seam it means "no view this turn" — a hook must not be handed a view the request
    /// would refuse. Two implementations of this read (one lenient, one strict) is exactly
    /// how the two could ever disagree.
    ///
    /// A failed durable write poisons this read for the rest of the attached store's life
    /// (F07). The store's projection is what requests are built from, so once a write it should
    /// have recorded has failed, the projection is missing a message runtime history holds —
    /// and serving it would hand the provider a transcript that silently disagrees with the
    /// loop's own record. Refusing is the honest answer: the caller reports a run error, and
    /// the user has already been told by the notice the failure emitted.
    ///
    /// Read-only by construction: the store lock is held for the read and nothing
    /// here can write it (ADR-007 — hooks never touch the write path).
    fn try_context_view(&self) -> Result<Vec<Message>, RunError> {
        if let Ok(slot) = self.durable_write_failure.lock()
            && let Some(what) = slot.as_deref()
        {
            return Err(RunError::Internal(format!(
                "the session file is missing a turn this session recorded ({what}), so no \
                 request can be built from it. The transcript in memory is intact; resume \
                 this session after checking the disk to see what was lost."
            )));
        }
        match self.store() {
            Some(store) => {
                let st = store
                    .lock()
                    .map_err(|_| RunError::Internal("store poisoned".into()))?;
                Ok(st
                    .model_view()
                    .map_err(|e| RunError::Internal(format!("session view: {e}")))?
                    .into_iter()
                    .map(|v| v.into_message())
                    .collect())
            }
            None => Ok(self.history()),
        }
    }

    /// pi's `currentContext.messages.push` for Epsilon's run-scoped replacement context
    /// (CORE-02, `agent-loop.ts:186,219`): every message the loop adds to runtime history
    /// during a run — drained steering and nudges, the turn's assistant message, its tool
    /// results — lands in the replacement too, so the provider sees the run's own output
    /// instead of the frozen install-time snapshot. No-op while no replacement is active.
    fn append_to_turn_context(turn_context: &mut Option<Vec<Message>>, m: &Message) {
        if let Some(ctx) = turn_context.as_mut() {
            ctx.push(m.clone());
        }
    }

    /// Run one prompt to completion.
    ///
    /// Returns `AlreadyRunning` rather than queueing — concurrent runs on one agent are a
    /// caller bug, not a workload to silently serialize.
    pub async fn run(self: &Arc<Self>, prompt: Message) -> Result<RunOutcome, RunError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(RunError::AlreadyRunning);
        }
        // F07: the early failure paths below (a dangling-call repair that cannot be recorded, a
        // prompt that cannot be appended) return before the old cleanup at the end of this
        // function, which would have left `running` set and every later run answering
        // `AlreadyRunning` — a durable-write failure would have looked like a wedged agent. The
        // guard runs the same cleanup on every exit, including those.
        let _cleanup = RunGuard { agent: self };

        // Fresh run-scoped token (M12): tools observe it via ToolContext; abort ends
        // the run cooperatively instead of only via process exit.
        let token = tokio_util::sync::CancellationToken::new();
        if let Ok(mut slot) = self.run_token.lock() {
            *slot = Some(token.clone());
        }

        if let Ok(mut s) = self.doom_streaks.lock() {
            s.clear();
        }
        if let Ok(mut g) = self.progress.lock() {
            *g = crate::guards::ProgressGuard::new();
        }

        // An interrupted run leaves calls without results. Session repair handles that for a
        // *reopened* file (`epsilon_session::repair::repair_on_open`); this is the same shape in a
        // live process, where the next request would otherwise carry a tool call nothing answered
        // — structurally invalid for strict wires, and a 400 rather than a puzzle to solve later.
        //
        // `?` since F07: closing a dangling call is a durable append, and a run must not start on
        // a store that just failed to record one. The guard below still has to undo `running`.
        let closed = self.close_dangling_tool_calls()?;
        if closed > 0 {
            self.emit(AgentEvent::Notice {
                text: format!("closed {closed} tool call(s) left unanswered by the previous run"),
            });
        }
        // Overflow recovery (ADR-007 trigger 2) is once PER RUN, not per process:
        // reset the latch here or the first recovered overflow disables recovery for
        // every later run in a long-lived TUI/server session.
        self.overflow_retried.store(false, Ordering::SeqCst);
        self.compact_suppressed.store(false, Ordering::SeqCst);
        // F07: the prompt's own append is the first durable write of the run, and a turn whose
        // prompt never reached the file must not start — the store's projection would be missing
        // the very message the run is answering.
        self.push_message(&prompt)?;
        self.emit(AgentEvent::AgentStart);

        let outcome = self.run_inner(prompt).await;
        self.emit(AgentEvent::AgentEnd {
            messages: outcome
                .as_ref()
                .map(|o| o.produced.clone())
                .unwrap_or_default(),
        });
        outcome
    }

    /// Cancel the active run's token: in-flight tools see cancellation and the loop
    /// stops at the next checkpoint with `TurnEndReason::Aborted`.
    ///
    /// Returns false when no run is active. Idempotent.
    pub fn abort_current_run(&self) -> bool {
        let cancelled = if let Ok(slot) = self.run_token.lock()
            && let Some(token) = slot.as_ref()
        {
            token.cancel();
            true
        } else {
            // No run in flight: arm the flag so a run that is about to start
            // (a just-spawned child, or a submit racing the abort) still stops.
            self.abort_armed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            false
        };
        // M54: a delegated child is its own Agent with its own run token, so
        // aborting the parent must reach the children too — otherwise a
        // background subagent keeps spending tokens after the user pressed Esc.
        self.abort_delegated_tasks();
        cancelled
    }

    /// Ask every running delegated child to stop; returns how many were asked.
    ///
    /// Cascaded from [`Agent::abort_current_run`]; callable directly by a
    /// frontend that wants to stop children without a parent run in flight.
    /// Finished tasks are left alone (their output is the point).
    pub fn abort_delegated_tasks(&self) -> usize {
        let children: Vec<Arc<Agent>> = self
            .tasks
            .lock()
            .map(|v| {
                v.iter()
                    .filter(|e| e.state == epsilon_contracts::tool::TaskState::Running)
                    .map(|e| Arc::clone(&e.child))
                    .collect()
            })
            .unwrap_or_default();
        let mut asked = 0;
        for child in children {
            // Cancels a live token or arms the child's pre-start flag — either
            // way the child was told to stop.
            child.abort_current_run();
            asked += 1;
        }
        asked
    }

    /// Clone of the active run's token for `ToolContext` distribution.
    fn current_run_token(&self) -> Option<tokio_util::sync::CancellationToken> {
        self.run_token
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
    }

    fn clone_task_host(self: &Arc<Self>) -> Arc<dyn epsilon_contracts::tool::TaskHost> {
        Arc::clone(self) as Arc<dyn epsilon_contracts::tool::TaskHost>
    }

    /// The [`GoalHost`] view of this agent, for a `ToolContext`.
    ///
    /// **Public because composition needs it, not because a test wanted it.** The execution path
    /// builds this itself (`exec.rs`), and a frontend that wants to invoke a goal tool outside a
    /// turn — or a test that wants to drive the tool against a *real* goal rather than a snapshot
    /// of one — needs the same adapter. A second implementation would be a second answer to
    /// "where does a goal live", which is the thing that goes wrong when there are two.
    pub fn clone_goal_host(self: &Arc<Self>) -> Arc<dyn epsilon_contracts::tool::GoalHost> {
        Arc::clone(self) as Arc<dyn epsilon_contracts::tool::GoalHost>
    }

    async fn run_inner(self: &Arc<Self>, _prompt: Message) -> Result<RunOutcome, RunError> {
        // M54: an abort that arrived before this run started still ends it —
        // consume the flag once, so it can never leak into a later run.
        if self
            .abort_armed
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(RunOutcome {
                produced: Vec::new(),
                reason: TurnEndReason::Aborted,
                usage: Usage::default(),
            });
        }
        let mut produced: Vec<Message> = Vec::new();
        let mut usage_total = Usage::default();
        let mut reason = TurnEndReason::Completed;
        let mut turn_index: usize = 0;
        // Turns that came back empty or degenerate, for the bounded-retry guard below.
        let mut empty_turns: usize = 0;
        // Verification evidence for THIS run (port plan item 5): edits and the checks that ran
        // after them. Run-scoped: a fresh run is a fresh question.
        let mut evidence = crate::verify::VerificationLog::new();
        let mut verification_nudges = 0usize;

        // CORE-02 run-scoped hook state, mirroring pi's mutable `currentContext` /
        // `config` locals (`agent-loop.ts:163-164`): a `prepareNextTurn` snapshot applies
        // to the REST of this run only — the session's model, thinking level and
        // transcript are never rewritten by a hook.
        //
        // `turn_context` is pi's `currentContext.messages`: while it is active every message
        // the loop adds to the transcript is appended to it too, so it tracks the run instead
        // of freezing at its install-time snapshot. `turn_context_epoch` is the history epoch
        // the replacement was built from — a committed compaction makes it stale (blocker 2).
        let mut turn_context: Option<Vec<Message>> = None;
        let mut turn_context_epoch: u64 = self.history_epoch();
        let mut run_model: Option<String> = None;
        let mut run_thinking: Option<epsilon_contracts::ThinkingLevel> = None;

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
                    // Plan 002: drained-but-not-yet-injected messages go back to the
                    // front of their queue so an abort never silently eats user input.
                    self.requeue_pending(&mut pending);
                    break 'outer;
                }

                if turn_index >= self.limits.max_turns {
                    reason = reason.merge_sticky(TurnEndReason::BudgetExhausted);
                    self.requeue_pending(&mut pending);
                    break 'outer;
                }

                // Drain point: inject queued messages before the next model call.
                //
                // `?` since F07: a queued message the store refused is a message the model
                // would not see on the next request even though the loop believes it recorded
                // it. Stopping here is the only answer that does not send a divergent view.
                for m in pending.drain(..) {
                    self.push_message(&m)?;
                    Agent::append_to_turn_context(&mut turn_context, &m);
                    produced.push(m);
                }

                // ---- compaction pressure check (ADR-007 trigger 1) ----------------
                // One pressure-triggered attempt per run after a failure: the
                // summarizer just failed once (provider error, empty answer, append
                // failure) and every retry is a paid provider call, so the next
                // attempt belongs to the next run.
                if !self.compact_suppressed.load(Ordering::SeqCst) && self.compaction_due() {
                    match self.compact_now(&mut produced).await? {
                        CompactionOutcome::Committed => {}
                        CompactionOutcome::Failed(why) => {
                            self.compact_suppressed.store(true, Ordering::SeqCst);
                            self.emit(AgentEvent::Notice {
                                text: format!("context compaction skipped: {why}"),
                            });
                        }
                        CompactionOutcome::Cancelled => {
                            reason = reason.merge_sticky(TurnEndReason::Aborted);
                            break 'outer;
                        }
                    }
                }

                // CORE-02 (blocker 2): a replacement is only valid while the history it was
                // built from is still the run's base. A committed compaction just replaced
                // that history (and restored it from the summary), so the run-scoped
                // replacement is stale: requests built from it would stop reading the store's
                // shadowed model view (the M86/ADR-007 invariant) and an overflow retry would
                // re-send the very request the compaction fixed. Drop it and rebuild from the
                // current source; the per-run model/thinking overrides are snapshot state, not
                // transcript state, and survive (pi keeps `config.model`/`reasoning` too).
                if turn_context.is_some() && self.history_epoch() != turn_context_epoch {
                    turn_context = None;
                }

                self.emit(AgentEvent::TurnStart { turn_index });

                // ---- one provider turn --------------------------------------------
                let assistant = match self
                    .stream_one_turn(
                        &mut usage_total,
                        turn_context.as_deref(),
                        run_model.as_deref(),
                        run_thinking,
                    )
                    .await
                {
                    Ok(a) => a,
                    Err(e) => {
                        // Error path: same requeue discipline as abort/budget.
                        self.requeue_pending(&mut pending);
                        // **The run is billed for whatever it spent before it failed.**
                        //
                        // `usage_total` accumulates on every `Done` inside
                        // `stream_one_turn`, and `stream_one_turn` also updates
                        // `session_usage_by_model` live — so on this arm the two totals
                        // disagreed by exactly what the run had already cost, and RPC's
                        // `get_session_stats` publishes both. A client that adds up the
                        // per-model rows and compares them against the flat total got two
                        // different answers for one session, on the sessions where the bill
                        // actually mattered.
                        //
                        // Reachable only on a durable-write or poisoned-lock failure, which
                        // is the F07 class: the turn before this one was answered, delivered,
                        // and paid for.
                        if let Ok(mut session) = self.session_usage.lock() {
                            *session = session.combine(usage_total);
                        }
                        return Err(e);
                    }
                };
                let m = Message::Assistant(assistant.clone());
                produced.push(m);
                // pi pushes the assistant message into the run's context the moment the
                // stream finishes (`streamAssistantResponse` → `context.messages.push`), so a
                // replacement installed on an earlier turn gets its own output back — without
                // this the provider would receive the same frozen request forever.
                Agent::append_to_turn_context(
                    &mut turn_context,
                    &Message::Assistant(assistant.clone()),
                );

                // ---- overflow recovery (ADR-007 trigger #2): compact once, retry ----
                if assistant.stop_reason == StopReason::Error
                    && assistant
                        .error_message
                        .as_deref()
                        .is_some_and(looks_like_context_overflow)
                    && !self.overflow_retried.swap(true, Ordering::SeqCst)
                {
                    // Discard the failed attempt, shrink, and retry this turn. The replacement
                    // must drop it too, or runtime history and the run's context would
                    // disagree by one message. F07: a pop the store refused leaves the store
                    // *ahead* (still holding the oversized attempt), so the retry stops here
                    // instead of re-sending it — `?` is the whole point of the fallible pop.
                    self.pop_last_message()?;
                    produced.pop();
                    if let Some(ctx) = turn_context.as_mut() {
                        ctx.pop();
                    }
                    match self.compact_now(&mut produced).await? {
                        CompactionOutcome::Committed => continue,
                        // Recovery could not summarize: retrying the same oversized
                        // request would just fail again. Report the real reason.
                        CompactionOutcome::Failed(why) => {
                            self.emit(AgentEvent::Notice {
                                text: format!("context overflow recovery failed: {why}"),
                            });
                            reason = reason.merge_sticky(TurnEndReason::Error(why));
                            break 'outer;
                        }
                        CompactionOutcome::Cancelled => {
                            reason = reason.merge_sticky(TurnEndReason::Aborted);
                            break 'outer;
                        }
                    }
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
                // Kept for the empty-turn guard below: `assistant` is moved into the TurnEnd
                // event further down.
                let assistant_text = assistant.text();
                let mut results: Vec<epsilon_contracts::message::ToolResultMessage> = Vec::new();
                let mut stuck_at: Option<usize> = None;
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

                // ---- stuck-run guard (port plan item 7) --------------------------------
                // Each result is folded into the progress guard; when the model has received the
                // same thing several times in a row the guidance rides along with the result it
                // is about, which is the cheapest place to reach the model.
                if let Ok(mut guard) = self.progress.lock() {
                    let text = |r: &epsilon_contracts::message::ToolResultMessage| {
                        r.content
                            .iter()
                            .filter_map(|c| match c {
                                epsilon_contracts::content::ContentBlock::Text { text } => {
                                    Some(text.as_str())
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    for r in results.iter_mut() {
                        let repeats = match guard.observe(r.is_error, &text(r)) {
                            crate::guards::Progress::New => continue,
                            crate::guards::Progress::Repeated { repeats } => repeats,
                        };
                        let threshold = self.limits.no_progress_threshold;
                        if guard.stalled_at(threshold) {
                            let hard = threshold > 0 && repeats >= threshold.saturating_mul(2);
                            let note = crate::guards::stuck_guidance(repeats, hard);
                            r.content
                                .push(epsilon_contracts::content::ContentBlock::text(note));
                            stuck_at = Some(repeats);
                        }
                    }
                }

                // ---- verification evidence (port plan item 5) ---------------------------
                // Which files changed, and whether a check has passed since. The record is what
                // the stop-time nudge reads, and what a reader of the session can audit.
                for (call_id, tool_name, args) in assistant.tool_calls() {
                    let result = results.iter().find(|r| r.tool_call_id == call_id);
                    let delivered = result.map(|r| !r.is_error).unwrap_or(false);
                    // F05: "the tool call was delivered" is not "the command passed". A shell
                    // command that exits nonzero arrives as a successful call carrying a failed
                    // command, so a check is only evidence when it exited zero. Edits keep the
                    // plain test — a refused write changed nothing, which is what `delivered`
                    // already means for them.
                    let ok = match (tool_name, result) {
                        ("bash", Some(r)) => delivered && crate::verify::check_passed(&r.details),
                        _ => delivered,
                    };
                    evidence.observe(tool_name, args, ok, turn_index);
                }

                for r in &results {
                    // M229 / A11: **a tool's own provider call is this turn's spend too.** The
                    // `task` tool runs a subagent, which makes real billed requests on the
                    // parent's account; before this, that usage rode in on the tool result
                    // (`ToolResultMessage.usage`, persisted) and was aggregated nowhere — the
                    // provider stream below sums assistant messages only, so the turn total,
                    // `session_usage()` and the per-model cost map each under-reported by
                    // exactly the child's cost.
                    //
                    // Folded here rather than at the tool boundary because this is the point where
                    // the turn's own total is live, and a fold anywhere else would leave the
                    // transcript, the header and the cost panel disagreeing about the same tokens.
                    if let Some(u) = r.usage {
                        usage_total = usage_total.combine(u);
                        // Per-model only when the tool says which model it ran on: a bill with no
                        // model cannot be priced without inventing a rate, which is M141's defect.
                        // It still counts in the total above, so an unattributed bill under-reports
                        // *where* it went, never *whether* it was spent.
                        if let Some(model) = r.details.get("model").and_then(|v| v.as_str())
                            && let Ok(mut by_model) = self.session_usage_by_model.lock()
                        {
                            let row = by_model.entry(model.to_string()).or_default();
                            *row = row.combine(u);
                        }
                    }
                    let m = Message::ToolResult(r.clone());
                    // pi emits full message lifecycle for tool results too — frontends
                    // render from these events, not by scraping internals.
                    self.emit(AgentEvent::MessageStart {
                        message: Box::new(m.clone()),
                    });
                    // F07: stop rather than continue with a result the store does not have.
                    // The results were already computed, but the next request is built from the
                    // store's projection — carrying on would announce a tool outcome to the user
                    // that the model's transcript does not contain.
                    self.push_message(&m)?;
                    // ...and the run's context carries them like any transcript message (pi
                    // appends each result to `currentContext.messages` as it completes), so a
                    // replacement installed earlier keeps tracking the turn's output.
                    Agent::append_to_turn_context(&mut turn_context, &m);
                    produced.push(m);
                    self.emit(AgentEvent::MessageEnd {
                        message: Box::new(Message::ToolResult(r.clone())),
                    });
                }

                // CORE-02: the hook context carries the same messages the transcript just
                // recorded, so a replacement context can be built from the turn's output.
                let turn_message = Message::Assistant(assistant);
                let tool_messages: Vec<Message> =
                    results.iter().cloned().map(Message::ToolResult).collect();
                self.emit(AgentEvent::TurnEnd {
                    turn_index,
                    message: Box::new(turn_message.clone()),
                    tool_results: results,
                });
                turn_index += 1;

                // ---- turn hooks (CORE-02; pi `prepareNextTurn` / `shouldStopAfterTurn`) ----
                // Order follows pi `agent-loop.ts:224-257` exactly: `prepare_next_turn` runs
                // right after `turn_end`, its snapshot replaces the run's context/model/
                // thinking, and `should_stop_after_turn` — evaluated on the REPLACED context,
                // as pi does — runs BEFORE any queue polling. A true stop ends the run here:
                // no steering drain, no follow-up drain (those wait for the next run), no
                // stuck-run notice, and `agent_end` is emitted by `run()` as for any exit.
                // Read-only: neither hook can see the store lock or write history (ADR-007).
                //
                // The context a hook sees is the run's replacement when it holds one, else ONE
                // shared read with the request builder (`try_context_view`). No replacement and
                // no readable view (poisoned lock, failed model view) means the seams are
                // skipped for this turn: a hook must not be handed a view the request would
                // refuse, and the request below raises that failure as its own `RunError`.
                let hooks = self.turn_hooks();
                let owned_view: Option<Vec<Message>> = match turn_context {
                    Some(_) => None,
                    None => self.try_context_view().ok(),
                };
                let prepared = match turn_context.as_deref().or(owned_view.as_deref()) {
                    Some(seen) => {
                        contain_hook(
                            hooks.prepare_next_turn(&epsilon_contracts::turn::NextTurnContext {
                                message: &turn_message,
                                tool_results: &tool_messages,
                                context: seen,
                                new_messages: &produced,
                            }),
                            None,
                        )
                        .await
                    }
                    None => None,
                };
                if let Some(update) = prepared {
                    if let Some(ctx) = update.context {
                        turn_context = Some(ctx);
                        // The replacement is derived from the history as it is NOW; a later
                        // wholesale replacement of that history makes it stale (blocker 2).
                        turn_context_epoch = self.history_epoch();
                    }
                    if let Some(model) = update.model {
                        run_model = Some(model.id);
                    }
                    if update.thinking.is_some() {
                        run_thinking = update.thinking;
                    }
                }
                // pi evaluates the stop decision on the REPLACED context (`agent-loop.ts:248-253`
                // passes `currentContext`, which the snapshot above may have swapped).
                let stop = match turn_context.as_deref().or(owned_view.as_deref()) {
                    Some(seen) => {
                        contain_hook(
                            hooks.should_stop_after_turn(
                                &epsilon_contracts::turn::NextTurnContext {
                                    message: &turn_message,
                                    tool_results: &tool_messages,
                                    context: seen,
                                    new_messages: &produced,
                                },
                            ),
                            false,
                        )
                        .await
                    }
                    None => false,
                };
                if stop {
                    break 'outer;
                }
                if let Some(repeats) = stuck_at {
                    self.emit(AgentEvent::Notice {
                        text: format!("stuck-run guard: {repeats} identical tool results in a row"),
                    });
                }

                pending = self.drain_steering();

                // ---- empty / degenerate turn (port plan item 7) ------------------------
                // A run may end on a turn that said nothing at all, or on one that repeated a
                // single line until it hit the token limit. Both look like success to the loop
                // and like a stuck agent to the user. One nudge, then a clean failure.
                if call_count == 0
                    && let Some(why) = crate::guards::empty_turn_reason(&assistant_text, 0)
                {
                    empty_turns += 1;
                    if empty_turns <= self.limits.empty_turn_retries {
                        let nudge = crate::guards::nudge_for(&why, empty_turns);
                        let m = Message::User(epsilon_contracts::message::UserMessage {
                            id: epsilon_contracts::new_id("msg"),
                            timestamp_ms: now_ms(),
                            content: vec![UserContent::Text { text: nudge }],
                        });
                        // The next iteration's drain point puts it in history and `produced`,
                        // so the nudge is part of the record rather than a whisper.
                        pending.push(m);
                        continue;
                    }
                    reason = reason.merge_sticky(TurnEndReason::Error(why.describe()));
                    break 'outer;
                }

                // ---- stop-time verification nudge (port plan item 5) --------------------
                // The run is about to end. If it changed files and nothing has passed since, say
                // so once (twice at most) — the model decides, and the record stays in the
                // transcript either way.
                if call_count == 0
                    && evidence.has_edits()
                    && verification_nudges < self.limits.verification_nudges
                    && let Some(nudge) = evidence.stop_nudge()
                {
                    verification_nudges += 1;
                    self.emit(AgentEvent::Notice {
                        text: format!("unverified edits: {}", evidence.unverified().join(", ")),
                    });
                    pending.push(Message::User(epsilon_contracts::message::UserMessage {
                        id: epsilon_contracts::new_id("msg"),
                        timestamp_ms: now_ms(),
                        content: vec![UserContent::Text { text: nudge }],
                    }));
                    continue;
                }
            }

            // The agent would stop here; honor queued follow-ups before exiting.
            let followups = self.drain_followups();
            if followups.is_empty() {
                break;
            }
            pending = followups;
        }

        // Fold this run's usage into the session total BEFORE handing it back, so there is
        // one commit point rather than two places that could each be forgotten. A caller
        // that reads `session_usage()` immediately after `run()` returns therefore sees
        // the run it just made.
        if let Ok(mut session) = self.session_usage.lock() {
            *session = session.combine(usage_total);
        }

        Ok(RunOutcome {
            produced,
            reason,
            usage: usage_total,
        })
    }

    /// Provider usage summed over every run this agent has served.
    ///
    /// Read by pi's `get_session_stats` and by anything else that needs the session's
    /// spend without owning a `UiState`. Deliberately independent of the TUI's own
    /// accumulator: a fresh agent that has run one turn reports it, and an agent restored
    /// by `/resume` reports only what ran in this process — which is why the TUI keeps its
    /// hydration path and this is not a replacement for it.
    pub fn session_usage(&self) -> Usage {
        self.session_usage.lock().map(|u| *u).unwrap_or_default()
    }

    /// [`Agent::session_usage`] split by the model that produced each response.
    ///
    /// A clone rather than a borrow because the lock is a `Mutex` and the caller prices
    /// the rows; at one call per RPC request the copy is not worth a guard's lifetime.
    pub fn session_usage_by_model(&self) -> std::collections::BTreeMap<String, Usage> {
        self.session_usage_by_model
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    fn drain_steering(&self) -> Vec<Message> {
        let mut out = self
            .steering
            .lock()
            .map(|mut q| q.drain())
            .unwrap_or_default();
        // M53: a background child's completion notice rides the same drain
        // point as user interjections, so a finishing subagent reaches the
        // model at the next turn — or at the next run's start when the session
        // is idle (run start prefills from this same queue).
        if let Ok(mut rx) = self.task_notices_rx.lock() {
            while let Ok(m) = rx.try_recv() {
                // The model is not the only reader: a notice the user cannot
                // see is a notice the user cannot trust, so it enters the
                // transcript with the same event a normal message uses.
                self.emit(AgentEvent::MessageEnd {
                    message: Box::new(m.clone()),
                });
                out.push(m);
            }
        }
        out
    }

    /// Drain ALL queued follow-up messages, ignoring queue mode.
    ///
    /// The follow-up counterpart to [`Agent::drain_steering_all`], and it exists because
    /// pi's `clear_queue` returns **both** queues (`rpc-types.ts:59`: `data: { steering,
    /// followUp }`). Without this, a client asking to clear the queue would silently leave
    /// every follow-up queued — the messages that run *after* the agent would otherwise stop
    /// — and the reply would claim a clear that did not happen. That is the same
    /// acknowledge-without-acting failure the TUI hit when it restored steering to the
    /// editor but had nowhere to put follow-ups.
    pub fn drain_followups_all(&self) -> Vec<Message> {
        self.followups
            .lock()
            .map(|mut q| q.drain_all())
            .unwrap_or_default()
    }

    /// Number of queued follow-up messages, for a queue-depth reply.
    pub fn followup_queue_len(&self) -> usize {
        self.followups.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// M30: drain ALL queued steering messages on abort (ignoring queue mode)
    /// so the TUI can restore them to the editor.
    pub fn drain_steering_all(&self) -> Vec<Message> {
        self.steering
            .lock()
            .map(|mut q| q.drain_all())
            .unwrap_or_default()
    }

    /// Plan 002: on abnormal exits (abort/budget/error), put drained-but-uninjected
    /// messages back where they came from. `pending` is steering-sourced inside the
    /// inner loop (follow-ups only enter as fresh `pending` at the outer boundary,
    /// where an empty follow-up batch simply breaks), so requeueing to the steering
    /// queue preserves delivery order and loses nothing.
    fn requeue_pending(&self, pending: &mut Vec<Message>) {
        if pending.is_empty() {
            return;
        }
        if let Ok(mut q) = self.steering.lock() {
            for m in pending.drain(..).rev() {
                q.restore(vec![m]);
            }
        } else {
            pending.clear();
        }
    }

    fn drain_followups(&self) -> Vec<Message> {
        self.followups
            .lock()
            .map(|mut q| q.drain())
            .unwrap_or_default()
    }

    /// Pressure trigger (ADR-007 #1): estimated context exceeds the live budget.
    ///
    /// Pi parity (`shouldCompact`): the budget is window-relative —
    /// `context_window - reserve` — whenever the active model's window is known,
    /// gated on the `enabled` switch. Unknown windows fall back to the fixed
    /// threshold, preserving the pre-M34 behavior for custom/faux models.
    fn compaction_due(&self) -> bool {
        if !self.limits.compaction_enabled {
            return false;
        }
        let tokens = self.estimated_context_tokens();
        let window = self.model_window_tokens.lock().ok().and_then(|slot| *slot);
        let limit = window
            .map(|w| w.saturating_sub(self.limits.compaction_reserve_tokens))
            .unwrap_or(self.limits.compaction_threshold_tokens);
        tokens > limit
    }

    /// Best estimate of how many tokens the next request will carry.
    ///
    /// Measured where a measurement exists: the provider reports the true prompt
    /// size on every response, so the estimate is that number plus a heuristic
    /// over whatever has been appended since (`compact::anchored_tokens`). Before
    /// this, the trigger was a `chars / 4` guess over the whole transcript, which
    /// over-counts prose and under-counts code and CJK by the same factor —
    /// compacting early wastes a summary call and a cache miss, compacting late
    /// overflows the window.
    ///
    /// Public because frontends show it: the footer's context percentage should
    /// agree with the number the loop actually decides on.
    pub fn estimated_context_tokens(&self) -> usize {
        let anchor = self.usage_anchor.lock().ok().and_then(|slot| *slot);
        // Borrowed, not `&self.history()`. This is called once per drawn frame
        // from the TUI's event loop — the same thread that must read the
        // keyboard — and a deep clone of the whole transcript on that thread
        // scales with session length. Measured at 13µs for 20 messages and
        // 144µs for 200 (see `crates/epsilon-core/tests/context_mirror_cost.rs`),
        // i.e. 10.8x for a 10x larger transcript: the signature of a whole-history
        // scan rather than the anchored walk this intends to be. The tripwire in
        // that test fails until the clone is gone.
        let fixed = self.fixed_prefix_tokens();
        self.with_history(|h| crate::compact::anchored_tokens(h, anchor, fixed))
    }

    /// Tokens every request carries that are **not** in the transcript: the system
    /// prompt and the tool schemas.
    ///
    /// Needed only by the no-anchor fallback in [`Self::estimated_context_tokens`],
    /// where nothing else has counted them — the anchored arm uses the provider's own
    /// `prompt_tokens`, which already includes them. See
    /// [`crate::compact::anchored_tokens`] for why adding them twice would be worse
    /// than the omission.
    ///
    /// Measured from the same inputs the real request is built from, so it cannot
    /// drift from what is actually sent: the descriptors are serialized exactly as a
    /// provider receives them, and the system prompt uses the same `prompt::build`
    /// path the turn takes.
    fn fixed_prefix_tokens(&self) -> usize {
        let tools = self.tools.descriptors();
        let schema_tokens = tools
            .iter()
            .map(|d| {
                let name = crate::compact::estimate_tokens(&d.name);
                let description = crate::compact::estimate_tokens(&d.description);
                let schema = d
                    .parameters_schema
                    .to_string()
                    .len()
                    .div_ceil(4)
                    .saturating_add(8);
                name + description + schema
            })
            .sum::<usize>();
        // The agent contract is the floor: it is what `prompt::build` emits when no
        // overlay replaces it, and it is ~6.5 KB, so omitting it understated the
        // prefix by more than everything else here.
        let contract = crate::compact::estimate_tokens(crate::prompt::AGENT_CONTRACT);
        contract + schema_tokens
    }

    /// Record what the provider billed for the request that covered `covered`
    /// messages (called after a completed turn; cleared when history is rewritten).
    fn set_usage_anchor(&self, prompt_tokens: usize, covered: usize) {
        if let Ok(mut slot) = self.usage_anchor.lock() {
            *slot = Some(crate::compact::UsageAnchor {
                prompt_tokens,
                covered,
            });
        }
    }

    /// The cache-miss attribution report (`crate::cache_stats`), one line per fact.
    ///
    /// Public because it is a user-facing diagnostic: the footer's hit rate says *whether* the
    /// prompt cache is paying off, and this says *why* when it is not.
    pub fn cache_report(&self) -> Vec<String> {
        self.cache
            .lock()
            .map(|stats| stats.report())
            .unwrap_or_else(|_| vec!["cache: unavailable".to_string()])
    }

    /// How many consecutive tool results have been identical, when that is more than one
    /// (`crate::guards::ProgressGuard` — the stuck-run brake reads this before each call).
    pub fn progress_streak(&self) -> Option<usize> {
        let guard = self.progress.lock().ok()?;
        let streak = guard.streak();
        (streak > 1).then_some(streak)
    }

    /// Drop the anchor: the history it describes no longer exists.
    /// Whether the context figure is a provider measurement or a local estimate.
    ///
    /// The same number either way, and the difference is exactly what a reader cannot
    /// infer from it. `anchored_tokens` prefers provider truth for the prefix and
    /// estimates only the tail; when no anchor survives it estimates everything, from a
    /// 4-chars-per-token heuristic that over-counts prose and under-counts code. The
    /// result is still the best available figure — but on turn one, after every
    /// compaction, and after `/resume` it is a *guess*, and the header should say so
    /// rather than presenting a guess and a measurement in the same typeface.
    ///
    /// Hermes ships this on every status payload for the same reason
    /// (`context_breakdown.py:94-115`), and DSH is blunter: its projection doc calls the
    /// occupancy figure "a user-facing reference, not a billing or gating input" — which
    /// is only useful to a user if the figure says how it was made.
    pub fn context_is_estimated(&self) -> bool {
        let anchor = self.usage_anchor.lock().ok().and_then(|slot| *slot);
        // The same guard the estimator applies, kept in step deliberately: if the
        // anchor cannot describe this history, the number is an estimate. Deriving it
        // twice from one rule is better than a flag that can disagree with the value.
        let Some(a) = anchor else { return true };
        let len = self.history.lock().map(|h| h.len()).unwrap_or(0);
        a.prompt_tokens == 0 || a.covered > len
    }

    fn clear_usage_anchor(&self) {
        if let Ok(mut slot) = self.usage_anchor.lock() {
            *slot = None;
        }
    }

    /// Bumped by every wholesale replacement of runtime history (`compact_now` on commit,
    /// `restore_history`). A run's `prepare_next_turn` replacement is only valid while the
    /// history it was derived from is still the run's base — see the field docs.
    fn bump_history_epoch(&self) {
        self.history_epoch
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Current epoch of runtime history (see [`Agent::bump_history_epoch`]).
    fn history_epoch(&self) -> u64 {
        self.history_epoch.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Run one full compaction pass: prune → summarize (isolated call, amend-mode when a
    /// prior summary exists) → append the durable entry (store attached) → replace runtime
    /// history → emit events.
    ///
    /// Commits only after a usable summary arrived, cancellation was checked at the
    /// last moment, and the durable entry (when a store is attached) appended
    /// successfully; every other path leaves history, store and provider view as
    /// they were and says why. Provider failures arrive as outcomes rather than
    /// `RunError`s because a failed summarizer is a compaction problem, not an
    /// agent-invariant violation.
    ///
    /// Event shape: `CompactionStarted` is emitted for every attempt, `CompactionFinished`
    /// only when the pass committed. `Failed` is surfaced to frontends by its `Notice`
    /// (which replaces any "compacting…" status); `Cancelled` ends the run immediately, so
    /// the frontends' run-end handling clears the status instead.
    pub async fn compact_now(
        self: &Arc<Self>,
        produced: &mut Vec<Message>,
    ) -> Result<CompactionOutcome, RunError> {
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
        // ...and then to the nearest *legal* boundary. A tail that begins with a
        // tool result has lost its call to the summary; providers reject that
        // shape, so the turn would fail after the summary was already paid for.
        let split = compact::legal_cut_point(&pruned_hist, split);
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

        // Ledger: everything the previous summary recorded, plus the files this
        // pass is about to summarize away. Facts from the tool calls themselves,
        // so the model keeps knowing what it already read and changed.
        let mut ledger = prior
            .as_deref()
            .map(compact::FileLedger::from_summary)
            .unwrap_or_default();
        ledger.merge(&compact::FileLedger::scan(middle));

        // Isolated summarization call (fresh request; no tools).
        let sctx = compact::summary_context(middle, prior.as_deref(), &ledger);
        let model_id = self
            .model_id
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "faux-1".into());
        let model = epsilon_llm::ModelRef::new(model_id);
        // Summarization is a real provider call (isolated on the aux stream, but still
        // billable and still mine to cancel): cancellation must reach it the way it
        // reaches the main turn, and a cancelled pass commits nothing (pi checks the
        // signal before the append — `agent-session.ts:1854-1881`). Biased select:
        // an already-cancelled token wins over a ready frame.
        let run_token = self.current_run_token();
        // Bound to a local before the select: `summarizer_stream()` returns a
        // temporary `Arc`, and the stream future borrows it (E0716 otherwise — the
        // same shape `stream_one_turn` uses for the main provider).
        let summarizer = self.summarizer_stream();
        let mut stream = tokio::select! {
            biased;
            _ = async {
                match run_token {
                    Some(ref t) => t.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            } => return Ok(CompactionOutcome::Cancelled),
            s = summarizer.stream(&model, sctx) => s,
        };
        let answer = loop {
            tokio::select! {
                biased;
                _ = async {
                    match run_token {
                        Some(ref t) => t.cancelled().await,
                        None => std::future::pending::<()>().await,
                    }
                } => return Ok(CompactionOutcome::Cancelled),
                ev = stream.next() => {
                    match ev {
                        Some(StreamEvent::Done { message }) => break message,
                        Some(_) => continue,
                        None => {
                            return Ok(CompactionOutcome::Failed(
                                "summarizer stream ended early".into(),
                            ));
                        }
                    }
                }
            }
        };
        // A failed provider turn is not a summary: pi rejects `stopReason === "error"`
        // before the append (`compaction.ts:670-685`). Accepting it would replace
        // useful context with failure prose — the opposite of what compaction is for.
        if answer.stop_reason.is_failure() {
            return Ok(CompactionOutcome::Failed(format!(
                "summarization turn failed: {}",
                answer
                    .error_message
                    .as_deref()
                    .unwrap_or("provider reported an error")
            )));
        }
        // No usable text, for one of two reasons — and they need different
        // words, because they need different responses from whoever reads the
        // log. An empty answer is refused rather than padded: the template
        // fallback used to guarantee "valid-looking" output, which meant a blank
        // summary could silently replace the context it failed to summarize. A
        // summary cut off by the output limit is refused for the harder reason:
        // it looks complete, so committing it would replace the real transcript
        // with a plausible-looking record of a shorter session.
        let Some(text) = compact::summary_text(&answer) else {
            let why = if answer.stop_reason == epsilon_contracts::message::StopReason::MaxTokens {
                "summarization hit the output limit and was cut off; a truncated summary \
                 is worse than none because it reads as complete"
            } else {
                "summarization returned no text"
            };
            return Ok(CompactionOutcome::Failed(why.into()));
        };
        let summary = compact::summary_with_ledger(&text, &ledger);
        // ...plus the user's own words from the range just summarized, verbatim: a paraphrase of
        // an instruction is an instruction followed approximately (Hermes micro-compaction).
        let summary =
            compact::summary_with_instructions(&summary, &compact::user_instruction_block(middle));

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

        let estimate_messages_before: usize = self.estimated_context_tokens();
        // Last cancellation check before anything durable or live changes: a token
        // cancelled while the summary streamed must not still rewrite the transcript.
        if run_token.as_ref().is_some_and(|t| t.is_cancelled()) {
            return Ok(CompactionOutcome::Cancelled);
        }
        // Persist the compaction entry so the durable mirror's MODEL VIEW shadows the
        // range — before runtime history is replaced. With a store attached, provider
        // requests read the store's projection, so committing only one of the two views
        // would leave them disagreeing (the F07 append-failure shape).
        if let Some(store) = self.store() {
            let mut st = store.lock().map_err(|_| store_lock_failure())?;
            if let Err(e) = st.append(EntryKind::Compaction {
                summary: summary.clone(),
                tokens_before: estimate_messages_before as u64,
                retained_tail: kept_tail.to_vec(),
                details: Some(serde_json::json!({ "pruned_tool_results": pruned_count })),
                // M145/F22: the summarizer's real bill, on the entry that incurred it. The
                // response has been in scope here all along and the usage was discarded, so
                // a genuine provider call was free in every number the user could see.
                //
                // One entry, not two. M144 appended a SEPARATE accounting entry after this
                // one and was reverted: that row became the file's last entry, and
                // `pop_last_message` pops the last entry to discard a failed turn, so a
                // rollback separated a summary from its cost (F23). pi puts the usage here
                // for the same reason (`session-manager.ts:68-80`).
                //
                // The model comes from the `ModelRef` the summarizer actually ran with, not
                // from the header at read time — Epsilon prices from the catalog when it
                // tallies, so a bill with no model cannot be priced at all.
                usage: Some(answer.usage),
                model_id: Some(model.id.clone()),
            }) {
                // F07: nothing has diverged yet — memory is untouched and the caller reports the
                // failure — but the store just refused a durable write, so a later request must
                // not be built from it on the assumption that writes work.
                self.durable_write_failed(format!("compaction append: {e}"));
                return Ok(CompactionOutcome::Failed(format!(
                    "compaction entry could not be appended: {e}"
                )));
            }
        }

        match self.history.lock() {
            Ok(mut h) => *h = new_history,
            Err(_) => {
                // Poisoned history: the durable entry was already appended. Drop it again
                // so the two views cannot disagree — with a store attached the provider
                // request reads the store's projection, and it must describe the same
                // transcript runtime history holds. F07: when that undo itself fails the
                // views DO disagree (the store keeps a compaction runtime history never
                // applied), so the failure is reported rather than discarded, exactly as
                // for an append that fails outright.
                if let Some(store) = self.store()
                    && let Ok(mut st) = store.lock()
                    && let Err(e) = st.pop_last()
                {
                    self.durable_write_failed(format!("compaction rollback pop: {e}"));
                }
                return Err(RunError::Internal("history poisoned".into()));
            }
        }
        // The anchor described the transcript that was just replaced; the next
        // request re-measures from scratch.
        self.clear_usage_anchor();
        // CORE-02: the history a mid-run `prepare_next_turn` replacement was built on
        // no longer exists. Bumping the epoch invalidates it, so the next request rebuilds
        // from the current source — the store's shadowed view when one is attached — and
        // an overflow retry cannot re-send the request the compaction just fixed.
        self.bump_history_epoch();
        produced.push(Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: now_ms(),
            content: vec![UserContent::Text {
                text: "[context compacted]".into(),
            }],
        }));
        // The same usage that went onto the durable entry above, handed to the live
        // UI as well. Without it the cost is only visible after `/resume`, so the
        // number rises when a user reopens a session they were just working in.
        self.emit(AgentEvent::CompactionFinished {
            usage: Some(answer.usage),
            model_id: Some(model.id.clone()),
        });
        Ok(CompactionOutcome::Committed)
    }

    /// Synthesize the in-band abort message when cancellation wins mid-stream.
    ///
    /// The folded-so-far text/thinking is preserved (it really streamed);
    /// tool-call fragments are dropped — they never executed, and keeping them
    /// would fabricate dangling calls. The caller treats this like any failed
    /// turn: `TurnEndReason::Aborted`, no tool batch, run ends.
    /// Terminal message for a run that dies mid-stream — abort, transport
    /// failure, or a stream that ends without a terminal frame (M62).
    ///
    /// Rule (mirrors `HttpEventStream::failure_message` on the wire side):
    /// whatever the user already watched arrive stays, the turn is flagged with
    /// the real stop reason, and tool calls whose arguments may be half-written
    /// are dropped. Throwing away visible output is worse than an ugly
    /// transcript — and a resumed session rebuilding context from the store
    /// needs the partial reasoning, not a hole where it used to be.
    fn failure_message(
        &self,
        partial: Option<&AssistantMessage>,
        stop: StopReason,
        detail: &str,
    ) -> AssistantMessage {
        use epsilon_contracts::content::ContentBlock;
        let model_id = self
            .model_id
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "faux-1".into());
        let (provider, content, usage) = match partial {
            Some(p) => (
                p.provider.clone(),
                p.content
                    .iter()
                    .filter(|b| match b {
                        ContentBlock::ToolCall { .. } => false,
                        ContentBlock::Text { text } => !text.is_empty(),
                        ContentBlock::Thinking { text, .. } => !text.is_empty(),
                        _ => true,
                    })
                    .cloned()
                    .collect(),
                p.usage,
            ),
            None => ("unknown".into(), Vec::new(), Usage::default()),
        };
        AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: now_ms(),
            provider,
            model_id,
            content,
            stop_reason: stop,
            error_message: Some(detail.to_string()),
            usage,
        }
    }

    /// [`Agent::failure_message`] for a user abort.
    fn aborted_message(&self, partial: Option<&AssistantMessage>) -> AssistantMessage {
        self.failure_message(partial, StopReason::Aborted, "aborted by user")
    }

    /// One provider turn, built from the run's current context.
    ///
    /// `turn_context` is a `prepare_next_turn` replacement when a hook installed one (it
    /// wins over every other source — pi replaces `currentContext` wholesale);
    /// `model_override` / `thinking_override` are that snapshot's per-run model and
    /// thinking level. All three are `None` for a run without turn hooks, which is the
    /// pre-CORE-02 behaviour exactly.
    async fn stream_one_turn(
        self: &Arc<Self>,
        usage_total: &mut Usage,
        turn_context: Option<&[Message]>,
        model_override: Option<&str>,
        thinking_override: Option<epsilon_contracts::ThinkingLevel>,
    ) -> Result<AssistantMessage, RunError> {
        // Provider-visible view: a hook replacement first, then the store's shadowed view
        // when attached, else runtime history. One shared read with the hook seam
        // (`try_context_view`), so a hook can never be handed a view this request would
        // refuse — and the two can never disagree about what the provider-bound context is.
        let from_replacement = turn_context.is_some();
        let history: Vec<Message> = match turn_context {
            Some(ctx) => ctx.to_vec(),
            None => self.try_context_view()?,
        };
        // How much of the history this request covers: the anchor recorded from
        // its usage describes exactly these messages, so the next pressure check
        // only has to estimate what is appended after them.
        //
        // A replacement is NOT runtime history: `covered` indexes the front of the
        // transcript (`crate::compact::anchored_tokens` slices `history[covered..]` of
        // runtime history), so a replacement's length would mis-index the anchor and
        // trigger needless paid compactions. Requests built from a replacement therefore
        // carry no anchor — dropped below, where the usage arrives.
        let covered = history.len();

        // ---- three-tier system prompt (ADR-008) ------------------------------------
        // The stable tier's fingerprint travels with this call: it is the bytes a provider's
        // prompt cache is expected to keep, so a change in it explains a cache miss
        // (`crate::cache_stats`) rather than leaving one unexplained.
        let tool_lines: Vec<prompt::ToolLine> = self
            .tools
            .descriptors()
            .iter()
            .map(|d| prompt::ToolLine {
                name: d.name.clone(),
                snippet: d.description.lines().next().unwrap_or("").to_string(),
            })
            .collect();
        // A `prepare_next_turn` model override wins for the rest of this run; otherwise the
        // session's model id (same lifecycle rule pi applies to `config.model`).
        let model_id = match model_override {
            Some(id) => id.to_string(),
            None => self
                .model_id
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "faux-1".into()),
        };
        // The identity overlays are discovered once per frontend, so a delegated child has to
        // be handed the same ones: a child that ran under the built-in contract while its
        // parent ran under the user's `SYSTEM.md` would be a different agent than the one the
        // task was delegated from.
        let overlay = self.system_overlay();
        let stable = prompt::Stable {
            tools: &tool_lines,
            custom_identity: overlay.custom.as_deref(),
            append: overlay.append.as_deref(),
        };
        let system_prompt = prompt::build(
            &stable,
            &prompt::ContextTier {
                agent_brief: self
                    .agent_brief
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_default(),
                instructions: {
                    let mut instructions = prompt::load_project_instructions(&self.workspace_root);
                    // Subdirectory hints (M73) ride in the same tier, after the root files:
                    // they are just as project-sourced, and the closest directory is the one
                    // that governs the file being edited.
                    instructions.extend(prompt::load_subdirectory_hints(
                        &self.workspace_root,
                        &self.touched_dirs(),
                    ));
                    instructions
                },
                skills_catalog: Some(crate::skills::catalog(&self.skill_list())),
                // The objective, when there is one — and only its words: the round counter and
                // the revision are what `get_goal` is for, and a number that moves every round
                // would rewrite the cached prefix on every round.
                goal: self
                    .goal()
                    .map(|view| format!("This session's goal: {}", view.goal.objective)),
            },
            &prompt::Volatile {
                cwd: &self.workspace_root,
                model_id: &model_id,
                today: today_string(),
            },
        );
        // The stable tier is what the provider is expected to keep cached; its fingerprint is
        // recorded with the call so an unexplained miss can be told from an explained one.
        // The same `Stable` value that built the prompt, so the two cannot drift apart.
        let stable_hash = prompt::stable_fingerprint(&stable);

        // CORE-02 request boundary (pi `agent-loop.ts:288-295`): transform the run's context
        // first, then convert to provider-visible messages. Both are pure reads — the store
        // is never written and runtime history is untouched (ADR-007) — and both are
        // contained: a hook that panics falls back to the seam's no-op default (identity for
        // the transform, the harness-visibility filter for the conversion), exactly as if no
        // host had registered one.
        let hooks = self.turn_hooks();
        let transformed = contain_hook(hooks.transform_context(&history), history.clone()).await;
        let messages = contain_hook(
            hooks.convert_to_llm(&transformed),
            transformed
                .iter()
                .filter(|m| m.is_llm_visible())
                .cloned()
                .collect::<Vec<Message>>(),
        )
        .await;
        let ctx = epsilon_llm::LlmContext {
            thinking_level: thinking_override.unwrap_or_else(|| self.thinking_level()),
            system_prompt,
            messages,
            tools: self.tools.descriptors(),
        };

        let model = epsilon_llm::ModelRef::new(model_id);
        let stream_fn = self
            .stream_fn
            .read()
            .map(|g| Arc::clone(&*g))
            .unwrap_or_else(|e| Arc::clone(&*e.into_inner()));
        // Abort reaches the provider stream, not just the tools (pi passes the
        // AbortSignal to `streamFunction`): cancellation wins the select below,
        // the provider stream is dropped, and the run ends in-band as Aborted.
        let token = self.current_run_token();
        // Both awaits below honor cancellation: stream creation (provider
        // connect/TTFB hangs) and event polling. Whichever select wins first
        // decides; `biased` prefers an already-cancelled token over a ready event.
        let mut stream = tokio::select! {
            biased;
            _ = async {
                match token {
                    Some(ref t) => t.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                // A terminal message reaches the transcript the same way a completed
                // turn does: runtime history, durable mirror, and a MessageEnd event.
                // Returning early used to leave the visible abort only in `produced`,
                // so a resumed conversation lost the interrupted answer (pi persists
                // assistant message-end events; `agent-session.ts:639-656`).
                let message = self.aborted_message(None);
                self.push_message(&Message::Assistant(message.clone()))?;
                self.emit(AgentEvent::MessageEnd {
                    message: Box::new(Message::Assistant(message.clone())),
                });
                return Ok(message);
            }
            s = stream_fn.stream(&model, ctx) => s,
        };
        let mut folder = StreamFolder::default();

        loop {
            tokio::select! {
                biased;
                _ = async {
                    match token {
                        Some(ref t) => t.cancelled().await,
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    // Same lifecycle as the connect-time abort above: the folded-so-far
                    // text is real output and must survive in the transcript.
                    let message = self.aborted_message(folder.partial());
                    self.push_message(&Message::Assistant(message.clone()))?;
                    self.emit(AgentEvent::MessageEnd {
                        message: Box::new(Message::Assistant(message.clone())),
                    });
                    return Ok(message);
                }
                ev = stream.next() => {
                    match ev {
                        Some(StreamEvent::Done { message }) => {
                            *usage_total = usage_total.combine(message.usage);
                            // Bill the response to the model that produced it, read from
                            // the message rather than from `self` — a response that lands
                            // after a `/model` switch belongs to the model that generated
                            // it, which is what makes a multi-model session price correctly.
                            if let Ok(mut by_model) = self.session_usage_by_model.lock() {
                                let row = by_model
                                    .entry(message.model_id.clone())
                                    .or_default();
                                *row = row.combine(message.usage);
                            }
                            // Cache bookkeeping (port plan item 4): what this call cost the
                            // prompt cache, and if it missed, why.
                            if let Ok(mut cache) = self.cache.lock() {
                                cache.observe(crate::cache_stats::CacheRequest {
                                    model: model.id.clone(),
                                    prefix_hash: stable_hash,
                                    usage: message.usage,
                                    at_ms: now_ms(),
                                });
                            }
                            // Anchor the next pressure check to what the provider
                            // actually billed for this request (pi usage anchor).
                            // Cache traffic counts: those tokens were in the prompt.
                            // A request built from a replacement drops the anchor instead
                            // (see `covered` above): its count indexes a transcript that is
                            // not the one `anchored_tokens` re-reads, so keeping it would
                            // mis-size the next pressure decision.
                            if from_replacement {
                                self.clear_usage_anchor();
                            } else {
                                self.set_usage_anchor(
                                    (message.usage.input
                                        + message.usage.cache_read
                                        + message.usage.cache_write)
                                        as usize,
                                    covered,
                                );
                            }
                            self.push_message(&Message::Assistant(message.clone()))?;
                            self.emit(AgentEvent::MessageEnd {
                                message: Box::new(Message::Assistant(message.clone())),
                            });
                            return Ok(message);
                        }
                        Some(other) => {
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
                        // Stream ended with no Done and no abort. If anything was
                        // decoded, that partial IS the turn's output: keep it and
                        // flag the failure (M62). With nothing decoded there is
                        // nothing to salvage, so this stays an internal error.
                        None => {
                            let partial = folder.partial().cloned();
                            let has_content = partial.as_ref().is_some_and(|p| {
                                !p.text().is_empty()
                                    || p.content.iter().any(|b| {
                                        matches!(
                                            b,
                                            epsilon_contracts::content::ContentBlock::Thinking { .. }
                                        )
                                    })
                            });
                            if !has_content {
                                return Err(RunError::Internal(
                                    "provider stream ended without Done".into(),
                                ));
                            }
                            let message = self.failure_message(
                                partial.as_ref(),
                                StopReason::Error,
                                "provider stream ended without a terminal frame — partial \\
                                 output kept",
                            );
                            self.push_message(&Message::Assistant(message.clone()))?;
                            self.emit(AgentEvent::MessageEnd {
                                message: Box::new(Message::Assistant(message.clone())),
                            });
                            return Ok(message);
                        }
                    }
                }
            }
        }
    }
}

/// One tracked delegated task (M53): the child stays alive between calls so a
/// later `task` request can resume it, and the tool layer can report status.
struct TaskEntry {
    id: String,
    description: String,
    subagent_type: String,
    /// Effective model for this child (M56) — see [`Agent::build_task_child`].
    model: Option<String>,
    state: epsilon_contracts::tool::TaskState,
    output: String,
    turns: u32,
    usage: Option<epsilon_contracts::message::Usage>,
    child: Arc<Agent>,
}

/// How many tasks the registry keeps before evicting the oldest finished one
/// (a long session must not accumulate live children forever).
const TASK_REGISTRY_CAP: usize = 16;

/// The goal record a `create` writes, validated.
///
/// A free function because two match arms build it — the empty slot and the slot held by a
/// *completed* goal — and two copies of "a goal needs an objective" is how one of them ends up
/// accepting an empty one.
fn new_goal(objective: &str, max_rounds: Option<u32>, now: u64) -> Result<GoalSnapshot, GoalError> {
    let objective = objective.trim();
    if objective.is_empty() {
        return Err(GoalError::Invalid("a goal needs an objective".into()));
    }
    let max_rounds = max_rounds.unwrap_or(DEFAULT_MAX_GOAL_ROUNDS);
    if max_rounds == 0 {
        return Err(GoalError::Invalid(
            "max_rounds must be at least 1; a goal that may never continue is a note".into(),
        ));
    }
    Ok(GoalSnapshot {
        id: epsilon_contracts::new_id("goal"),
        revision: 1,
        objective: objective.to_string(),
        status: GoalStatus::Active,
        max_rounds,
        rounds_started: 0,
        blocker: None,
        // A new goal has no completion rule beyond the caller's own judgement. Gates are
        // added by an explicit edit, never inferred from the objective's text: a harness
        // that guesses `cargo test` from the word "fix" would run a command the user never
        // asked for.
        gates: Vec::new(),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

/// The message a continuation round sends.
///
/// It names the objective rather than assuming the model remembers it, states the round and the
/// cap so the model can judge how much room is left, and says what ends the loop — because a
/// continuation the model cannot stop is a loop only a human can interrupt, and the human is not
/// watching.
///
/// The wording is deliberately plain and imperative, and it is the *only* thing a continuation
/// adds to the conversation: DSH's rule that a goal mutation injects no model context is kept,
/// so a round costs one message and nothing else.
fn goal_round_prompt(goal: &GoalSnapshot) -> String {
    format!(
        "Continuing the session goal (round {}/{}). Objective: {}\n\n\
         Work on it, then stop and report what changed and what is left. If the objective is \
         achieved, mark the goal complete with the `goal` tool; if you are blocked by something \
         you cannot change, mark it blocked and say what it is. The harness starts the next \
         round only if the goal is still active.",
        goal.rounds_started, goal.max_rounds, goal.objective
    )
}

impl Agent {
    /// Build the child agent for a `task` delegation: agent-type routing
    /// (def lookup, tool subset, model/thinking inheritance, skills, role
    /// brief) plus the depth stamp that stops nested fan-out.
    ///
    /// M56: returns the effective model alongside the child. Precedence is
    /// `def.model` (the operator's configuration) → `request_model` (the call's
    /// own choice) → inherited parent model, matching opencode's
    /// `next.model ?? parent` with the operator's config kept on top.
    fn build_task_child(
        &self,
        subagent_type: &str,
        request_model: Option<&str>,
    ) -> Result<(Arc<Agent>, Option<String>), String> {
        // Child agent shares the same provider/gate/workspace but has an
        // isolated transcript (no steering/follow-up carry-over). Caps are
        // modest so one delegated task cannot exhaust the parent's budget.
        let limits = LoopLimits {
            max_turns: 32,
            ..Default::default()
        };
        // M50: agent-type routing (pi `agents.ts` + opencode `Agent.get`
        // parity). `default` (or empty) means "no def matched": full registry,
        // inherited model, no extra prompt — exactly the old behavior. A named
        // type MUST resolve, or the failure names what's available instead of
        // silently running the wrong configuration.
        let agents = self.agent_list();
        let def = if subagent_type.trim().is_empty() || subagent_type == "default" {
            None
        } else {
            match agents.iter().find(|a| a.name == subagent_type) {
                Some(d) => Some(d.clone()),
                None => {
                    let mut names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
                    names.sort_unstable();
                    let available = if names.is_empty() {
                        "none — no agent defs discovered".to_string()
                    } else {
                        names.join(", ")
                    };
                    return Err(format!(
                        "unknown agent type: \"{subagent_type}\" is not a valid agent type. Available: {available}"
                    ));
                }
            }
        };
        // M50: depth cap (opencode `subagent_depth` parity, default 1). A child
        // at the cap runs with a registry whose `task` tool keeps working for
        // display but whose host refuses — nested fan-out fails honestly
        // instead of recursing the loop.
        const MAX_TASK_DEPTH: usize = 1;
        let depth = self.task_depth.lock().map(|s| *s).unwrap_or(0);
        if depth >= MAX_TASK_DEPTH {
            return Err(format!(
                "subagent depth limit reached ({MAX_TASK_DEPTH}). Increase the limit to allow nested subagents."
            ));
        }
        let stream_fn = self
            .stream_fn
            .read()
            .map(|g| std::sync::Arc::clone(&*g))
            .unwrap_or_else(|e| std::sync::Arc::clone(&*e.into_inner()));
        let gate: std::sync::Arc<dyn crate::gate::PermissionGate> = self
            .gate
            .child_gate()
            .unwrap_or_else(|| std::sync::Arc::clone(&self.gate));
        // Tool subset: a def WITH a tools list sees only those tools (pi
        // scout/planner/reviewer parity); no list (or no def) = full registry.
        let tools = match def.as_ref().and_then(|d| d.tools.as_ref()) {
            Some(names) => std::sync::Arc::new(self.tools.subset(names)),
            None => std::sync::Arc::clone(&self.tools),
        };
        let child = Agent::new(limits, stream_fn, tools, gate, self.workspace_root.clone());
        // Model: def override wins, then the call's request, else inherit (pi:
        // omitted model inherits the dispatching session's model + thinking
        // level). The resolved id is reported back so the tool result, the
        // registry and `/tasks` all name the model that actually ran.
        let model = if let Some(m) = def.as_ref().and_then(|d| d.model.as_ref()) {
            child.set_model_id(m.clone());
            Some(m.clone())
        } else if let Some(requested) = request_model {
            let resolved = Self::resolve_task_model(requested)?;
            child.set_model_id(resolved.clone());
            Some(resolved)
        } else if let Ok(m) = self.model_id.lock() {
            child.set_model_id(m.clone());
            Some(m.clone()).filter(|s| !s.trim().is_empty())
        } else {
            None
        };
        child.set_thinking_level(self.thinking_level());
        // **The parent's hooks, for the same reason the gate is derived and not shared.**
        //
        // `Agent::new` starts every agent on `NoopToolHooks`/`NoopTurnHooks`, and nothing here
        // ever replaced them — so an ADR-019 `before_tool_call` that exists to **vet or block** a
        // tool call was installed on the parent (`epsilon-runtime/src/lib.rs:1318`) and reached
        // no child at all. A hook that blocks `bash` blocked it for the top-level agent and let
        // every subagent run it, silently. That is the `AGENTS.md` §2.7 shape — a control the
        // operator installed that does not act on a real path — and it is the same class as the
        // M300 permission leak through a second door.
        //
        // **Inherited rather than derived, and that is the difference from the gate.** The gate
        // needed a fresh memory because a *session's remembered allow* is a decision about one
        // conversation. A hook is not a decision about a conversation: it is operator policy —
        // the extension's own veto — and policy that applied to the agent and not to the agents
        // it spawns is the defect, not the design. There is nothing in a hook to un-remember, so
        // the parent's `Arc` is handed over as-is. Both accessors clone an `Arc`, so this costs
        // two refcounts per delegated task.
        child.set_tool_hooks(self.tool_hooks());
        child.set_turn_hooks(self.turn_hooks());
        // Child discovers the same skill catalog (read-only share).
        child.set_skills(self.skill_list());
        // ...and the same identity overlays, for the same reason: the workspace's
        // `SYSTEM.md` is the contract the user wrote for this repository, not a property of
        // the top-level session.
        child.set_system_overlay(self.system_overlay());
        // Child inherits agent defs (so ITS error messages name the same
        // types) at one deeper depth.
        child.set_agents(self.agent_list());
        if let Ok(mut slot) = child.task_depth_slot() {
            *slot = depth + 1;
        }
        // System prompt: the def body (pi: agent file body past frontmatter)
        // becomes a CONTEXT-tier project instruction so the child knows its
        // role while the task stays the user message. No def (or an empty
        // body) = bare prompt, exactly the old behavior.
        if let Some(body) = def.as_ref().and_then(|d| {
            if d.system_prompt.trim().is_empty() {
                None
            } else {
                Some(d.system_prompt.clone())
            }
        }) {
            child.set_agent_brief(body);
        }
        Ok((child, model))
    }

    /// M58: give a child its own durable session file beside the parent's, so a
    /// `task_id` survives a restart (the child's file is a session: header with
    /// `parent_session`, a `task_meta` entry the rehydrator reads back).
    ///
    /// Returns the file path, or `None` when the parent has no store attached —
    /// children then live in memory only, exactly the pre-M58 behavior (tests
    /// and embedding callers keep working unchanged).
    fn attach_child_session(
        &self,
        child: &Arc<Agent>,
        task_id: &str,
        description: &str,
        subagent_type: &str,
        model: Option<&str>,
    ) -> Option<std::path::PathBuf> {
        use epsilon_contracts::session_entry::EntryKind;
        let parent = self.store()?;
        let (dir, stem, parent_id) = {
            let st = parent.lock().ok()?;
            let path = st.path().to_path_buf();
            (
                path.parent()?.to_path_buf(),
                path.file_stem()?.to_string_lossy().to_string(),
                st.header().map(|h| h.id.clone()),
            )
        };
        let file = dir.join(format!("{stem}-{task_id}.jsonl"));
        // **Both failures on this path are reported, not printed (M311).** These were an
        // `eprintln!` and a `let _ =`, which is the same defect twice: a window has no console,
        // so `eprintln!` reaches nobody, and a discarded `append` reaches nobody by definition.
        // The `RunParams::notices` sink that fixed sixteen `eprintln!` lines one layer up
        // (`epsilon-runtime`) does not exist here — but an `AgentEvent::Notice` does, and every
        // front end already renders those, so this needs no new plumbing.
        // `parent_id` is `None` for a parent with no header; the original `parent_id?` returned
        // early there, and clippy is right that it still reads better than an explicit `let else`
        // — the two failure paths below report *after* this point, so nothing is lost by it.
        let parent_id = parent_id?;
        let mut store = match SessionStore::open_child_in(&file, &self.workspace_root, &parent_id) {
            Ok(s) => s,
            Err(e) => {
                self.emit(AgentEvent::Notice {
                    text: format!(
                        "task '{task_id}': could not open its own session file ({}): {e} — this \
                         subagent runs in memory only and will not be resumable",
                        file.display()
                    ),
                });
                return None;
            }
        };
        if let Err(e) = store.append(EntryKind::Custom {
            custom_type: "task_meta".to_string(),
            data: Some(serde_json::json!({
                "task_id": task_id,
                "description": description,
                "subagent_type": subagent_type,
                "model": model,
            })),
        }) {
            // The file exists and is attached, but it carries no `task_meta` — so a later
            // `/tasks` resume would find a session it cannot rehydrate, silently.
            self.emit(AgentEvent::Notice {
                text: format!(
                    "task '{task_id}': its session file was created but the resume record could \
                     not be written ({e}) — this task will not be resumable after a restart"
                ),
            });
        }
        child.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
        Some(file)
    }

    /// M58: rebuild a child from its persisted session file, for a `task_id`
    /// that outlived the process that created it.
    ///
    /// `Ok(None)` means "no such child on disk" — the caller keeps its honest
    /// unknown-id error. A file that exists but cannot be read is an error, not
    /// a silent miss: the caller asked for a specific task.
    fn rehydrate_task(
        &self,
        id: &str,
    ) -> Result<Option<epsilon_contracts::tool::TaskSnapshot>, String> {
        use epsilon_contracts::session_entry::EntryKind;
        let Some(dir) = self
            .session_path()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        else {
            return Ok(None);
        };
        let file = std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(&format!("-{id}.jsonl")))
            });
        let Some(file) = file else {
            return Ok(None);
        };

        // Open + repair exactly like a live resume, so a crash mid-turn does
        // not make a child unresumable.
        let mut store =
            SessionStore::open(&file).map_err(|e| format!("open {}: {e}", file.display()))?;
        let _ = epsilon_session::repair::repair_on_open(&mut store);
        let entries = store
            .read_all()
            .map_err(|e| format!("read {}: {e}", file.display()))?;

        // Meta comes from the LAST `task_meta` entry (a resumed child rewrites
        // nothing, so the first is usually it too).
        let mut description = String::new();
        let mut subagent_type = "default".to_string();
        let mut model: Option<String> = None;
        for e in &entries {
            if let EntryKind::Custom {
                custom_type,
                data: Some(data),
            } = &e.kind
                && custom_type == "task_meta"
            {
                if let Some(d) = data.get("description").and_then(|v| v.as_str()) {
                    description = d.to_string();
                }
                if let Some(t) = data.get("subagent_type").and_then(|v| v.as_str()) {
                    subagent_type = t.to_string();
                }
                if let Some(m) = data.get("model").and_then(|v| v.as_str()) {
                    model = Some(m.to_string());
                }
            }
        }
        if description.is_empty() {
            description = id.to_string();
        }

        // Settled state from the last turn's ending, when the file has one.
        let ended = entries.iter().rev().find_map(|e| match &e.kind {
            EntryKind::TurnEnd { reason } => Some(reason.clone()),
            _ => None,
        });
        let failed = matches!(
            ended,
            Some(TurnEndReason::Error(_))
                | Some(TurnEndReason::Aborted)
                | Some(TurnEndReason::BudgetExhausted)
                | Some(TurnEndReason::Blocked)
        );

        let messages: Vec<Message> = store
            .model_view()
            .map_err(|e| format!("read {}: {e}", file.display()))?
            .into_iter()
            .map(|v| v.into_message())
            .collect();
        let turns = messages
            .iter()
            .filter(|m| matches!(m, Message::Assistant(_)))
            .count() as u32;
        let last_assistant = messages.iter().rev().find_map(|m| match m {
            Message::Assistant(a) => Some(a),
            _ => None,
        });
        let mut output = match ended {
            Some(TurnEndReason::Error(ref e)) => {
                format!("[restored from disk] run ended in error: {e}")
            }
            Some(TurnEndReason::Aborted) => "[restored from disk] run was aborted".to_string(),
            _ => last_assistant
                .map(|a| a.text())
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| format!("[restored from disk: {} message(s)]", messages.len())),
        };
        let usage = last_assistant.map(|a| a.usage);
        let state = if failed {
            epsilon_contracts::tool::TaskState::Failed
        } else {
            epsilon_contracts::tool::TaskState::Completed
        };

        // A stored model can have left the catalog since the child ran (models
        // retire): restoring the transcript still matters more than honoring the
        // old model, so fall back to inheritance and say so in the report.
        let (child, effective_model) = match self.build_task_child(&subagent_type, model.as_deref())
        {
            Ok(built) => built,
            Err(_retired) => {
                let (child, inherited) = self.build_task_child(&subagent_type, None)?;
                output.push_str(&format!(
                    "\n[restored on the inherited model: {}]",
                    inherited.as_deref().unwrap_or("(session default)")
                ));
                (child, inherited)
            }
        };
        child.restore_history(messages);
        child.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
        self.register_task_entry(TaskEntry {
            id: id.to_string(),
            description,
            subagent_type,
            model: effective_model,
            state,
            output,
            turns,
            usage,
            child,
        });
        Ok(self.task_snapshot(id))
    }

    /// Resolve a per-delegation model id against the catalog.
    ///
    /// Deliberately stricter than `resolve_model`'s partial rule: a delegated
    /// run must never silently swap models, so only exact, `provider/id`, and
    /// case-insensitive matches are accepted — a partial match is *offered* as
    /// a suggestion instead of used.
    fn resolve_task_model(raw: &str) -> Result<String, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("task: model must not be empty (omit it to inherit)".to_string());
        }
        match epsilon_llm::catalog::resolve_model(raw) {
            Some(r) if r.kind != epsilon_llm::catalog::ModelMatchKind::Partial => {
                Ok(r.info.id.to_string())
            }
            Some(r) => Err(format!(
                "task: model \"{raw}\" only partially matches \"{}\", and a delegated run never swaps models on a guess — pass the full id if that is the one you meant",
                r.info.id
            )),
            None => Err(format!(
                "task: unknown model \"{raw}\" — use a catalog id (`epsilon --list-models`, or the /model picker)"
            )),
        }
    }

    /// Collapse a finished child run into the envelope text + turn count.
    fn summarize_task_output(
        subagent_type: &str,
        description: &str,
        model: Option<&str>,
        produced: &[Message],
    ) -> (String, u32) {
        let turns = produced
            .iter()
            .filter(|m| matches!(m, epsilon_contracts::message::Message::Assistant(_)))
            .count() as u32;
        let mut parts: Vec<String> = Vec::new();
        for m in produced {
            match m {
                epsilon_contracts::message::Message::Assistant(a) => {
                    let t = a.text();
                    if !t.trim().is_empty() {
                        parts.push(t);
                    }
                }
                epsilon_contracts::message::Message::ToolResult(r) => {
                    let preview = r
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            epsilon_contracts::content::ContentBlock::Text { text } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !preview.trim().is_empty() {
                        parts.push(format!(
                            "[{}] {}",
                            r.tool_name,
                            preview.chars().take(4000).collect::<String>()
                        ));
                    }
                }
                _ => {}
            }
        }
        if parts.is_empty() {
            let on_model = model.map(|m| format!(" on {m}")).unwrap_or_default();
            parts.push(format!(
                "[subagent {subagent_type}{on_model}: {description} — no textual output]"
            ));
        }
        let text = parts.join("\n\n---\n\n");
        (text, turns)
    }

    /// The user message a delegated prompt travels in.
    fn task_user_message(prompt: &str) -> Message {
        use epsilon_contracts::content::UserContent;
        use epsilon_contracts::message::UserMessage;
        epsilon_contracts::message::Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: crate::now_ms(),
            content: vec![UserContent::Text {
                text: prompt.to_string(),
            }],
        })
    }

    /// The user-shaped notice the parent model sees when a background child
    /// finishes (delivered through the steering drain point).
    fn task_notice(
        id: &str,
        description: &str,
        state: &str,
        model: Option<&str>,
        body: &str,
    ) -> Message {
        // M56: the notice names the model the child ran on — the parent (and
        // the transcript) must be able to tell a scout on a cheap model from
        // the session's own model without asking.
        let on_model = model.map(|m| format!(" model=\"{m}\"")).unwrap_or_default();
        let text = format!(
            "<task id=\"{id}\" state=\"{state}\" description=\"{description}\"{on_model}>\n<task_result>\n{body}\n</task_result>\n</task>\n[background task {state} — resume it with task_id \"{id}\" if you need more]"
        );
        Self::task_user_message(&text)
    }

    /// Insert a new task, evicting the oldest finished entry at the cap.
    fn register_task_entry(&self, entry: TaskEntry) {
        if let Ok(mut v) = self.tasks.lock() {
            if v.len() >= TASK_REGISTRY_CAP
                && let Some(pos) = v
                    .iter()
                    .position(|e| e.state != epsilon_contracts::tool::TaskState::Running)
            {
                v.remove(pos);
            }
            v.push(entry);
        }
    }

    /// Update one tracked task in place; unknown ids are ignored.
    fn update_task_entry(&self, id: &str, f: impl FnOnce(&mut TaskEntry)) {
        if let Ok(mut v) = self.tasks.lock()
            && let Some(entry) = v.iter_mut().find(|e| e.id == id)
        {
            f(entry);
        }
    }

    /// Snapshot one tracked task by id.
    pub fn task_snapshot(&self, id: &str) -> Option<epsilon_contracts::tool::TaskSnapshot> {
        let v = self.tasks.lock().ok()?;
        v.iter().find(|e| e.id == id).map(Self::snapshot_of)
    }

    /// Every tracked task, in dispatch order (UI badges, tests).
    pub fn task_snapshots(&self) -> Vec<epsilon_contracts::tool::TaskSnapshot> {
        self.tasks
            .lock()
            .map(|v| v.iter().map(Self::snapshot_of).collect())
            .unwrap_or_default()
    }

    /// Ids still tracked, in dispatch order (honest unknown-id errors).
    pub fn task_ids(&self) -> Vec<String> {
        self.tasks
            .lock()
            .map(|v| v.iter().map(|e| e.id.clone()).collect())
            .unwrap_or_default()
    }

    fn snapshot_of(entry: &TaskEntry) -> epsilon_contracts::tool::TaskSnapshot {
        epsilon_contracts::tool::TaskSnapshot {
            id: entry.id.clone(),
            description: entry.description.clone(),
            subagent_type: entry.subagent_type.clone(),
            model: entry.model.clone(),
            state: entry.state.clone(),
            output: entry.output.clone(),
            turns: entry.turns,
            usage: entry.usage,
        }
    }

    /// M93/F18: claim a tracked task for a new run, in ONE lock hold.
    ///
    /// The check-then-set this replaced was two acquisitions: two racing resumes both read
    /// the settled snapshot, both saw a non-running state — before either had written
    /// anything — and both started a leg on the same child. Doing the find, the refusal and
    /// the state change under a single hold makes the claim atomic, so exactly one caller
    /// can win; the losers get the same honesty a status probe would.
    ///
    /// Errors describe the two states a caller can meet, and leave the entry untouched: an
    /// id that is no longer tracked (evicted, or never registered) and a child already
    /// running. Callers invoke this after their last fallible lookup, so a failure can never
    /// leave a phantom `Running` behind.
    fn claim_task_running(&self, id: &str) -> Result<(), String> {
        let mut v = self
            .tasks
            .lock()
            .map_err(|_| "task registry lock poisoned".to_string())?;
        let Some(entry) = v.iter_mut().find(|e| e.id == id) else {
            return Err(format!("task '{id}' is no longer tracked"));
        };
        if entry.state == epsilon_contracts::tool::TaskState::Running {
            return Err(format!(
                "task '{id}' is still running — poll it with task_id and no prompt, or wait for its completion notice"
            ));
        }
        entry.state = epsilon_contracts::tool::TaskState::Running;
        Ok(())
    }

    /// Run, resume, or report a delegated task (M53 wires `task`'s
    /// `background` and `task_id` parameters here).
    ///
    /// - fresh + foreground: build a child, await it, return its envelope;
    /// - fresh + background: register, spawn the run, return `Running` at once;
    /// - `task_id` + prompt: continue the same child (its history is intact);
    /// - `task_id` + empty prompt: status report, never a new run.
    pub async fn delegate_task(
        &self,
        request: epsilon_contracts::tool::TaskRequest,
    ) -> Result<epsilon_contracts::tool::TaskOutcome, String> {
        let prompt = request.prompt.trim().to_string();
        let request_model = request
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let resume = request
            .resume
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(id) = resume {
            // M58: a task from a previous run is not tracked in memory, but its
            // session file is on disk — rehydrate instead of failing.
            let snapshot = match self.task_snapshot(&id) {
                Some(snapshot) => snapshot,
                None => match self.rehydrate_task(&id)? {
                    Some(snapshot) => snapshot,
                    None => {
                        let known = self.task_ids();
                        return Err(if known.is_empty() {
                            format!(
                                "unknown task id '{id}' — no such task is tracked or on disk (ids come from a `task` call's result)"
                            )
                        } else {
                            format!("unknown task id '{id}' — tracked: {}", known.join(", "))
                        });
                    }
                },
            };
            if prompt.is_empty() {
                return Ok(epsilon_contracts::tool::TaskOutcome::Running(snapshot));
            }
            let child = self
                .tasks
                .lock()
                .ok()
                .and_then(|v| v.iter().find(|e| e.id == id).map(|e| Arc::clone(&e.child)))
                .ok_or_else(|| format!("task '{id}' is no longer tracked"))?;
            // M93/F18: the claim IS the refusal — one lock hold, after the last fallible
            // lookup, so a second resume is turned away while the first leg is in flight and
            // a failed lookup never leaves a phantom `Running` behind.
            self.claim_task_running(&id)?;
            if request.background {
                self.spawn_task_run(
                    id.clone(),
                    snapshot.description.clone(),
                    snapshot.subagent_type.clone(),
                    snapshot.model.clone(),
                    Arc::clone(&child),
                    prompt,
                );
                let now = self
                    .task_snapshot(&id)
                    .ok_or_else(|| "task entry vanished while resuming".to_string())?;
                return Ok(epsilon_contracts::tool::TaskOutcome::Running(now));
            }
            return self
                .await_task_run(
                    id,
                    snapshot.subagent_type.clone(),
                    snapshot.model.clone(),
                    prompt,
                    Arc::clone(&child),
                )
                .await;
        }
        let id = epsilon_contracts::new_id("task");
        let description = request.description.clone();
        let subagent_type = request.subagent_type.clone();
        let (child, model) =
            self.build_task_child(&request.subagent_type, request_model.as_deref())?;
        // M58: persist the child as its own session file (before it runs, so a
        // crash mid-turn still leaves a resumable task).
        self.attach_child_session(&child, &id, &description, &subagent_type, model.as_deref());
        self.register_task_entry(TaskEntry {
            id: id.clone(),
            description: description.clone(),
            subagent_type: subagent_type.clone(),
            model: model.clone(),
            state: epsilon_contracts::tool::TaskState::Running,
            output: String::new(),
            turns: 0,
            usage: None,
            child: Arc::clone(&child),
        });
        if request.background {
            self.spawn_task_run(
                id.clone(),
                description,
                subagent_type,
                model,
                Arc::clone(&child),
                prompt,
            );
            let now = self
                .task_snapshot(&id)
                .ok_or_else(|| "task entry vanished while starting".to_string())?;
            return Ok(epsilon_contracts::tool::TaskOutcome::Running(now));
        }
        self.await_task_run(id, subagent_type, model, prompt, child)
            .await
    }

    /// A child run that ended in an in-band provider failure (the never-fail
    /// contract delivers those as an assistant message with a failure stop
    /// reason, so `run()` still returns Ok).
    fn task_run_failure(outcome: &RunOutcome) -> Option<String> {
        // An armed abort returns before any message exists; the reason is the
        // only evidence. Check it first, then the in-band failure message.
        if outcome.reason == TurnEndReason::Aborted {
            return Some("subagent run aborted".to_string());
        }
        let produced = &outcome.produced;
        produced.iter().rev().find_map(|m| match m {
            Message::Assistant(a) if a.stop_reason.is_failure() => Some(
                if a.stop_reason == epsilon_contracts::message::StopReason::Aborted {
                    "subagent run aborted".to_string()
                } else {
                    a.error_message
                        .clone()
                        .unwrap_or_else(|| format!("subagent stopped: {:?}", a.stop_reason))
                },
            ),
            _ => None,
        })
    }

    /// Foreground path: await the child, record it, envelope the result.
    ///
    /// M93/F18: the claim — or the fresh register — writes `Running` before this body runs,
    /// so exactly like the detached body it must always end in a terminal record. The
    /// fallible work is contained the way `contain_panic` contains the detached body, so a
    /// panic here lands the entry `Failed` naming the panic instead of stranding the
    /// just-claimed entry `Running` forever.
    async fn await_task_run(
        &self,
        id: String,
        subagent_type: String,
        model: Option<String>,
        prompt: String,
        child: Arc<Agent>,
    ) -> Result<epsilon_contracts::tool::TaskOutcome, String> {
        let description = self
            .task_snapshot(&id)
            .map(|s| s.description)
            .unwrap_or_default();
        let panic_id = id.clone();
        contain_panic(
            async {
                match child.run(Self::task_user_message(&prompt)).await {
                    Ok(outcome) => {
                        // In-band failures are still failures: a subagent that died on
                        // a provider error must not be reported as a completed task.
                        if let Some(reason) = Self::task_run_failure(&outcome) {
                            let message = format!("subagent run failed: {reason}");
                            self.update_task_entry(&id, |entry| {
                                entry.state = epsilon_contracts::tool::TaskState::Failed;
                                entry.output = message.clone();
                            });
                            return Err(message);
                        }
                        let (text, turns) = Self::summarize_task_output(
                            &subagent_type,
                            &description,
                            model.as_deref(),
                            &outcome.produced,
                        );
                        self.update_task_entry(&id, |e| {
                            e.state = epsilon_contracts::tool::TaskState::Completed;
                            e.output = text.clone();
                            // Cumulative: a resumed child keeps counting its turns.
                            e.turns += turns;
                            e.usage = Some(outcome.usage);
                        });
                        Ok(epsilon_contracts::tool::TaskOutcome::Completed(
                            epsilon_contracts::tool::TaskHostResult {
                                text,
                                usage: Some(outcome.usage),
                                details: serde_json::json!({
                                    "task_id": id,
                                    "subagent_type": subagent_type,
                                    "model": model,
                                    "description": description,
                                    "state": "completed",
                                    "turns": turns,
                                    "produced": outcome.produced.len(),
                                    "reason": format!("{:?}", outcome.reason),
                                }),
                            },
                        ))
                    }
                    Err(e) => {
                        let message = format!("task host run failed: {e}");
                        self.update_task_entry(&id, |entry| {
                            entry.state = epsilon_contracts::tool::TaskState::Failed;
                            entry.output = message.clone();
                        });
                        Err(message)
                    }
                }
            },
            move |panic| {
                // The claim is now paired: the foreground leg lands the same truthful record
                // the detached body's panic arm writes.
                let message = format!("subagent run failed: the run panicked ({panic})");
                self.update_task_entry(&panic_id, |entry| {
                    entry.state = epsilon_contracts::tool::TaskState::Failed;
                    entry.output = message.clone();
                });
                Err(message)
            },
        )
        .await
    }

    /// Background path: detach the run, record its result, queue the notice.
    fn spawn_task_run(
        &self,
        id: String,
        description: String,
        subagent_type: String,
        model: Option<String>,
        child: Arc<Agent>,
        prompt: String,
    ) {
        let tasks = Arc::clone(&self.tasks);
        let notices = self.task_notices_tx.clone();
        let notice_id = id.clone();
        tokio::spawn(async move {
            // M93/F18: from the resume claim (and the fresh register) onward the entry says
            // `Running`, so this body must always end with a terminal record — including when
            // the run itself unwinds. The fallible work is contained exactly like a
            // panicking turn hook (`contain_panic`, the std-only `poll_fn` + `catch_unwind`
            // boundary), so a panic lands as a failed record naming the panic instead of
            // leaving the task `Running` forever.
            let (state, text, turns, usage) = contain_panic(
                async {
                    match child.run(Agent::task_user_message(&prompt)).await {
                        Ok(outcome) => match Agent::task_run_failure(&outcome) {
                            Some(reason) => (
                                "failed",
                                format!("subagent run failed: {reason}"),
                                0,
                                Some(outcome.usage),
                            ),
                            None => {
                                let (text, turns) = Agent::summarize_task_output(
                                    &subagent_type,
                                    &description,
                                    model.as_deref(),
                                    &outcome.produced,
                                );
                                ("completed", text, turns, Some(outcome.usage))
                            }
                        },
                        Err(e) => ("failed", format!("task host run failed: {e}"), 0, None),
                    }
                },
                |panic| {
                    (
                        "failed",
                        format!("subagent run failed: the run panicked ({panic})"),
                        0,
                        None,
                    )
                },
            )
            .await;
            // The registry must learn the outcome even when some unrelated panic poisoned the
            // lock: a `Running` entry whose run is over is exactly the lie this milestone
            // removes, so this write recovers the guard instead of silently skipping.
            let mut v = tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(entry) = v.iter_mut().find(|e| e.id == id) {
                entry.state = if state == "completed" {
                    epsilon_contracts::tool::TaskState::Completed
                } else {
                    epsilon_contracts::tool::TaskState::Failed
                };
                entry.output = text.clone();
                entry.turns += turns;
                entry.usage = usage;
            }
            let _ = notices.send(Agent::task_notice(
                &notice_id,
                &description,
                state,
                model.as_deref(),
                &text,
            ));
        });
    }
}

#[async_trait::async_trait]
impl epsilon_contracts::tool::TaskHost for Agent {
    async fn run_task(
        &self,
        request: epsilon_contracts::tool::TaskRequest,
    ) -> Result<epsilon_contracts::tool::TaskOutcome, String> {
        self.delegate_task(request).await
    }

    fn task_snapshot(&self, id: &str) -> Option<epsilon_contracts::tool::TaskSnapshot> {
        Agent::task_snapshot(self, id)
    }

    fn task_ids(&self) -> Vec<String> {
        Agent::task_ids(self)
    }
}

/// The `goal` tool's way back into session state, over the same seam `task` uses.
///
/// No `#[async_trait]`: both methods are synchronous by design (see the trait's own docs), and
/// making them `async` to match a neighbouring trait would be a shape borrowed from a different
/// problem.
impl epsilon_contracts::tool::GoalHost for Agent {
    fn current_goal(&self) -> Option<GoalView> {
        Agent::goal(self)
    }

    fn apply_goal(
        &self,
        expect: Option<GoalRef>,
        op: GoalOp,
    ) -> Result<Option<GoalView>, GoalError> {
        Agent::goal_apply(self, expect, op)
    }
}
