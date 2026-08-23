//! Monotonic, sortable identifier generation.
//!
//! Format: `{prefix}_{unix_millis:x}_{counter:x}`. Sortable by creation time within a process,
//! unique across processes with overwhelming probability for our lifetimes (millisecond clock +
//! 32-bit counter). Deliberately dependency-free; swap for UUIDv7 later if cross-process
//! ordering ever matters (tracked as a post-v1 nicety, not a need).

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Generate a new prefixed identifier: `msg_19ab34..., tc_19ab35...`, etc.
pub fn new_id(prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{millis:x}{counter:x}")
}

/// Identifier of a user or assistant message.
pub type MessageId = String;
/// Identifier of a single tool invocation requested by the model.
pub type ToolCallId = String;
/// Identifier of a session (one JSONL file).
pub type SessionId = String;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_prefix_unique_and_monotonic() {
        let a = new_id("msg");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_id("msg");
        assert_ne!(a, b);
        assert!(a.starts_with("msg_") && b.starts_with("msg_"));
        assert!(a < b, "ids must sort by creation time");
    }
}
