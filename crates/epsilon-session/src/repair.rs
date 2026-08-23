//! Crash recovery: repair dangling state after an interrupted run (DSH's repair helpers).
//!
//! Two failure shapes observed in the reference audit and handled here:
//! 1. a crash-truncated final JSONL line — dropped by [`SessionStore::reindex`] +
//!    [`SessionStore::truncate_to_index`];
//! 2. dangling tool calls — the run died after the model requested tools but before their
//!    results landed. The transcript would be provider-invalid (tool_use without results), so
//!    we synthesize error tool-result messages plus a `TurnEnd{Aborted}` marker.

use epsilon_contracts::agent_event::TurnEndReason;
use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::ids::new_id;
use epsilon_contracts::message::{Message, ToolResultMessage};
use epsilon_contracts::session_entry::{EntryKind, SessionEntry};

use crate::store::SessionStore;

/// What a repair pass found and did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RepairReport {
    /// Dangling tool calls that received synthetic error results.
    pub repaired_tool_calls: usize,
    /// A synthetic turn-end marker was appended.
    pub appended_turn_end: bool,
}

/// Scan the active branch; synthesize missing tool results / turn-end markers.
///
/// Idempotent: after one pass the branch ends with either complete results or a TurnEnd
/// entry, so a second pass is a no-op.
pub fn repair_on_open(store: &mut SessionStore) -> Result<RepairReport, crate::store::StoreError> {
    let mut report = RepairReport::default();

    let branch = store.branch_entries()?;
    if branch.is_empty() {
        return Ok(report);
    }

    // Find trailing unfinished state on the active lane.
    let mut pending_calls: Vec<(String, String)> = Vec::new(); // (tool_call_id, tool_name)
    for e in &branch {
        match &e.kind {
            EntryKind::Message { message, .. } => match message {
                Message::Assistant(a) => {
                    for (id, name, _) in a.tool_calls() {
                        pending_calls.push((id.to_string(), name.to_string()));
                    }
                }
                Message::ToolResult(t) => {
                    pending_calls.retain(|(id, _)| id != &t.tool_call_id);
                }
                _ => {}
            },
            EntryKind::TurnEnd { .. } => {
                pending_calls.clear();
            }
            _ => {}
        }
    }

    if pending_calls.is_empty() {
        // Also ensure the session does not end mid-run without any terminal marker when the
        // last assistant message ended with a failure stop reason.
        if let Some(SessionEntry {
            kind: EntryKind::Message { message, .. },
            ..
        }) = branch.last()
            && let Message::Assistant(a) = message
            && a.stop_reason.is_failure()
            && !ends_with_turn_end(&branch)
        {
            store.record_turn_end(TurnEndReason::Aborted)?;
            report.appended_turn_end = true;
        }
        return Ok(report);
    }

    for (tool_call_id, tool_name) in &pending_calls {
        let result = Message::ToolResult(ToolResultMessage {
            id: new_id("msg"),
            timestamp_ms: now_ms(),
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            content: vec![ContentBlock::text(format!(
                "Tool '{tool_name}' was interrupted before it could run (process restart)."
            ))],
            details: serde_json::json!({"interrupted": true}),
            usage: None,
            is_error: true,
        });
        store.append(EntryKind::Message {
            message: result,
            api_request_sha256: None,
        })?;
        report.repaired_tool_calls += 1;
    }

    store.record_turn_end(TurnEndReason::Aborted)?;
    report.appended_turn_end = true;
    Ok(report)
}

fn ends_with_turn_end(branch: &[SessionEntry]) -> bool {
    matches!(
        branch.last().map(|e| &e.kind),
        Some(EntryKind::TurnEnd { .. })
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
