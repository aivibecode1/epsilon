//! Minimal MCP stdio server used by epsilon-mcp's offline connector tests.
//!
//! Speaks the real MCP protocol over real stdio so `StdioMcpConnector` is tested against
//! an actual rmcp peer — no network, no paid services, deterministic output.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt, transport::stdio};

/// Fixture handler exposing one `echo` tool.
struct FixtureServer;

impl ServerHandler for FixtureServer {
    fn get_info(&self) -> ServerInfo {
        // #[non_exhaustive]: default-construct, then mutate.
        let mut info = ServerInfo::default();
        info.instructions = Some("epsilon test fixture".into());
        info.server_info = Implementation::new("epsilon-fixture", env!("CARGO_PKG_VERSION"));
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let schema: serde_json::Value = serde_json::json!({
            "type": "object",
            "properties": { "text": { "type": "string" } },
            "required": ["text"]
        });
        Ok(ListToolsResult {
            next_cursor: None,
            tools: vec![Tool::new(
                "echo",
                "Echo back the provided text.",
                Arc::new(schema.as_object().cloned().expect("schema object")),
            )],
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        match request.name.as_ref() {
            "echo" => {
                let text = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                Ok(
                    CallToolResult::success(vec![ContentBlock::text(format!("echo: {text}"))])
                        .into(),
                )
            }
            other => Err(McpError::invalid_params(
                format!("unknown fixture tool: {other}"),
                None,
            )),
        }
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("fixture runtime");
    runtime.block_on(async move {
        let service = FixtureServer
            .serve(stdio())
            .await
            .expect("fixture serve over stdio");
        service.waiting().await.expect("fixture service task");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The handler must at least describe itself; full wire coverage lives in the
    /// connector integration test which spawns this binary.
    #[test]
    fn info_reports_fixture_identity() {
        let info = FixtureServer.get_info();
        assert_eq!(info.server_info.name, "epsilon-fixture");
        assert!(info.instructions.is_some());
    }
}
