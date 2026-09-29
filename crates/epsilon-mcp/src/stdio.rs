//! The real [`Connector`]: an MCP client over rmcp 3.1's stdio (child-process) transport.
//!
//! ADR-013 amendment: this replaces the "staged" in-process-only state from M8. rmcp types
//! stay inside this module (adapter rule); the rest of the crate still sees only the
//! transport-agnostic [`Connector`] seam and [`McpToolInfo`].

use std::sync::Arc;

use async_trait::async_trait;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo, ContentBlock, Implementation};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use tokio::process::Command;

use crate::{Connector, McpServerConfig, McpToolInfo};

/// Connector speaking MCP over a spawned child process' stdin/stdout.
///
/// The child is killed when the connector drops (rmcp kill-on-drop); call
/// [`StdioConnector::shutdown`] for graceful close.
pub struct StdioConnector {
    service: RunningService<RoleClient, ClientInfo>,
}

impl StdioConnector {
    /// Spawn `command args…` and complete the MCP initialize handshake.
    pub async fn spawn(cfg: &McpServerConfig) -> Result<Self, String> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args);
        // An MCP server is a stdio peer, not a terminal program. Leaving stdin unset
        // means INHERIT, and in the TUI that is the console the user is typing into:
        // the server competes with crossterm for keystrokes, which is the reported
        // "frozen" symptom. rmcp's transport needs a pipe, but Rust's default is the
        // parent's handle, so say so explicitly. Same rule as
        // `epsilon_tools::shell::ChildStdio`.
        cmd.stdin(std::process::Stdio::piped());
        let transport =
            TokioChildProcess::new(cmd).map_err(|e| format!("spawn {}: {e}", cfg.command))?;

        // ClientInfo/Implementation are #[non_exhaustive]: default-construct, then mutate.
        let mut info = ClientInfo::default();
        info.client_info = Implementation::new("epsilon", env!("CARGO_PKG_VERSION"));

        let service = info
            .serve(transport)
            .await
            .map_err(|e| format!("mcp initialize with {}: {e}", cfg.command))?;
        Ok(Self { service })
    }

    /// Close the transport and wait for the child to exit (kill after rmcp's grace).
    pub async fn shutdown(self) -> Result<(), String> {
        match self.service.cancel().await {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("mcp service shutdown: {e}")),
        }
    }
}

#[async_trait]
impl Connector for StdioConnector {
    /// DIRECTION-02: a dead stdio child means every call would hang or fail
    /// with an opaque transport error; report that honestly and cheaply.
    async fn is_healthy(&self) -> Result<(), String> {
        if self.service.is_closed() {
            return Err("mcp server process is not running (exited or crashed); \
                 restart the session or re-attach with --mcp"
                .to_string());
        }
        Ok(())
    }
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String> {
        let tools = self
            .service
            .list_all_tools()
            .await
            .map_err(|e| format!("tools/list: {e}"))?;
        Ok(tools
            .into_iter()
            .map(|t| McpToolInfo {
                name: t.name.to_string(),
                description: t.description.map(|d| d.to_string()).unwrap_or_default(),
                schema: serde_json::Value::Object((*t.input_schema).clone()),
            })
            .collect())
    }

    async fn call_tool(&self, tool: &str, args: serde_json::Value) -> Result<String, String> {
        let Some(map) = args.as_object().cloned() else {
            return Err("MCP tool arguments must be a JSON object".into());
        };
        let mut params = CallToolRequestParams::new(tool.to_string());
        params.arguments = Some(map);

        let result = self
            .service
            .call_tool(params)
            .await
            .map_err(|e| format!("tools/call {tool}: {e}"))?;

        let mut text = String::new();
        for block in &result.content {
            if let ContentBlock::Text(t) = block {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t.text);
            }
        }
        if result.is_error.unwrap_or(false) {
            return Err(if text.is_empty() {
                format!("tool {tool} failed")
            } else {
                text
            });
        }
        // Structured output without text content still carries something useful.
        if text.is_empty()
            && let Some(sc) = result.structured_content.as_ref()
        {
            text = sc.to_string();
        }
        Ok(text)
    }
}

/// Convenience: spawn a stdio connector and bridge its tools into `registry`.
pub async fn attach_stdio_server(
    registry: &mut epsilon_core::ToolRegistry,
    cfg: &McpServerConfig,
) -> Result<usize, String> {
    let connector: Arc<dyn Connector> = Arc::new(StdioConnector::spawn(cfg).await?);
    crate::bridge(registry, &cfg.name, connector).await
}
