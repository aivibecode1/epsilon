//! The Tool trait: how every callable capability — built-in, extension, or MCP — looks to the loop.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::content::ContentBlock;
use crate::ids::ToolCallId;
use crate::message::Usage;

/// Whether a tool may run concurrently with others in the same batch.
///
/// Mirrors pi's per-tool `executionMode` + DSH's `isConcurrencySafe`, unified: tools declare
/// once, the pool enforces. Any `Sequential` tool in a batch serializes the whole batch
/// (barrier semantics), because partial ordering is harder to reason about than all-or-nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Safe to run concurrently with other parallel-safe calls.
    Parallel,
    /// Acts as a barrier: runs alone within its batch.
    Sequential,
}

/// Static description of a tool as sent to providers and shown in UIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Name used in provider tool-call payloads (e.g. `read`, `mcp__gh__list_prs`).
    pub name: String,
    /// Human label for UI.
    pub label: String,
    /// LLM-facing description (loaded from `.txt` at build time for built-ins).
    pub description: String,
    /// JSON Schema (2020-12) object describing accepted arguments.
    ///
    /// Stored as raw JSON rather than a schemars type so this crate stays dependency-light;
    /// built-ins derive it from typed structs via schemars in their own crate and convert.
    pub parameters_schema: serde_json::Value,
    /// Concurrency classification.
    pub execution_mode: ExecutionMode,
}

impl ToolDescriptor {
    /// Validate that `arguments` is structurally plausible against the declared schema.
    ///
    /// V1 checks shape only (object-ness); full JSON-Schema validation arrives with the
    /// safety brick, which owns the validator dependency. Kept here so the *contract* of
    /// "validate before execute" is enforced by type, not convention.
    pub fn basic_validate(&self, arguments: &serde_json::Value) -> Result<(), String> {
        if !arguments.is_object() {
            return Err("arguments must be a JSON object".to_string());
        }
        Ok(())
    }
}

/// One invocation request handed to [`Tool::execute`].
#[derive(Debug, Clone)]
pub struct ToolInvocation {
    /// Provider-assigned invocation id (echoed in results).
    pub tool_call_id: ToolCallId,
    /// Raw arguments (already passed `basic_validate`).
    pub args: serde_json::Value,
}

/// Environment handed to every execution.
#[derive(Clone)]
pub struct ToolContext {
    /// Owning session id.
    pub session_id: String,
    /// Working directory the agent operates in.
    pub cwd: PathBuf,
    /// Directory where oversized outputs spill (locator pattern, ADR from DSH).
    pub spill_dir: PathBuf,
    /// Cooperative cancellation; checked by long-running tools between steps.
    pub token: CancellationToken,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("session_id", &self.session_id)
            .field("cwd", &self.cwd)
            .field("spill_dir", &self.spill_dir)
            .field("cancelled", &self.token.is_cancelled())
            .finish()
    }
}

/// Streaming partial result from a long-running tool (pi's `AgentToolUpdateCallback`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPartial {
    /// Snapshot of content produced so far.
    pub content: Vec<ContentBlock>,
    /// Structured details snapshot (e.g. truncation state).
    pub details: serde_json::Value,
}

/// Callback receiving throttled partial updates; calls after completion are dropped.
pub type ToolUpdateSink = Arc<dyn Fn(ToolPartial) + Send + Sync>;

/// Successful execution outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    /// Content returned to the model.
    pub content: Vec<ContentBlock>,
    /// Structured payload for UI/log rendering; never sent to the model.
    pub details: serde_json::Value,
    /// Token usage of the tool's own model work (subagents), if any.
    pub usage: Option<Usage>,
    /// Early-termination hint (pi/DSH): end the run after this batch when EVERY result
    /// in the batch sets it — prevents one tool from hijacking the stop decision.
    #[serde(default)]
    pub terminate: bool,
}

impl ToolOutput {
    /// Text-only output convenience constructor.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::text(text)],
            details: serde_json::json!({}),
            usage: None,
            terminate: false,
        }
    }
}

/// Failure raised by a tool body. Converted into an error `ToolResultMessage`
/// (is_error=true) — tools report failures; they do not panic the run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFailure {
    /// Message the model will see in the error result.
    pub message: String,
}

impl ToolFailure {
    /// Construct from anything string-like.
    pub fn msg(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// The trait every callable capability implements.
///
/// Ported from pi's AgentTool with three Rust-shaped changes:
/// 1. schema travels with the descriptor as JSON Schema (single source of truth lives in
///    typed structs via schemars at the definition site);
/// 2. failures return `Err(ToolFailure)` and are converted to error results by the pipeline —
///    panics are caught at the boundary, never propagated;
/// 3. cancellation is a token checked cooperatively plus kill-on-drop at the executor level.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Static descriptor (name/schema/mode).
    fn descriptor(&self) -> &ToolDescriptor;

    /// Execute one invocation. Implementations should:
    /// - poll `ctx.token` between work units and bail out promptly on cancellation;
    /// - emit `on_update` sparingly (the sink throttles, but formatting costs are real).
    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure>;
}
