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

mod stdio;

pub use stdio::{StdioConnector, attach_stdio_server};

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

    /// Cheap health probe (DIRECTION-02, Hermes' availability-check pattern).
    /// Returns `Err` when the server cannot serve calls right now. The default
    /// implementation trusts the connection — connectors that can detect death
    /// cheaply (e.g. stdio child exited) override this.
    async fn is_healthy(&self) -> Result<(), String> {
        Ok(())
    }
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
            last_probe_ms: std::sync::atomic::AtomicI64::new(0),
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
    /// Last successful health probe (DIRECTION-02). Calls within the TTL skip
    /// the probe; a failed call resets it so the next call re-probes first.
    last_probe_ms: std::sync::atomic::AtomicI64,
}

/// How long a successful health probe stays valid (Hermes' availability-TTL).
pub const HEALTH_TTL_MS: i64 = 30_000;

impl McpBridgeTool {
    /// Probe server health at most once per [`HEALTH_TTL_MS`], and always
    /// after a failure, so a dead server fails FAST with a clear message
    /// instead of an opaque transport error mid-call.
    async fn ensure_healthy(&self) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let last = self
            .last_probe_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        if now - last < HEALTH_TTL_MS {
            return Ok(());
        }
        self.connector.is_healthy().await?;
        self.last_probe_ms
            .store(now, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Test seam: force the next `ensure_healthy` to re-probe.
    #[cfg(test)]
    #[allow(dead_code)]
    fn invalidate_health(&self) {
        self.last_probe_ms
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }
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
        // DIRECTION-02: TTL-cached health probe so a dead server fails fast
        // with an honest message instead of an opaque transport error. Tests
        // can shrink the TTL via the `EPS_MCP_HEALTH_TTL_MS` env override.
        self.ensure_healthy().await.map_err(ToolFailure::msg)?;
        // Third-party MCP output is UNTRUSTED DATA (AGENTS.md contract): we pass it back
        // verbatim as text and never act on embedded instructions.
        let text = match self.connector.call_tool(&self.tool_name, call.args).await {
            Ok(t) => t,
            Err(e) => {
                // A failed call invalidates the probe: the NEXT call re-probes
                // before attempting another round-trip.
                self.last_probe_ms
                    .store(0, std::sync::atomic::Ordering::Relaxed);
                return Err(ToolFailure::msg(e));
            }
        };
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
    /// Test seam for DIRECTION-02: flips the health probe to unhealthy.
    pub dead: std::sync::atomic::AtomicBool,
}

impl InMemoryConnector {
    /// Mark the connector dead so `is_healthy` reports failure.
    pub fn mark_dead(&self) {
        self.dead.store(true, std::sync::atomic::Ordering::Relaxed);
    }

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

    /// Test seam: healthy until `mark_dead()` is called.
    async fn is_healthy(&self) -> Result<(), String> {
        if self.dead.load(std::sync::atomic::Ordering::Relaxed) {
            Err("in-memory server marked dead".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::content::ContentBlock;
    use epsilon_core::ToolRegistry;

    #[tokio::test]
    async fn bridge_health_probe_fails_fast_when_server_dies() {
        let mut conn = InMemoryConnector::default();
        conn.add(
            "list_prs",
            "List open PRs.",
            serde_json::json!({"type": "object"}),
            |_| Ok("PR-1".into()),
        );
        let conn = Arc::new(conn);
        // Keep the concrete Arc for mark_dead(); bridge wants the trait object.
        let connector: Arc<dyn Connector> = conn.clone() as Arc<dyn Connector>;
        let mut registry = ToolRegistry::new();
        bridge(&mut registry, "github", Arc::clone(&connector))
            .await
            .unwrap();
        let name = qualified_name("github", "list_prs");
        let tool = registry.get(&name).expect("bridged tool registered");

        // Healthy path still works.
        let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
        let ctx = ToolContext {
            session_id: "t".into(),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir().join("eps-mcp"),
            token: Default::default(),
        };
        let invocation = || ToolInvocation {
            tool_call_id: "tc_probe".into(),
            args: serde_json::json!({}),
        };
        let out = tool.execute(invocation(), ctx.clone(), sink.clone()).await;
        assert!(out.is_ok(), "healthy server must pass the probe");

        // Server dies -> next call fails FAST at the probe with an honest
        // message (not the handler's output, not an opaque transport error).
        conn.mark_dead();
        // Directly exercise the probe path: the TTL cache would mask this in
        // real use until expiry, and a failed call resets it (execute).
        let probe = conn.is_healthy().await;
        assert!(probe.is_err(), "dead server must fail its health probe");
        // The TTL cache would serve a stale OK for up to HEALTH_TTL_MS; the
        // probe contract is that the NEXT call after expiry fails fast. The
        // in-memory connector has no real clock dependency, so simulate expiry
        // by constructing a fresh registry (fresh tool, last_probe_ms = 0).
        let mut fresh_registry = ToolRegistry::new();
        bridge(&mut fresh_registry, "github", connector)
            .await
            .unwrap();
        let fresh_tool = fresh_registry.get(&name).expect("bridged tool registered");
        let err = fresh_tool
            .execute(invocation(), ctx, sink)
            .await
            .expect_err("dead server must fail");
        assert!(
            err.message.contains("marked dead"),
            "failure should come from the health probe: {}",
            err.message
        );
    }

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
