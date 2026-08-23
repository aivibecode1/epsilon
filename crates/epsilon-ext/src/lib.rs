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
pub const INVOKE_TIMEOUT_SECS: u64 = 10;
/// Crashes allowed before the host disables an extension.
pub const CRASH_BUDGET: u32 = 2;
