//! Built-in tools for Epsilon: the real file/shell/search capabilities.
//!
//! Every tool:
//! - validates arguments by typed deserialization (single source of truth with the
//!   schemars-derived provider schema, per ADR-004);
//! - refuses paths that escape the declared workspace;
//! - caps and spills oversized outputs (DSH locator pattern).

mod edit_match;
mod files;
mod search;
mod shell;
pub mod util;

use std::sync::Arc;

use epsilon_core::ToolRegistry;
use epsilon_safety::Sandbox;

pub use files::{EditArgs, EditTool, ReadArgs, ReadTool, WriteArgs, WriteTool};
pub use search::{GlobArgs, GlobTool, GrepArgs, GrepTool, LsArgs, LsTool};
pub use shell::{BashArgs, BashTool};

/// How Layer-2 OS confinement applies to the built-ins (ADR-005).
#[derive(Clone)]
pub enum Confinement {
    /// Best backend for this platform; commands REFUSE to run when none exists
    /// (fail-closed). Windows: Job Objects. Unix: bubblewrap when installed.
    Auto,
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
            Confinement::Off => write!(f, "Off"),
            Confinement::Custom(_) => write!(f, "Custom"),
        }
    }
}

fn resolve_sandbox(choice: &Confinement) -> Option<Arc<dyn Sandbox>> {
    match choice {
        Confinement::Off => None,
        Confinement::Custom(sb) => Some(Arc::clone(sb)),
        Confinement::Auto => Some(epsilon_safety::best_available()),
    }
}

/// Register every built-in tool into `registry` under the given confinement choice.
pub fn register_builtins(registry: &mut ToolRegistry, confinement: &Confinement) {
    let sandbox = resolve_sandbox(confinement);
    registry.register(Arc::new(ReadTool::new()));
    registry.register(Arc::new(WriteTool::new()));
    registry.register(Arc::new(EditTool::new()));
    let bash = match sandbox.as_ref() {
        Some(sb) => BashTool::new().with_sandbox(Arc::clone(sb)),
        None => BashTool::new(),
    };
    registry.register(Arc::new(bash));
    registry.register(Arc::new(GrepTool::new()));
    registry.register(Arc::new(GlobTool::new()));
    registry.register(Arc::new(LsTool::new()));
}

/// A fresh registry containing exactly the built-ins.
pub fn builtin_registry(confinement: &Confinement) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    register_builtins(&mut r, confinement);
    r
}
