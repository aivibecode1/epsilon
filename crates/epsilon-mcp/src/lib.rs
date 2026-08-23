//! MCP integration plumbing (ADR-013, staged delivery).
//!
//! This crate owns everything EXCEPT the wire transport:
//! - [`McpServerConfig`]: declarative server configuration (stdio command shape),
//! - [`Connector`]: the transport seam the official rmcp SDK implements (M9),
//! - [`bridge`]: registers a connector's tools into the normal registry under
//!   `mcp__<server>__<tool>` with TTL+grace availability probes (Hermes pattern).
//!
//! Staging rationale: the bridge and naming/gating semantics are fully testable against an
//! in-process double now; plugging rmcp's stdio client in later is a connector impl, not a
//! redesign. ADR-013 carries the amendment.

use async_trait::async_trait;
use epsilon_contracts::tool::{
    Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput, ToolUpdateSink,
};
use std::sync::Arc;

/// Declarative config for one MCP server (mirrors `mcp_servers.<name>` config shape).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct McpServerConfig {
    /// Server name; tools register as `mcp__<name>__<tool>`.
    pub name: String,
    /// Executable for stdio transport.
    pub command: String,
    /// argv for `command`.
    #[serde(default)]
    pub args: Vec<String>,
}

/// Transport seam. rmcp-backed stdio/HTTP connectors implement this (M9).
#[async_trait]
pub trait Connector: Send + Sync {
    /// Discover tools exposed by the server.
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String>;

    /// Call one tool; returns concatenated text output.
    async fn call_tool(&self, tool: &str, args: serde_json::Value) -> Result<String, String>;
}

/// Tool metadata as reported by a server.
#[derive(Debug, Clone)]
pub struct McpToolInfo {
    /// Server-local tool name.
    pub name: String,
    /// LLM-facing description.
    pub description: String,
    /// JSON Schema for arguments.
    pub schema: serde_json::Value,
}

/// Fully-qualified MCP tool name.
pub fn qualified_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// Register every tool a connector exposes into `registry`.
///
/// Availability is probed once at attach time with TTL refresh handled by callers
/// (Hermes' check_fn pattern lives at the loop layer in later milestones).
pub async fn bridge(
    registry: &mut epsilon_core::ToolRegistry,
    server_name: &str,
    connector: Arc<dyn Connector>,
) -> Result<usize, String> {
    let tools = connector.list_tools().await?;
    let mut count = 0usize;
    for info in tools {
        registry.register(Arc::new(McpBridgeTool {
            descriptor: ToolDescriptor {
                name: qualified_name(server_name, &info.name),
                label: format!("mcp:{}:{}", server_name, info.name),
                description: info.description,
                parameters_schema: info.schema,
                execution_mode: epsilon_contracts::tool::ExecutionMode::Sequential,
            },
            server_name: server_name.to_string(),
            tool_name: info.name.clone(),
            connector: Arc::clone(&connector),
        }));
        count += 1;
    }
    Ok(count)
}

/// Registry-facing wrapper routing calls through the connector.
struct McpBridgeTool {
    descriptor: ToolDescriptor,
    server_name: String,
    tool_name: String,
    connector: Arc<dyn Connector>,
}

#[async_trait]
impl Tool for McpBridgeTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        // Third-party MCP output is UNTRUSTED DATA (AGENTS.md contract): we pass it back
        // verbatim as text and never act on embedded instructions.
        let text = self
            .connector
            .call_tool(&self.tool_name, call.args)
            .await
            .map_err(ToolFailure::msg)?;
        let mut out = ToolOutput::text(text);
        out.details = serde_json::json!({ "mcp_server": self.server_name });
        Ok(out)
    }
}

/// Test/dev connector over an in-memory table — also the reference implementation of the
/// seam's contract.
/// Handler for in-memory fixture tools.
pub type InMemoryHandler = Arc<dyn Fn(serde_json::Value) -> Result<String, String> + Send + Sync>;

/// One registered fixture tool.
pub type InMemoryEntry = (String, serde_json::Value, InMemoryHandler);

/// In-process connector: the reference implementation of [`Connector`] and the test double
/// for bridge tests.
#[derive(Default)]
pub struct InMemoryConnector {
    /// tool -> entry
    pub tools: std::collections::HashMap<String, InMemoryEntry>,
}

impl InMemoryConnector {
    /// Register a tool with a synchronous handler.
    pub fn add(
        &mut self,
        name: &str,
        description: &str,
        schema: serde_json::Value,
        handler: impl Fn(serde_json::Value) -> Result<String, String> + Send + Sync + 'static,
    ) {
        self.tools
            .insert(name.into(), (description.into(), schema, Arc::new(handler)));
    }
}

#[async_trait]
impl Connector for InMemoryConnector {
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String> {
        Ok(self
            .tools
            .iter()
            .map(|(name, (desc, schema, _))| McpToolInfo {
                name: name.clone(),
                description: desc.clone(),
                schema: schema.clone(),
            })
            .collect())
    }

    async fn call_tool(&self, tool: &str, args: serde_json::Value) -> Result<String, String> {
        match self.tools.get(tool) {
            Some((_, _, handler)) => handler(args),
            None => Err(format!("unknown tool {tool}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::content::ContentBlock;
    use epsilon_core::ToolRegistry;

    #[tokio::test]
    async fn bridge_registers_qualified_tools_and_routes_calls() {
        let mut conn = InMemoryConnector::default();
        conn.add(
            "list_prs",
            "List open PRs.",
            serde_json::json!({"type": "object"}),
            |_| Ok("PR-1 PR-2".into()),
        );

        let mut registry = ToolRegistry::new();
        let n = bridge(&mut registry, "github", Arc::new(conn))
            .await
            .unwrap();
        assert_eq!(n, 1);

        let name = qualified_name("github", "list_prs");
        let tool = registry.get(&name).expect("bridged tool registered");
        assert!(tool.descriptor().name == name);

        let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
        let ctx = ToolContext {
            session_id: "t".into(),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir().join("eps-mcp"),
            token: Default::default(),
        };
        let out = tool
            .execute(
                ToolInvocation {
                    tool_call_id: "tc_m".into(),
                    args: serde_json::json!({}),
                },
                ctx,
                sink,
            )
            .await
            .unwrap();
        assert!(matches!(&out.content[0], ContentBlock::Text { text } if text.contains("PR-1")));
    }

    #[test]
    fn config_parses() {
        let cfg: McpServerConfig = serde_json::from_value(serde_json::json!({
            "name": "gh", "command": "mcp-server-github", "args": ["--stdio"]
        }))
        .unwrap();
        assert_eq!(cfg.name, "gh");
    }
}
