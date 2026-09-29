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

use epsilon_core::ToolRegistry;
use epsilon_safety::Sandbox;

pub use files::{EditArgs, EditTool, ReadArgs, ReadTool, WriteArgs, WriteTool};
pub use find::{FindArgs, FindTool};
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

/// Whether edits ask a language server what the compiler thinks (M79).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lsp {
    /// Ask when a server for the file's language is installed; say nothing when none is.
    Auto,
    /// Never ask, never spawn. The default for library callers and tests.
    Off,
}

/// Register every built-in tool with diagnostics off.
///
/// The inert default on purpose: a library that silently spawns `rust-analyzer` because a test
/// wrote a file would be a surprise, and the product path opts in explicitly
/// ([`register_builtins_with`] with [`Lsp::Auto`]).
pub fn register_builtins(registry: &mut ToolRegistry, confinement: &Confinement) {
    register_builtins_with(registry, confinement, Lsp::Off);
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
    lsp: Lsp,
    skills: Vec<crate::skill::LoadableSkill>,
) {
    register_builtins_with(registry, confinement, lsp);
    if !skills.is_empty() {
        registry.register(Arc::new(crate::skill::SkillTool::new(skills)));
    }
}

/// Register every built-in tool into `registry` under the given confinement and diagnostics choice.
pub fn register_builtins_with(registry: &mut ToolRegistry, confinement: &Confinement, lsp: Lsp) {
    let diagnostics: Option<Arc<dyn epsilon_contracts::diagnostics::DiagnosticsProvider>> =
        match lsp {
            Lsp::Off => None,
            Lsp::Auto => {
                // The subprocess credential rule lives in epsilon-safety; hand it to the LSP
                // client rather than letting a second copy of the policy drift here.
                let cfg = epsilon_lsp::ServiceConfig {
                    scrub_env: Some(Arc::new(|cmd: &mut tokio::process::Command| {
                        epsilon_safety::scrub_secret_env(cmd)
                    })),
                    ..Default::default()
                };
                Some(Arc::new(epsilon_lsp::DiagnosticsService::new(cfg)))
            }
        };
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
    registry.register(Arc::new(TaskTool::new()));
}

/// A fresh registry containing exactly the built-ins.
pub fn builtin_registry(confinement: &Confinement) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    register_builtins(&mut r, confinement);
    r
}

/// The full built-in registry: confinement, plus language-server diagnostics when `lsp` asks for
/// them. This is what the product path uses.
pub fn builtin_registry_with(confinement: &Confinement, lsp: Lsp) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    register_builtins_with(&mut r, confinement, lsp);
    r
}
