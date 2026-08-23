//! Tool registry: name -> tool, plus descriptor export for provider payloads.

use std::collections::HashMap;
use std::sync::Arc;

use epsilon_contracts::tool::{Tool, ToolDescriptor};

/// Simple name-keyed registry. Duplicate names replace the old entry (last registration
/// wins), matching pi's extension-override behavior; collisions are visible via
/// [`ToolRegistry::register`] returning the replaced name.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool; returns the displaced tool's name if one existed.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Option<String> {
        let name = tool.descriptor().name.clone();
        self.tools
            .insert(name.clone(), tool)
            .map(|old| old.descriptor().name.clone())
    }

    /// Look up by tool name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Descriptors of every registered tool (provider payload order is insertion order of
    /// the underlying map — deterministic per process run).
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools
            .values()
            .map(|t| t.descriptor().clone())
            .collect()
    }
}
