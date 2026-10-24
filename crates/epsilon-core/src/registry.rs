//! Tool registry: name -> tool, plus descriptor export for provider payloads.
//!
//! **Interior mutability is deliberate.** The registry is owned behind an
//! `Arc` by the running agent, and a skill reload has to replace one tool
//! without rebuilding the agent (or the session it is attached to). A
//! `&mut self` API made that impossible; an `RwLock` costs one uncontended read
//! per tool lookup and lets composition and `/reload` use the same registry.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use epsilon_contracts::tool::{Tool, ToolDescriptor};

/// Simple name-keyed registry. Duplicate names replace the old entry (last registration
/// wins), matching pi's extension-override behavior; collisions are visible via the
/// return value.
#[derive(Default)]
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
}

impl ToolRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool through a mutable handle (the composition/build shape).
    ///
    /// Kept as `&mut self` so every existing caller and test fixture compiles
    /// unchanged; [`Self::insert`] is the same operation on a shared registry.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Option<String> {
        self.insert(tool)
    }

    /// Register or replace a tool on a shared registry.
    ///
    /// Returns the displaced tool's name if one existed. A poisoned lock is
    /// recovered through `into_inner`: a single failed tool registration must
    /// not turn every later lookup into "no such tool".
    pub fn insert(&self, tool: Arc<dyn Tool>) -> Option<String> {
        let name = tool.descriptor().name.clone();
        let mut guard = self
            .tools
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .insert(name.clone(), tool)
            .map(|old| old.descriptor().name.clone())
    }

    /// Remove a tool by name, if present.
    pub fn remove(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(name)
    }

    /// Look up by tool name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(name)
            .cloned()
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
        let guard = self
            .tools
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut out: Vec<ToolDescriptor> = guard.values().map(|t| t.descriptor().clone()).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Registry containing only the named tools, sharing the same tool Arcs
    /// (M50: agent-type tool subsets — the child sees a filtered view, never
    /// copies). Unknown names are skipped, never an error: discovery data is
    /// advisory and one stale name must not break delegation.
    pub fn subset(&self, names: &[String]) -> ToolRegistry {
        let source = self
            .tools
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sub = ToolRegistry::new();
        {
            let mut out = sub
                .tools
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for n in names {
                if let Some(t) = source.get(n) {
                    out.insert(n.clone(), Arc::clone(t));
                }
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

    /// A shared registry can replace and remove one tool without touching the
    /// others — the operation `/reload` needs for the `skill` tool.
    #[test]
    fn a_shared_registry_can_replace_and_remove_one_tool() {
        let reg = Arc::new(ToolRegistry::new());
        reg.insert(Arc::new(Named::new("skill")));
        reg.insert(Arc::new(Named::new("read")));
        assert_eq!(
            reg.insert(Arc::new(Named::new("skill"))),
            Some("skill".into())
        );
        assert_eq!(reg.descriptors().len(), 2);
        assert!(reg.remove("skill").is_some());
        assert!(reg.get("skill").is_none());
        assert!(reg.get("read").is_some());
    }
}
