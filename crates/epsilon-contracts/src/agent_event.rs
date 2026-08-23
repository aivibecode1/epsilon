//! Agent lifecycle events — the only output of the loop; every UI is a reducer over these.

use serde::{Deserialize, Serialize};

use crate::ids::ToolCallId;
use crate::message::{Message, ToolResultMessage};

/// Why a turn (assistant response + its tool batch) ended.
///
/// Ported from DSH's typed sticky turn endings: `MaxTokens` is *sticky* — once seen in a run,
/// later nominally-completed steps do not erase it (the context is still saturated).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEndReason {
    /// Normal completion: no tool calls remained and no failure occurred.
    Completed,
    /// Output token limit was hit somewhere in the run.
    MaxTokens,
    /// The permission gate blocked a tool call.
    Blocked,
    /// User cancelled the run.
    Aborted,
    /// A provider or harness error ended the run.
    Error(String),
    /// Iteration/step budget exhausted (Hermes' budget guard, typed).
    BudgetExhausted,
}

impl TurnEndReason {
    /// Sticky merge: the "worse" reason wins. Ported from DSH's invariant that
    /// `max-tokens` remains visible even if a subsequent step completes cleanly.
    pub fn merge_sticky(self, other: TurnEndReason) -> TurnEndReason {
        use TurnEndReason::*;
        match (self, other) {
            (MaxTokens, _) | (_, MaxTokens) => MaxTokens,
            (Error(e), Error(f)) => Error(format!("{e}; {f}")),
            (Error(e), _) => Error(e),
            (_, Error(e)) => Error(e),
            (Aborted, _) => Aborted,
            (_, Aborted) => Aborted,
            (Blocked, _) => Blocked,
            (_, Blocked) => Blocked,
            (BudgetExhausted, _) => BudgetExhausted,
            (_, BudgetExhausted) => BudgetExhausted,
            (Completed, Completed) => Completed,
        }
    }
}

/// A permission ask surfaced by the tool pipeline or doom-loop brake.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    /// Unique id for correlating the decision.
    pub request_id: String,
    /// Capability being exercised, e.g. `"bash"`, `"write"`, `"doom_loop"`.
    pub capability: String,
    /// Human-readable pattern/detail of the specific action (path, command head...).
    pub pattern: String,
    /// Free-form context shown to the user (command text, diff summary...).
    pub detail: serde_json::Value,
}

/// The user's answer to a [`PermissionRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Allow exactly this invocation.
    AllowOnce,
    /// Allow this invocation and remember for the session.
    AlwaysSession,
    /// Deny with optional corrective guidance fed back to the model.
    Deny,
}

/// Everything that happens during a run, in order.
///
/// Port of pi's AgentEvent taxonomy plus permission and compaction lifecycle events so
/// frontends can render gates without scraping internals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A run began.
    AgentStart,
    /// A run finished; carries every message produced by the run.
    AgentEnd {
        /// Messages created during this run (prompts excluded).
        messages: Vec<Message>,
    },
    /// One turn (assistant response + tool batch) began.
    TurnStart {
        /// Zero-based index within the run.
        turn_index: usize,
    },
    /// One turn ended.
    TurnEnd {
        /// Zero-based index within the run.
        turn_index: usize,
        /// The assistant message that closed the turn.
        message: Box<Message>,
        /// Tool results executed during the turn, in model order.
        tool_results: Vec<ToolResultMessage>,
    },
    /// A message entered the transcript.
    MessageStart {
        /// The message.
        message: Box<Message>,
    },
    /// Streaming update on the current assistant message.
    MessageUpdate {
        /// Snapshot of the growing message.
        message: Box<Message>,
    },
    /// A message finalized and joined the transcript.
    MessageEnd {
        /// The final message.
        message: Box<Message>,
    },
    /// A tool invocation started executing.
    ToolExecutionStart {
        /// Invocation id.
        tool_call_id: ToolCallId,
        /// Tool name.
        tool_name: String,
        /// Raw arguments.
        args: serde_json::Value,
    },
    /// A tool streamed partial output (throttled at the emission site).
    ToolExecutionUpdate {
        /// Invocation id.
        tool_call_id: ToolCallId,
        /// Tool name.
        tool_name: String,
        /// Partial result snapshot.
        partial: crate::tool::ToolPartial,
    },
    /// A tool invocation finished.
    ToolExecutionEnd {
        /// Invocation id.
        tool_call_id: ToolCallId,
        /// Tool name.
        tool_name: String,
        /// Whether the result is an error result.
        is_error: bool,
    },
    /// The gate needs a human decision before proceeding.
    PermissionRequested {
        /// The pending request.
        request: PermissionRequest,
    },
    /// A decision arrived for a pending request.
    PermissionDecided {
        /// Correlates with the request id.
        request_id: String,
        /// What was decided.
        decision: PermissionDecision,
    },
    /// Compaction pass started.
    CompactionStarted,
    /// Compaction pass finished.
    CompactionFinished,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticky_merge_prefers_worse_reasons() {
        let merged = TurnEndReason::Completed
            .clone()
            .merge_sticky(TurnEndReason::MaxTokens);
        assert_eq!(merged, TurnEndReason::MaxTokens);
        let e1 = TurnEndReason::Error("a".into());
        let e2 = TurnEndReason::Error("b".into());
        assert_eq!(e1.merge_sticky(e2), TurnEndReason::Error("a; b".into()));
        assert_eq!(
            TurnEndReason::Aborted.merge_sticky(TurnEndReason::Completed),
            TurnEndReason::Aborted
        );
    }

    #[test]
    fn agent_events_roundtrip() {
        let ev = AgentEvent::PermissionRequested {
            request: PermissionRequest {
                request_id: "p_1".into(),
                capability: "bash".into(),
                pattern: "cargo test".into(),
                detail: serde_json::json!({}),
            },
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("permission_requested"));
        assert_eq!(serde_json::from_str::<AgentEvent>(&json).unwrap(), ev);
    }
}
