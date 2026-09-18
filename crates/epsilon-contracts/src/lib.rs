//! Shared vocabulary for every Epsilon brick.
//!
//! This crate is the "stud standard" of the lego methodology: message, stream, agent-event,
//! session-entry, and tool types that all other crates compile against. It contains zero
//! business logic and depends on no other Epsilon crate by design (`design/epsilon-architecture.md` §2).
//!
//! Semantics are ported from pi's `packages/agent/src/types.ts` and
//! `packages/agent/src/harness/session/types.ts`; divergences are documented per item.

pub mod agent_event;
pub mod content;
pub mod diagnostics;
pub mod ids;
pub mod message;
pub mod session_entry;
pub mod stream;
pub mod tool;
pub mod turn;

pub use agent_event::{AgentEvent, PermissionDecision, PermissionRequest, TurnEndReason};
pub use content::{ContentBlock, ImageRef, UserContent};
pub use ids::{MessageId, SessionId, ToolCallId, new_id};
pub use message::{
    AssistantMessage, CustomMessage, Message, StopReason, ThinkingLevel, ToolResultMessage, Usage,
};
pub use session_entry::{EntryKind, SessionEntry};
pub use stream::StreamEvent;
pub use tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolPartial, ToolUpdateSink,
};
pub use turn::{NextTurnContext, NoopTurnHooks, TurnHooks, TurnModel, TurnUpdate};
