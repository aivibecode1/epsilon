//! Built-in tools for Epsilon: the real file/shell/search capabilities.
//!
//! Every tool:
//! - validates arguments by typed deserialization (single source of truth with the
//!   schemars-derived provider schema, per ADR-004);
//! - refuses paths that escape the declared workspace;
//! - caps and spills oversized outputs (DSH locator pattern).

mod files;
mod search;
mod shell;
pub mod util;

use std::sync::Arc;

use epsilon_core::ToolRegistry;

pub use files::{EditArgs, EditTool, ReadArgs, ReadTool, WriteArgs, WriteTool};
pub use search::{GlobArgs, GlobTool, GrepArgs, GrepTool, LsArgs, LsTool};
pub use shell::{BashArgs, BashTool};

/// Register every built-in tool into `registry`.
pub fn register_builtins(registry: &mut ToolRegistry) {
    registry.register(Arc::new(ReadTool::new()));
    registry.register(Arc::new(WriteTool::new()));
    registry.register(Arc::new(EditTool::new()));
    registry.register(Arc::new(BashTool::new()));
    registry.register(Arc::new(GrepTool::new()));
    registry.register(Arc::new(GlobTool::new()));
    registry.register(Arc::new(LsTool::new()));
}

/// A fresh registry containing exactly the built-ins.
pub fn builtin_registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    register_builtins(&mut r);
    r
}
