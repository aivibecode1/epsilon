//! Built-in tools for Epsilon: the real file/shell/search capabilities.
//!
//! Every tool:
//! - validates arguments by typed deserialization (single source of truth with the
//!   schemars-derived provider schema, per ADR-004);
//! - refuses paths that escape the declared workspace;
//! - caps and spills oversized outputs (DSH locator pattern).

pub mod checkpoint;
mod edit_match;
mod files;
mod find;
pub mod goal;
pub mod image;
mod mutation;
mod read_log;
mod search;
mod shell;
pub mod skill;
mod task;
mod todo;
pub mod util;
mod webfetch;

use std::sync::Arc;

use epsilon_contracts::diagnostics::DiagnosticsProvider;
use epsilon_core::ToolRegistry;
use epsilon_safety::Sandbox;

pub use files::{EditArgs, EditTool, ReadArgs, ReadTool, WriteArgs, WriteTool};
pub use find::{FindArgs, FindTool};
pub mod goal_gate;
pub use goal::{
    CreateGoalArgs, CreateGoalTool, GetGoalArgs, GetGoalTool, GoalAction, UpdateGoalArgs,
    UpdateGoalTool,
};
pub use search::{GlobArgs, GlobTool, GrepArgs, GrepTool, LS_DEFAULT_LIMIT, LsArgs, LsTool};
pub use shell::CommandVerdict;
pub use shell::{BashArgs, BashTool, ChildStdio};
pub use task::{TaskArgs, TaskTool};
pub use todo::{TodoArgs, TodoItemArgs, TodoStatus, TodoTool};
pub use util::TOOL_OUTPUT_MAX_BYTES;
pub use webfetch::{WebFetchArgs, WebFetchTool};

/// How Layer-2 OS confinement applies to the built-ins (ADR-005).
#[derive(Clone)]
pub enum Confinement {
    /// Best backend for this platform; commands REFUSE to run when none exists
    /// (fail-closed). Windows: Job Objects. Unix: bubblewrap when installed.
    Auto,
    /// Confine commands to the workspace's *contents*: they may read and execute, never
    /// write (the read-only rung of the mode ladder, port plan item 15). Windows job
    /// objects cannot enforce it, so the per-call report says which part held.
    ReadOnly,
    /// Explicitly run unconfined — trusted automation and tests only; every tool
    /// result records `confinement: "off"` so transcripts stay auditable.
    Off,
    /// Bring your own backend.
    Custom(Arc<dyn Sandbox>),
}

impl std::fmt::Debug for Confinement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Confinement::Auto => write!(f, "Auto"),
            Confinement::ReadOnly => write!(f, "ReadOnly"),
            Confinement::Off => write!(f, "Off"),
            Confinement::Custom(_) => write!(f, "Custom"),
        }
    }
}

fn resolve_sandbox(choice: &Confinement) -> Option<Arc<dyn Sandbox>> {
    match choice {
        Confinement::Off => None,
        Confinement::Custom(sb) => Some(Arc::clone(sb)),
        Confinement::Auto | Confinement::ReadOnly => Some(epsilon_safety::best_available()),
    }
}

/// Register every built-in tool with diagnostics off.
///
/// The inert default on purpose: a library that silently spawns `rust-analyzer` because a test
/// wrote a file would be a surprise, and the product path opts in explicitly by passing a
/// [`DiagnosticsProvider`] to [`register_builtins_with`].
pub fn register_builtins(registry: &mut ToolRegistry, confinement: &Confinement) {
    register_builtins_with(registry, confinement, None);
}

/// Register every built-in tool, with the skills the model may load.
///
/// Skills are discovered by the frontend, which owns the trust policy; passing
/// them in keeps that decision in one place instead of teaching this crate where
/// skill directories live. An empty list registers no `skill` tool at all, so a
/// workspace with no skills does not pay for a tool that could only ever fail.
pub fn register_builtins_with_skills(
    registry: &mut ToolRegistry,
    confinement: &Confinement,
    diagnostics: Option<Arc<dyn DiagnosticsProvider>>,
    skills: Vec<crate::skill::LoadableSkill>,
) {
    register_builtins_with(registry, confinement, diagnostics);
    if !skills.is_empty() {
        registry.register(Arc::new(crate::skill::SkillTool::new(skills)));
    }
}

/// Register every built-in tool into `registry` under the given confinement and diagnostics handle.
///
/// **`diagnostics` is a handle, not a policy (M215).** Whether to start a language server at all,
/// and how to configure one, is the frontend's decision — it owns the `--lsp` flag and the
/// workspace. What the write and edit tools need is only the seam, so this is the shape
/// `ToolContext::task_host` and `goal_host` already use, and it is why this crate does not depend
/// on `epsilon-lsp`. `None` means no provider, and the tools then say nothing about diagnostics
/// rather than failing.
pub fn register_builtins_with(
    registry: &mut ToolRegistry,
    confinement: &Confinement,
    diagnostics: Option<Arc<dyn DiagnosticsProvider>>,
) {
    let sandbox = resolve_sandbox(confinement);
    registry.register(Arc::new(ReadTool::new()));
    let write = WriteTool::new();
    let edit = EditTool::new();
    let (write, edit) = match diagnostics.as_ref() {
        Some(provider) => (
            write.with_diagnostics(Arc::clone(provider)),
            edit.with_diagnostics(Arc::clone(provider)),
        ),
        None => (write, edit),
    };
    registry.register(Arc::new(write));
    registry.register(Arc::new(edit));
    let mut bash = match sandbox.as_ref() {
        Some(sb) => BashTool::new().with_sandbox(Arc::clone(sb)),
        None => BashTool::new(),
    };
    if matches!(confinement, Confinement::ReadOnly) {
        bash = bash.with_session_mode(epsilon_safety::SandboxMode::ReadOnly);
    }
    registry.register(Arc::new(bash));
    registry.register(Arc::new(GrepTool::new()));
    registry.register(Arc::new(GlobTool::new()));
    registry.register(Arc::new(FindTool::new()));
    registry.register(Arc::new(LsTool::new()));
    // M30: closes the agent-contract gap — the contract told the model to use
    // `todo` long before a tool with that name existed.
    // Plan 010: webfetch joins the read group (Ask-gated, spill-aware).
    registry.register(Arc::new(WebFetchTool::new()));
    registry.register(Arc::new(TodoTool::new()));
    // The goal tools reach session state through `ToolContext::goal_host`, so registering them
    // costs nothing when the agent has no durable mirror: they say so rather than failing
    // obscurely. (A `task` child gets its own host through the same seam.)
    registry.register(Arc::new(goal::CreateGoalTool::new()));
    registry.register(Arc::new(goal::GetGoalTool::new()));
    // The gate runner is built from **the same `sandbox`** the shell tool got. That is the
    // security property, not an implementation convenience: a goal gate runs a shell
    // command, and a gate running outside the session's confinement would be a command the
    // agent's own `bash` is refused — a confused deputy wearing the permission gate as a
    // disguise. One resolution, so a transcript cannot name one backend for a tool call and
    // another for the gate that decided whether it succeeded.
    let update_goal = goal::UpdateGoalTool::new().with_gate_runner(Arc::new(
        crate::goal_gate::GoalGateRunner::new(sandbox.clone()),
    ));
    registry.register(Arc::new(update_goal));
    registry.register(Arc::new(TaskTool::new()));
}

/// A fresh registry containing exactly the built-ins.
pub fn builtin_registry(confinement: &Confinement) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    register_builtins(&mut r, confinement);
    r
}

/// The full built-in registry: confinement, plus diagnostics when the caller supplies a provider.
/// This is what the product path uses.
pub fn builtin_registry_with(
    confinement: &Confinement,
    diagnostics: Option<Arc<dyn DiagnosticsProvider>>,
) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    register_builtins_with(&mut r, confinement, diagnostics);
    r
}
