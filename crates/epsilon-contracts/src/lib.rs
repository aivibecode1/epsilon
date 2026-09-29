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
pub mod goal;
pub mod ids;
pub mod lagged_notice;
pub mod message;
pub mod prompt_template;
pub mod queue_mode;
pub mod session_entry;
pub mod session_state;
pub mod stream;
pub mod tool;
pub mod turn;

pub use agent_event::{AgentEvent, PermissionDecision, PermissionRequest, TurnEndReason};
pub use content::{ContentBlock, ImageRef, UserContent};
pub use goal::{
    BLOCKED_MIN_ROUNDS, DEFAULT_MAX_GOAL_ROUNDS, GoalBlocker, GoalChange, GoalError, GoalOp,
    GoalRef, GoalSnapshot, GoalStatus, GoalView,
};
pub use ids::{MessageId, SessionId, ToolCallId, new_id};
pub use lagged_notice::LaggedNotice;
pub use message::{
    AssistantMessage, CustomMessage, Message, StopReason, ThinkingLevel, ToolResultMessage, Usage,
};
pub use queue_mode::{KNOWN_QUEUE_MODES, QueueMode};
pub use session_entry::{
    CURRENT_SESSION_VERSION, EntryKind, SessionEntry, SessionHeader, SessionHeaderKind,
    SessionLine, SessionTreeNode,
};
pub use session_state::SessionState;
pub use stream::StreamEvent;
pub use tool::{
    AfterToolCallContext, AfterToolCallResult, BeforeToolCallContext, BeforeToolCallResult,
    ExecutionMode, NoopToolHooks, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolHooks,
    ToolInvocation, ToolOutput, ToolPartial, ToolUpdateSink,
};
pub use turn::{NextTurnContext, NoopTurnHooks, TurnHooks, TurnModel, TurnUpdate};
