//! The Tool trait: how every callable capability — built-in, extension, or MCP — looks to the loop.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::content::ContentBlock;
use crate::goal::{GoalError, GoalOp, GoalRef, GoalView};
use crate::ids::ToolCallId;
use crate::message::Usage;

/// Whether a tool may run concurrently with others in the same batch.
///
/// Mirrors pi's per-tool `executionMode` + DSH's `isConcurrencySafe`, unified: tools declare
/// once, the pool enforces. A `Sequential` tool with **no** [`Tool::resource_key`] serializes the
/// whole batch (its reach is unknown — a shell command can touch anything). A `Sequential` tool
/// that names its resource only orders itself against calls that name the same one (M72), which
/// is what lets two edits to two different files run at the same time.
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

/// A bounded rendering of a payload for an error message: enough to recognise, short enough that
/// a malicious or broken payload cannot fill the transcript.
fn bounded(text: &str) -> String {
    const MAX: usize = 200;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX).collect();
    format!("{head}… ({} chars total)", text.chars().count())
}

impl ToolDescriptor {
    /// Validate that `arguments` is structurally plausible against the declared schema.
    ///
    /// V1 checks shape only (object-ness); full JSON-Schema validation arrives with the
    /// safety brick, which owns the validator dependency. Kept here so the *contract* of
    /// "validate before execute" is enforced by type, not convention.
    pub fn basic_validate(&self, arguments: &serde_json::Value) -> Result<(), String> {
        if !arguments.is_object() {
            // M70: the old message said only what was expected. When a wire could not read the
            // model's arguments it hands the raw text here (as a JSON string), and the model
            // needs to see what it actually sent to fix it.
            return Err(match arguments {
                serde_json::Value::String(raw) => format!(
                    "the tool arguments were not valid JSON — received: {}",
                    bounded(raw)
                ),
                other => format!(
                    "arguments must be a JSON object, got {}",
                    serde_json::to_string(other)
                        .map(|s| bounded(&s))
                        .unwrap_or_else(|_| "an unprintable value".to_string())
                ),
            });
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

/// Result returned by an isolated [`TaskHost`] run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskHostResult {
    /// Combined textual output of the subagent run.
    pub text: String,
    /// Aggregated token usage, if reported.
    pub usage: Option<Usage>,
    /// Structured details for UI (may be empty).
    pub details: serde_json::Value,
}

/// Result returned from a [`ToolHooks::before_tool_call`] hook.
///
/// Returning `block: true` prevents the tool from executing; the loop emits an
/// error tool result instead, with `reason` as its text (or a default blocked
/// message when omitted). A blocked result can also set `terminate: true` to
/// participate in the batch early-termination rule (ALL results must agree).
#[derive(Debug, Clone, Default)]
pub struct BeforeToolCallResult {
    /// Prevent execution; emit an error result instead.
    pub block: bool,
    /// Text shown in the error result when blocked.
    pub reason: Option<String>,
    /// Early-termination hint when blocked.
    pub terminate: bool,
}

/// Partial override returned from a [`ToolHooks::after_tool_call`] hook.
///
/// Merge is field-by-field (`None` keeps the executed value; no deep merge):
/// `content` replaces the full content array, `details` the full details
/// payload, `is_error` the error flag, `usage` the tool result usage,
/// `terminate` the early-termination hint.
#[derive(Debug, Clone, Default)]
pub struct AfterToolCallResult {
    /// Replace the full content array.
    pub content: Option<Vec<ContentBlock>>,
    /// Replace the full details payload.
    pub details: Option<serde_json::Value>,
    /// Replace the error flag.
    pub is_error: Option<bool>,
    /// Replace the tool result usage.
    pub usage: Option<Usage>,
    /// Replace the early-termination hint.
    pub terminate: Option<bool>,
}

/// Context passed to [`ToolHooks::before_tool_call`].
#[derive(Debug, Clone)]
pub struct BeforeToolCallContext {
    /// Provider-assigned invocation id.
    pub tool_call_id: ToolCallId,
    /// Registered tool name.
    pub tool_name: String,
    /// Validated tool arguments.
    pub args: serde_json::Value,
}

/// Context passed to [`ToolHooks::after_tool_call`].
#[derive(Debug, Clone)]
pub struct AfterToolCallContext {
    /// Provider-assigned invocation id.
    pub tool_call_id: ToolCallId,
    /// Registered tool name.
    pub tool_name: String,
    /// Validated tool arguments.
    pub args: serde_json::Value,
    /// The executed tool result before overrides.
    pub result: crate::message::ToolResultMessage,
    /// Whether the executed result is currently treated as an error.
    pub is_error: bool,
}

/// Interception points around tool execution (pi's `beforeToolCall` /
/// `afterToolCall` in `AgentLoopConfig`).
///
/// Hooks run INLINE in the execution pipeline: `before_tool_call` after
/// argument validation (and after the permission gate — denied calls never
/// reach hooks), `after_tool_call` after the tool settles but before the
/// result commits to the batch. A failing `after_tool_call` replaces the
/// result with an error result (pi parity); a failing/blocking path never
/// panics the loop.
///
/// Object-safe + `Send + Sync` so hosts (extension runner, compaction
/// summarizer, tests) can share one `Arc<dyn ToolHooks>` with the agent.
#[async_trait]
pub trait ToolHooks: Send + Sync {
    /// Called before a tool executes. Return `block: true` to prevent
    /// execution (an error result is emitted instead). `Err` also blocks,
    /// with the message as the error text (pi surfaces hook errors as
    /// error results, never loop panics).
    async fn before_tool_call(
        &self,
        ctx: BeforeToolCallContext,
    ) -> Result<BeforeToolCallResult, String> {
        let _ = ctx;
        Ok(BeforeToolCallResult::default())
    }

    /// Called after a tool settles. Return field overrides to merge into the
    /// executed result (`None` = keep). `Err` replaces the result with an
    /// error result carrying the message (pi parity).
    async fn after_tool_call(
        &self,
        ctx: AfterToolCallContext,
    ) -> Result<Option<AfterToolCallResult>, String> {
        let _ = ctx;
        Ok(None)
    }
}

/// No-op hooks (default when no host registers any).
#[derive(Debug, Default)]
pub struct NoopToolHooks;

#[async_trait]
impl ToolHooks for NoopToolHooks {}

/// One delegated `task` call (M53: wires the tool's `background` and
/// `task_id` parameters to the child-agent registry).
#[derive(Debug, Clone)]
pub struct TaskRequest {
    /// Work for the child. Empty (with `resume`) means "report status".
    pub prompt: String,
    /// Short label (pi/opencode `description`).
    pub description: String,
    /// Agent-type name; `default` means "no def matched".
    pub subagent_type: String,
    /// Return as soon as the child starts instead of awaiting it.
    pub background: bool,
    /// Address an existing task instead of starting a fresh child.
    pub resume: Option<String>,
    /// Model the child should run on (catalog id, e.g. `deepseek-flash` for a
    /// cheap scout). `None` inherits, exactly like an omitted model on an
    /// agent def (opencode `next.model ?? parent model`). Ignored when the
    /// addressed agent type defines its own model — the operator's config
    /// outranks a call — and on `resume`, which keeps the child's model.
    pub model: Option<String>,
}

/// Lifecycle of one tracked task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    /// The child is running right now.
    Running,
    /// The child finished; `output` holds its summary.
    Completed,
    /// The child failed; `output` holds the error.
    Failed,
}

/// Read-only snapshot of a tracked task (status reports, UI badges).
#[derive(Debug, Clone)]
pub struct TaskSnapshot {
    /// Task id (`task-…`) — the handle later calls use.
    pub id: String,
    /// Label from the dispatching call.
    pub description: String,
    /// Agent type the child runs as.
    pub subagent_type: String,
    /// Model the child actually runs on (agent-def override, then the call's
    /// request, then the parent's inherited model) — reported so a delegation
    /// never silently diverges from what was asked for.
    pub model: Option<String>,
    /// Current state.
    pub state: TaskState,
    /// Latest output: the summary once finished, the running note before that.
    pub output: String,
    /// Assistant turns the child has taken so far.
    pub turns: u32,
    /// Aggregated usage when the child has finished at least once.
    pub usage: Option<Usage>,
}

/// What one `task` call produced.
#[derive(Debug, Clone)]
pub enum TaskOutcome {
    /// The child finished within the call (foreground path).
    Completed(TaskHostResult),
    /// A child is running (or a status report was requested) — `id` addresses it.
    Running(TaskSnapshot),
}

/// Host capable of running a delegated `task` in an isolated child agent.
///
/// Implemented by `epsilon-core::Agent` and injected via [`ToolContext::task_host`].
/// The trait lives in the contracts crate so `epsilon-tools` can call it without
/// depending on `epsilon-core` types (dependency inversion, no cycle).
#[async_trait]
pub trait TaskHost: Send + Sync {
    /// Run, resume, or report a delegated task. Children stay alive between
    /// calls, so a later request can continue one by id.
    async fn run_task(&self, request: TaskRequest) -> Result<TaskOutcome, String>;

    /// Snapshot a tracked task; `None` for an id this host does not know.
    fn task_snapshot(&self, id: &str) -> Option<TaskSnapshot> {
        let _ = id;
        None
    }

    /// Ids still tracked (used to make unknown-id errors honest).
    fn task_ids(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Host that reads and mutates the session's one long-running goal.
///
/// Implemented by `epsilon-core::Agent` and injected via [`ToolContext::goal_host`], for the same
/// reason [`TaskHost`] exists: the tool crate must be able to reach session state without
/// depending on the crate that owns it, and the dependency inversion belongs in contracts rather
/// than in a second implementation.
///
/// Both methods are **synchronous on purpose.** The store they reach is behind a `std::sync`
/// mutex and does file I/O, and an `async fn` holding that guard across an await point is how a
/// tool deadlocks a run. There is nothing to await here: a goal change is one append.
pub trait GoalHost: Send + Sync {
    /// The current goal, with this process's activation state. `None` when no goal is current.
    fn current_goal(&self) -> Option<GoalView>;

    /// Apply one operation.
    ///
    /// `expect` is the compare-and-set reference the caller read. `None` means "whatever is
    /// current" and is for a *frontend* acting on what the user is looking at right now;
    /// `Some` is for a caller holding a view it read earlier — which is every model tool call,
    /// and the reason a stale tool call cannot silently overwrite a change the user made in
    /// between.
    ///
    /// `Ok(None)` is the outcome of `clear` and means "there is no goal now" — an outcome rather
    /// than an error, because the caller asked for exactly that.
    fn apply_goal(
        &self,
        expect: Option<GoalRef>,
        op: GoalOp,
    ) -> Result<Option<GoalView>, GoalError>;
}

/// Environment handed to every execution.
#[derive(Clone)]
pub struct ToolContext {
    /// Owning session id.
    pub session_id: String,
    /// Opaque observation lifetime, shared by calls from one agent/session view.
    /// New agents and restored transcripts must use a fresh allocation; clones share it.
    pub observation_scope: Arc<()>,
    /// Working directory the agent operates in.
    pub cwd: PathBuf,
    /// Directory where oversized outputs spill (locator pattern, ADR from DSH).
    pub spill_dir: PathBuf,
    /// Cooperative cancellation; checked by long-running tools between steps.
    pub token: CancellationToken,
    /// Optional isolated host for `task` delegation (injected by the loop; `None` in
    /// unit tests and non-loop callers keeps the capture fallback).
    pub task_host: Option<Arc<dyn TaskHost>>,
    /// Optional host for the session goal (injected by the loop; `None` in unit tests and for an
    /// agent with no durable mirror, where a goal tool can only report that there is nowhere to
    /// record one).
    pub goal_host: Option<Arc<dyn GoalHost>>,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("session_id", &self.session_id)
            .field("cwd", &self.cwd)
            .field("spill_dir", &self.spill_dir)
            .field("cancelled", &self.token.is_cancelled())
            .field("has_task_host", &self.task_host.is_some())
            .field("has_goal_host", &self.goal_host.is_some())
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

    /// The single resource this call acts on, when it has exactly one.
    ///
    /// The batch planner uses this to decide what may run concurrently: two calls naming the same
    /// resource are ordered by call order; calls naming different resources are independent,
    /// provided neither is a whole-batch barrier. `None` means "unknown reach" — a shell command,
    /// a subagent, a network fetch — and such a call keeps the conservative behaviour.
    ///
    /// `base` is the directory relative paths resolve against — the same `cwd` the tool will
    /// receive when it executes, so the planner and the execution agree by construction.
    ///
    /// Return a *normalised absolute* path where possible: `/a/b` and `/a/./b` are the same file,
    /// and a planner that cannot tell them apart serializes work that could have run in parallel.
    /// Reads and writes are distinguished by [`ToolDescriptor::execution_mode`] — readers declare
    /// `Parallel`, mutators `Sequential` — which is the same distinction the pool already
    /// enforces, rather than a second vocabulary that could disagree with it.
    fn resource_key(
        &self,
        arguments: &serde_json::Value,
        base: &std::path::Path,
    ) -> Option<PathBuf> {
        let _ = (arguments, base);
        None
    }

    /// A preview of what this call will change, for the screen that decides whether it may.
    ///
    /// The dialog's one-line `what` names the *subject* — the path, the command. This carries
    /// the **change**: a unified diff between the bytes the file holds now and the bytes the
    /// call would leave, in the format [`crate::tool::Tool`]'s result details already use for
    /// an edit. Without it a reader approves a write having seen neither the old nor the new
    /// content, which is the gap the dialog was carrying (opencode 1.18.34,
    /// `packages/tui/src/routes/session/permission.tsx:49-85` renders the request's diff and
    /// prints `No diff provided` rather than omitting it).
    ///
    /// `None` — **the default, so every tool that does not implement it is unchanged** — means
    /// "there is nothing to preview". The core merges the preview into the ask's `detail`, so
    /// a front end renders whatever it is given and adds no policy of its own.
    ///
    /// # It is a preview, so it is read-only
    ///
    /// It runs at **ask** time, before the gate decides, and therefore runs for calls that are
    /// ultimately denied. It must not write, rename, or otherwise change anything: no
    /// checkpoints, no read-log entries, no mutation queue. Reading the current bytes is the
    /// whole cost.
    ///
    /// # The two ends of one disagreement
    ///
    /// It cannot be authoritative, and does not need to be. It reads the file *now*, while
    /// `execute` re-reads it under the mutation queue; if the bytes change in between, the
    /// executed write wins and its own `read_log`/checkpoint guards still apply. A preview is
    /// what the approver is shown, not what runs.
    ///
    /// `base` is the same workspace root [`Tool::resource_key`] resolves against, so the
    /// preview and the call name the same file by construction.
    fn preview(&self, arguments: &serde_json::Value, base: &std::path::Path) -> Option<String> {
        let _ = (arguments, base);
        None
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> ToolDescriptor {
        ToolDescriptor {
            name: "read".to_string(),
            label: "Read".to_string(),
            description: "Read a file".to_string(),
            parameters_schema: serde_json::json!({"type": "object"}),
            execution_mode: ExecutionMode::Parallel,
        }
    }

    #[test]
    fn an_object_passes_validation() {
        assert!(
            descriptor()
                .basic_validate(&serde_json::json!({"path": "a.rs"}))
                .is_ok()
        );
        assert!(descriptor().basic_validate(&serde_json::json!({})).is_ok());
    }

    #[test]
    fn an_unreadable_payload_is_quoted_back_to_the_model() {
        // M70: a wire that could not read the model's arguments hands the raw text on as a
        // string. The message used to say only "arguments must be a JSON object", which tells the
        // model nothing about what it sent.
        let err = descriptor()
            .basic_validate(&serde_json::Value::String("{unquoted: keys}".to_string()))
            .unwrap_err();
        assert!(
            err.contains("not valid JSON"),
            "the failure names the real problem: {err}"
        );
        assert!(
            err.contains("{unquoted: keys}"),
            "and quotes what arrived: {err}"
        );
    }

    #[test]
    fn the_quoted_payload_is_bounded() {
        let huge = "x".repeat(5_000);
        let err = descriptor()
            .basic_validate(&serde_json::Value::String(huge.clone()))
            .unwrap_err();
        assert!(
            err.chars().count() < 400,
            "a broken payload cannot fill the transcript: {} chars",
            err.chars().count()
        );
        assert!(
            err.contains("chars total"),
            "and says how much there was: {err}"
        );
    }

    /// A tool that declares nothing keeps the conservative scheduling path (M72).
    struct Undeclared;
    #[async_trait::async_trait]
    impl Tool for Undeclared {
        fn descriptor(&self) -> &ToolDescriptor {
            static D: std::sync::OnceLock<ToolDescriptor> = std::sync::OnceLock::new();
            D.get_or_init(|| ToolDescriptor {
                name: "undeclared".into(),
                label: "Undeclared".into(),
                description: "declares nothing".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                execution_mode: ExecutionMode::Sequential,
            })
        }
        async fn execute(
            &self,
            _call: ToolInvocation,
            _ctx: ToolContext,
            _on_update: ToolUpdateSink,
        ) -> Result<ToolOutput, ToolFailure> {
            Err(ToolFailure::msg("never executed"))
        }
    }

    #[test]
    fn a_tool_that_names_no_resource_is_a_barrier() {
        let tool = Undeclared;
        assert_eq!(
            tool.resource_key(
                &serde_json::json!({"path": "a.rs"}),
                std::path::Path::new("/ws")
            ),
            None,
            "the default must not let a tool be scheduled as if its reach were known"
        );
        // The mode is what tells the planner whether `None` means "reader, harmless" or
        // "unknown reach, serialize everything".
        assert_eq!(tool.descriptor().execution_mode, ExecutionMode::Sequential);
    }

    /// A tool that implements nothing but `execute` — which is every implementor that
    /// existed before the preview seam did.
    #[test]
    fn a_tool_that_previews_nothing_keeps_the_dialog_exactly_as_it_was() {
        // The default is `None`, so nothing is added to the ask's detail and the front end
        // renders the same six-row dialog it rendered before. A defaulted method is the only
        // shape that can promise this: every implementor inherits it without being edited.
        let tool = Undeclared;
        assert_eq!(
            tool.preview(
                &serde_json::json!({ "path": "a.rs", "content": "x" }),
                std::path::Path::new("/ws")
            ),
            None,
            "the seam must be opt-in: a tool that does not preview must not change the ask"
        );
    }

    /// A tool that does preview — the shape `write` and `edit` take.
    struct Previewing;
    #[async_trait::async_trait]
    impl Tool for Previewing {
        fn descriptor(&self) -> &ToolDescriptor {
            static D: std::sync::OnceLock<ToolDescriptor> = std::sync::OnceLock::new();
            D.get_or_init(|| ToolDescriptor {
                name: "write".into(),
                label: "Write".into(),
                description: "previews".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                execution_mode: ExecutionMode::Sequential,
            })
        }
        fn preview(
            &self,
            arguments: &serde_json::Value,
            _base: &std::path::Path,
        ) -> Option<String> {
            // The real tools read the file; this one only shows that the seam carries the
            // diff text through to whoever merged it into the ask.
            arguments
                .get("content")
                .and_then(|v| v.as_str())
                .map(|c| format!("@@ -0,0 +1,1 @@\n+{c}\n"))
        }
        async fn execute(
            &self,
            _call: ToolInvocation,
            _ctx: ToolContext,
            _on_update: ToolUpdateSink,
        ) -> Result<ToolOutput, ToolFailure> {
            Err(ToolFailure::msg("never executed"))
        }
    }

    #[test]
    fn a_tool_that_previews_names_the_change_it_would_make() {
        let tool = Previewing;
        let preview = tool
            .preview(
                &serde_json::json!({ "path": "a.rs", "content": "let x = 1;" }),
                std::path::Path::new("/ws"),
            )
            .expect("a previewing tool previews");
        assert!(
            preview.contains("+let x = 1;"),
            "the added line is in it: {preview}"
        );
    }

    #[test]
    fn other_shapes_still_say_what_was_expected() {
        let err = descriptor()
            .basic_validate(&serde_json::json!([1, 2, 3]))
            .unwrap_err();
        assert!(err.contains("JSON object"), "{err}");
        assert!(err.contains("[1,2,3]"), "the value is shown: {err}");
    }
}
