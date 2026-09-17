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

/// The kinds of repair this harness can perform.
///
/// pi classifies a contradiction into twelve machine-readable reasons; this harness can only
/// *produce* two shapes (a call without a result, a branch without its end marker), and a reason
/// type for a state that cannot occur is a claim nobody can test. The enum exists so callers
/// report the kind instead of a count — "3 tool results synthesized" and "the session ended
/// mid-run" are different facts about what the user lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairKind {
    /// A tool call whose result never landed: a synthetic error result was appended, marked as
    /// `interrupted`.
    DanglingToolCall,
    /// The branch ended mid-run: a `TurnEnd{Aborted}` marker was appended.
    MissingTurnEnd,
}

impl RepairKind {
    /// Stable string for logs, notices and `details` — never translated.
    pub fn as_str(self) -> &'static str {
        match self {
            RepairKind::DanglingToolCall => "dangling_tool_call",
            RepairKind::MissingTurnEnd => "missing_turn_end",
        }
    }

    /// One line a user can read, with the count for this kind.
    pub fn describe(self, count: usize) -> String {
        match self {
            RepairKind::DanglingToolCall => format!(
                "{count} tool call(s) had no result — synthesized an error result so the \
                 transcript stays provider-valid"
            ),
            RepairKind::MissingTurnEnd => {
                "the session ended mid-run — appended an aborted turn marker".to_string()
            }
        }
    }
}

/// What a repair pass found and did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepairReport {
    /// Dangling tool calls that received synthetic error results.
    pub repaired_tool_calls: usize,
    /// A synthetic turn-end marker was appended.
    pub appended_turn_end: bool,
}

impl RepairReport {
    /// The kinds this pass performed, with counts — empty when the session was intact.
    pub fn kinds(&self) -> Vec<(RepairKind, usize)> {
        let mut out = Vec::new();
        if self.repaired_tool_calls > 0 {
            out.push((RepairKind::DanglingToolCall, self.repaired_tool_calls));
        }
        if self.appended_turn_end {
            out.push((RepairKind::MissingTurnEnd, 1));
        }
        out
    }

    /// One line per kind, for a notice or a `/session` listing.
    pub fn summary(&self) -> Vec<String> {
        self.kinds()
            .into_iter()
            .map(|(kind, n)| kind.describe(n))
            .collect()
    }
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
            details: serde_json::json!({
                "interrupted": true,
                // Typed repair reason (M75): the entry says what was wrong, not just that
                // something was. `interrupted` stays for frontends that already read it.
                "repair": RepairKind::DanglingToolCall.as_str(),
            }),
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
