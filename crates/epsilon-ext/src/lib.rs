//! Process-isolated extension host (ADR-009).
//!
//! Extensions are child processes speaking line-delimited JSON over stdio:
//!
//! ```text
//! child -> {"type":"hello","name":"git-safety","version":"0.1.0"}
//! host  -> {"type":"invoke","id":"i_1","call_id":"tc_x","tool":"pr_summary","args":{...}}
//! child -> {"type":"result","id":"i_1","is_error":false,"text":"...","details":{}}
//! ```
//!
//! Isolation properties: memory/CPU separation, per-call timeout (default 10s), crash budget
//! (2 crashes disable the extension for the session), manifest-declared tools validated at
//! load. Third-party code never runs inside the harness process.

mod host;
pub mod manifest;
mod tool;

pub use host::{ExtError, ExtHost};
pub use manifest::{ExtManifest, ExtToolSpec};
pub use tool::ExtTool;

/// Default per-invoke timeout.
///
/// Ten seconds is right for a *tool call*: an extension doing real work may need it, and a
/// tool call has no user watching a spinner.
pub const INVOKE_TIMEOUT_SECS: u64 = 10;

/// Default budget for waiting on an **interpose** decision.
///
/// Deliberately far shorter than [`INVOKE_TIMEOUT_SECS`], and the reason is arithmetic
/// rather than taste. An interpose happens before *every* tool call, so its cost is
/// multiplied by the number of calls in a turn — a 30-call turn would spend 300 s waiting on
/// a harness whose extension had stopped answering. A decision is also not work: it is a
/// yes or a no, and an extension that needs more than a second to form one is doing
/// something other than deciding.
///
/// A second is still generous for a child process on this machine, and the failure mode is
/// the good one: exceeding it means "no opinion", so the call proceeds.
///
/// This is a real defect found by a test that assumed otherwise. ADR-019 states the failure
/// policy correctly — a silent extension must not *block* a run — but "must not block" is not
/// "must not delay", and reusing the invoke budget quietly violated the second while
/// satisfying the first. See `a_live_extension_that_never_answers_does_not_stall_the_call`.
pub const INTERPOSE_TIMEOUT_SECS: u64 = 1;
/// Crashes allowed before the host disables an extension.
pub const CRASH_BUDGET: u32 = 2;
