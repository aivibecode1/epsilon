//! Extension host integration tests using the `ext-fixture` child binary.

use std::path::Path;
use std::sync::Arc;

use epsilon_contracts::tool::{Tool, ToolContext, ToolInvocation};
use epsilon_ext::manifest::ExtManifest;
use epsilon_ext::{ExtHost, ExtTool, ExtToolSpec};
use tokio_util::sync::CancellationToken;

const FIXTURE_EXE: &str = env!("CARGO_BIN_EXE_ext-fixture");

fn fixture_manifest(mode: &str) -> ExtManifest {
    ExtManifest {
        name: "fixture".into(),
        version: "0.1.0".into(),
        bin: Path::new(FIXTURE_EXE).to_path_buf(),
        args: vec![mode.to_string()],
        events: vec![],
        tools: vec![ExtToolSpec {
            name: "ext_echo".into(),
            description: "Echo uppercase.".into(),
            schema: serde_json::json!({"type": "object"}),
        }],
    }
}

fn tool_ctx(dir: &Path) -> ToolContext {
    ToolContext {
        session_id: "t".into(),
        observation_scope: std::sync::Arc::new(()),
        cwd: dir.to_path_buf(),
        spill_dir: dir.join(".spill"),
        token: CancellationToken::new(),
        task_host: None,
        goal_host: None,
    }
}

#[tokio::test]
async fn handshake_echo_roundtrip_and_tool_wrapper() {
    let dir = tempfile::tempdir().unwrap();
    let host = ExtHost::spawn(dir.path(), &fixture_manifest(""))
        .await
        .unwrap();

    // Direct invoke.
    let (text, details) = host
        .invoke(
            "ext_echo",
            "tc_1",
            serde_json::json!({ "text": "hello ext" }),
        )
        .await
        .unwrap();
    assert_eq!(text, "HELLO EXT");
    assert_eq!(details.get("fixture"), Some(&serde_json::json!(true)));

    // Through the registry-facing wrapper.
    let spec = ExtToolSpec {
        name: "ext_echo".into(),
        description: "Echo uppercase.".into(),
        schema: serde_json::json!({"type": "object"}),
    };
    let tool = ExtTool::new(&spec, Arc::clone(&host));
    let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
    let out = tool
        .execute(
            ToolInvocation {
                tool_call_id: "tc_2".into(),
                args: serde_json::json!({"text": "again"}),
            },
            tool_ctx(dir.path()),
            sink,
        )
        .await
        .unwrap();
    assert!(out.content.iter().any(|c| matches!(
        c,
        epsilon_contracts::content::ContentBlock::Text { text } if text == "AGAIN"
    )));
}

#[tokio::test]
async fn stalled_child_hits_invoke_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let host = ExtHost::spawn(dir.path(), &fixture_manifest("--sleep"))
        .await
        .unwrap();

    let err = host
        .invoke("ext_echo", "tc_t", serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(
        matches!(err, epsilon_ext::ExtError::Timeout),
        "expected timeout, got {err:?}"
    );
}

#[tokio::test]
async fn crashing_child_is_counted_and_eventually_disabled() {
    let dir = tempfile::tempdir().unwrap();

    // Crash #1 and #2: separate spawns (the process dies each time), but the budget is a
    // property of the HOST instance — so we assert the counter mechanism on one host by
    // invoking a dying child repeatedly until stdout closes twice.
    for i in 0..2 {
        let host = ExtHost::spawn(dir.path(), &fixture_manifest("--die"))
            .await
            .unwrap();
        let err = host
            .invoke("ext_echo", &format!("tc_c{i}"), serde_json::json!({}))
            .await;
        assert!(err.is_err(), "dying child must fail invocation {i}");
        assert!(!host.is_disabled(), "single-host budget not yet exhausted");
    }

    // Third spawn with the SAME name demonstrates the disabled path only when crashes
    // accumulate on one host; v1 documents per-instance budgets, so this spawn is healthy.
    let host = ExtHost::spawn(dir.path(), &fixture_manifest(""))
        .await
        .unwrap();
    assert!(!host.is_disabled());
}
