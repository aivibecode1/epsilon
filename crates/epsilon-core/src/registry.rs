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

    /// Descriptors of every registered tool, in a **stable, sorted order**.
    ///
    /// The order matters more than it looks: this list is what the prompt's
    /// STABLE tier prints *and* what every provider payload sends as `tools`.
    /// It used to come straight out of a `HashMap`, whose iteration order is
    /// seeded per process — so two runs of the same session sent byte-different
    /// system prompts and tool arrays, and a provider prompt cache could never
    /// hit across invocations. Sorting by name makes the bytes a function of the
    /// tool set alone (ADR-008's byte-stable premise), which is the whole point
    /// of the tier discipline.
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        let mut out: Vec<ToolDescriptor> = self
            .tools
            .values()
            .map(|t| t.descriptor().clone())
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Registry containing only the named tools, sharing the same tool Arcs
    /// (M50: agent-type tool subsets — the child sees a filtered view, never
    /// copies). Unknown names are skipped, never an error: discovery data is
    /// advisory and one stale name must not break delegation.
    pub fn subset(&self, names: &[String]) -> ToolRegistry {
        let mut sub = ToolRegistry::new();
        for n in names {
            if let Some(t) = self.tools.get(n) {
                sub.tools.insert(n.clone(), std::sync::Arc::clone(t));
            }
        }
        sub
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::tool::{
        ExecutionMode, Tool, ToolContext, ToolFailure, ToolInvocation, ToolOutput, ToolUpdateSink,
    };

    /// A tool whose only job is to carry a name: registration order vs descriptor
    /// order is the whole point of these tests, so the descriptor is built once
    /// per fixture and leaked (test-process lifetime, a handful of names).
    struct Named(&'static ToolDescriptor);

    impl Named {
        fn new(name: &'static str) -> Self {
            Self(Box::leak(Box::new(ToolDescriptor {
                name: name.to_string(),
                label: name.to_string(),
                description: format!("{name} tool"),
                parameters_schema: serde_json::json!({"type": "object"}),
                execution_mode: ExecutionMode::Parallel,
            })))
        }
    }

    #[async_trait::async_trait]
    impl Tool for Named {
        fn descriptor(&self) -> &ToolDescriptor {
            self.0
        }

        async fn execute(
            &self,
            _call: ToolInvocation,
            _ctx: ToolContext,
            _sink: ToolUpdateSink,
        ) -> Result<ToolOutput, ToolFailure> {
            Ok(ToolOutput::text("ok"))
        }
    }

    #[test]
    fn descriptor_order_is_a_function_of_the_tool_set_not_of_hash_seeds() {
        // Registration order deliberately differs from alphabetical order: the
        // bug this guards against was a HashMap iteration order that changed per
        // process, which made the prompt's STABLE tier and every provider `tools`
        // array byte-different between two runs of the same session (and so made
        // provider prompt caching impossible across invocations).
        let mut reg = ToolRegistry::new();
        for name in ["write", "read", "glob", "grep", "bash", "edit"] {
            reg.register(Arc::new(Named::new(name)));
        }
        let names: Vec<String> = reg.descriptors().into_iter().map(|d| d.name).collect();
        assert_eq!(
            names,
            vec!["bash", "edit", "glob", "grep", "read", "write"],
            "descriptors must be sorted, not hash-ordered"
        );

        // A second, identically-built registry produces the same order.
        let mut again = ToolRegistry::new();
        for name in ["grep", "bash", "write", "edit", "read", "glob"] {
            again.register(Arc::new(Named::new(name)));
        }
        let names2: Vec<String> = again.descriptors().into_iter().map(|d| d.name).collect();
        assert_eq!(names, names2);
    }

    #[test]
    fn subset_keeps_only_known_names_and_shares_the_tool() {
        let mut reg = ToolRegistry::new();
        for name in ["read", "write", "bash"] {
            reg.register(Arc::new(Named::new(name)));
        }
        let sub = reg.subset(&["write".to_string(), "nope".to_string()]);
        assert!(sub.get("write").is_some());
        assert!(sub.get("read").is_none());
        assert!(
            sub.get("nope").is_none(),
            "unknown names are skipped, not errors"
        );
    }
}
