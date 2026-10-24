//! Integration coverage for the rmcp stdio transport (ADR-013 amendment).
//!
//! The fixture binary (`mcp-fixture-server`) is a REAL MCP server speaking the real
//! protocol over real stdio pipes; these tests spawn it, bridge its tools into a normal
//! registry, and call one end-to-end — offline, deterministic, free.
//!
//! The fixture's `[[bin]]` declares `required-features = ["rmcp/server", …]`; the
//! `[dev-dependencies]` entry enables exactly those whenever any test target builds,
//! so plain `cargo test --workspace` compiles and runs this file.

use std::sync::Arc;

use epsilon_core::ToolRegistry;
use epsilon_mcp::{
    CALL_TIMEOUT_SECS, Connector, McpServerConfig, StdioConnector, attach_stdio_server,
    qualified_name,
};

fn fixture_config() -> McpServerConfig {
    McpServerConfig {
        name: "fixture".into(),
        command: env!("CARGO_BIN_EXE_mcp-fixture-server").into(),
        args: vec![],
        timeout_secs: CALL_TIMEOUT_SECS,
    }
}

#[tokio::test]
async fn stdio_connector_discovers_and_calls_fixture_tools() {
    let mut registry = ToolRegistry::new();
    let bridged = attach_stdio_server(&mut registry, &fixture_config(), None)
        .await
        .expect("bridge fixture server");
    assert_eq!(bridged, 1, "fixture exposes exactly one tool");

    let name = qualified_name("fixture", "echo");
    let tool = registry.get(&name).expect("bridged tool present");

    let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
    let ctx = epsilon_contracts::tool::ToolContext {
        session_id: "stdio-test".into(),
        observation_scope: std::sync::Arc::new(()),
        cwd: std::env::temp_dir(),
        spill_dir: std::env::temp_dir().join("eps-mcp-stdio"),
        token: Default::default(),
        task_host: None,
        goal_host: None,
    };
    let out = tool
        .execute(
            epsilon_contracts::tool::ToolInvocation {
                tool_call_id: "tc_stdio".into(),
                args: serde_json::json!({ "text": "over-stdio" }),
            },
            ctx,
            sink,
        )
        .await
        .expect("echo call succeeds");
    assert!(
        matches!(&out.content[0], epsilon_contracts::content::ContentBlock::Text { text }
            if text.contains("echo: over-stdio")),
        "round-tripped text must come back, got {:?}",
        out.content
    );
}

#[tokio::test]
async fn unknown_tool_surfaces_as_failure_not_panic() {
    let connector = StdioConnector::spawn(&fixture_config(), None)
        .await
        .expect("connect");
    let err = connector
        .call_tool("does_not_exist", serde_json::json!({}))
        .await
        .expect_err("unknown tool must fail in-band");
    assert!(
        err.contains("does_not_exist") || err.to_lowercase().contains("unknown"),
        "{err}"
    );
    connector.shutdown().await.expect("graceful shutdown");
}
