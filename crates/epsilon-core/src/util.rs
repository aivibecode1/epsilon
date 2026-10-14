//! Helpers the loop shares: token and clock math, failure classifiers, the
//! run error, and the run-scoped cleanup guard.

use std::sync::atomic::Ordering;

use epsilon_contracts::QueueMode;
use epsilon_contracts::agent_event::TurnEndReason;
use epsilon_contracts::message::{Message, Usage};

use crate::Agent;

/// Compile-time marker: this build carries the real-provider path (M52).
///
/// `epsilon-cli` binds it under its own `real` feature, so a build that turns
/// on the network layer without turning on this crate's real code (which
/// would leave `Agent::try_hot_swap_to_real` stubbed while the CLI advertises
/// real providers) fails to compile instead of degrading silently.
#[cfg(feature = "real")]
pub const REAL_PATH_ENABLED: () = ();

/// Token estimate for one message (delegates to compact's heuristic).
pub(crate) fn estimate_one(m: &Message) -> usize {
    crate::compact::estimate_messages(std::slice::from_ref(m))
}

pub(crate) fn clamp_tail_tokens(v: usize) -> usize {
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
pub(crate) fn today_string() -> String {
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
/// Public because the TUI and the desktop both need it to word their own message: the
/// classifier is a fact about provider errors, and a front end that had to re-derive it would
/// get a second answer.
///
/// It matches what the retry policy calls [`epsilon_llm::FailureKind::Auth`]
/// (`epsilon-llm/src/retry.rs:36-41`), which is the half that decides a failure is never
/// retried. **The two must agree, and that is the whole rule.** When they disagree the retry
/// policy correctly gives up while both front ends say nothing — the 2026-10-04 report's
/// shape reproduced with a different status code.
///
/// The matching is deliberately as loose as the sibling's. A tighter rule that demanded a
/// delimiter around the status code was tried and rejected: `contains("403")` fires on
/// `{"input": 403}` and so does any delimiter-based version of it, because the two spellings
/// are structurally identical and only the context differs. Whichever way this is drawn, the
/// boundary is drawn from *provider error bodies* — this is handed `err`, the message a
/// provider sent back about a call that did not produce a reply, and never a successful
/// reply's own usage. A front end that wanted to call it on anything else would need to say
/// what "403" meant there first.
pub fn looks_like_auth_failure(msg: &str) -> bool {
    const NEEDLES: [&str; 8] = [
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "authentication",
        "invalid api key",
        "invalid_api_key",
        "authentication_error",
    ];
    let m = msg.to_ascii_lowercase();
    NEEDLES.iter().any(|needle| m.contains(needle))
}

/// Does this provider error mean "your prompt did not fit"?
///
/// The message we classify is the provider's body verbatim, so the classifier has
/// to know every spelling the providers actually use. The underscore forms are
/// the ones that mattered: Anthropic answers `request_too_large`, and an earlier
/// version of this list carried only the spaced spelling — so a real overflow
/// failed the run instead of compacting once and retrying, which is the whole
/// point of the branch this feeds.
pub(crate) fn looks_like_context_overflow(msg: &str) -> bool {
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

/// Run a turn-hook call to completion, containing a panic the way the provider seam's
/// `PanicGuard` contains a panicking stream (CORE-02).
///
/// The trait contract says a hook must not panic, but the loop cannot depend on a host
/// keeping it: an unwind out of `run_inner` would skip the cleanup that resets `running`,
/// after which every later run returns `RunError::AlreadyRunning`. On a panic the caller
/// gets `fallback` — the seam's no-op default (the input messages, `None`, `false`).
///
pub(crate) async fn contain_hook<T>(fut: impl std::future::Future<Output = T>, fallback: T) -> T {
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
/// The tool seam is the one boundary that runs *user* code — a native tool is arbitrary
/// Rust and an extension tool is arbitrary someone else's — and it was the only call site
/// without a containment. `pub(crate)` so `exec.rs` can reach it, which is where the
/// `Tool::execute` call lives.
pub(crate) async fn contain_panic<T>(
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
pub(crate) fn store_lock_failure() -> RunError {
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
pub(crate) struct RunGuard<'a> {
    pub(crate) agent: &'a Agent,
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.agent.run_token.lock() {
            *slot = None;
        }
        self.agent.running.store(false, Ordering::SeqCst);
    }
}
