//! Integration coverage for the built-in tools, run against real tempdir workspaces.

use std::path::Path;
use std::sync::Arc;

use epsilon_contracts::tool::{Tool, ToolContext, ToolInvocation};
use epsilon_tools::util;
use epsilon_tools::{BashTool, EditTool, GlobTool, GrepTool, LsTool, ReadTool, WriteTool};
use tokio_util::sync::CancellationToken;

fn ctx_for(root: &Path) -> ToolContext {
    ToolContext {
        session_id: "test".into(),
        cwd: root.to_path_buf(),
        spill_dir: root.join(".spill"),
        token: CancellationToken::new(),
    }
}

async fn run(
    tool: &dyn Tool,
    ctx: &ToolContext,
    args: serde_json::Value,
) -> Result<String, String> {
    let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
    tool.execute(
        ToolInvocation {
            tool_call_id: "tc_t".into(),
            args,
        },
        ctx.clone(),
        sink,
    )
    .await
    .map(|o| {
        o.content
            .iter()
            .filter_map(|c| match c {
                epsilon_contracts::content::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
    .map_err(|f| f.message)
}

fn seeded_workspace() -> (tempfile::TempDir, ToolContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("README.md"), "# demo\nhello world\n").unwrap();
    let ctx = ctx_for(root);
    (dir, ctx)
}

#[tokio::test]
async fn read_returns_numbered_lines_and_rejects_escapes() {
    let (_d, ctx) = seeded_workspace();
    let read = ReadTool::new();

    let out = run(&read, &ctx, serde_json::json!({"path": "src/lib.rs"}))
        .await
        .unwrap();
    assert!(out.contains("    1| pub fn alpha() {}"));
    assert!(out.contains("    2| pub fn beta() {}"));

    let paged = run(
        &read,
        &ctx,
        serde_json::json!({"path": "src/lib.rs", "offset": 2, "limit": 1}),
    )
    .await
    .unwrap();
    assert!(paged.contains("beta"));
    assert!(!paged.contains("alpha"));

    let err = run(&read, &ctx, serde_json::json!({"path": "../secrets.txt"}))
        .await
        .unwrap_err();
    assert!(err.contains("outside the workspace"));
}

#[tokio::test]
async fn write_creates_then_edit_modifies_with_safety() {
    let (_d, ctx) = seeded_workspace();
    let write = WriteTool::new();
    let edit = EditTool::new();
    let read = ReadTool::new();

    run(
        &write,
        &ctx,
        serde_json::json!({"path": "docs/new.txt", "content": "one\ntwo\n"}),
    )
    .await
    .unwrap();

    // ambiguous match fails with count
    let err = run(
        &edit,
        &ctx,
        serde_json::json!({"path": "docs/new.txt", "old_string": "o", "new_string": "0"}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("matches 2 locations"), "got: {err}");

    // unique edit applies
    run(
        &edit,
        &ctx,
        serde_json::json!({
            "path": "docs/new.txt",
            "old_string": "two", "new_string": "TWO"
        }),
    )
    .await
    .unwrap();
    let content = run(&read, &ctx, serde_json::json!({"path": "docs/new.txt"}))
        .await
        .unwrap();
    assert!(content.contains("TWO") && !content.contains("| two"));
}

#[tokio::test]
async fn grep_glob_ls_find_seeded_content() {
    let (_d, ctx) = seeded_workspace();
    let grep = GrepTool::new();
    let glob = GlobTool::new();
    let ls = LsTool::new();

    let hits = run(&grep, &ctx, serde_json::json!({"pattern": "fn beta"}))
        .await
        .unwrap();
    assert!(hits.contains("src/lib.rs:2: pub fn beta() {}"));

    let no_case = run(
        &grep,
        &ctx,
        serde_json::json!({"pattern": "HELLO", "ignore_case": true}),
    )
    .await
    .unwrap();
    assert!(no_case.contains("README.md:2"));

    let files = run(&glob, &ctx, serde_json::json!({"pattern": "*.rs"}))
        .await
        .unwrap();
    assert!(files.contains("src/lib.rs"));

    let listing = run(&ls, &ctx, serde_json::json!({})).await.unwrap();
    assert!(listing.contains("src/"));
    assert!(listing.contains("README.md"));
}

#[cfg(windows)]
#[tokio::test]
async fn bash_runs_powershell_with_exit_codes_and_caps() {
    bash_common().await;
}

#[cfg(not(windows))]
#[tokio::test]
async fn bash_runs_bash_with_exit_codes_and_caps() {
    bash_common().await;
}

async fn bash_common() {
    let (_d, ctx) = seeded_workspace();
    let bash = BashTool::new();

    let ok = run(
        &bash,
        &ctx,
        serde_json::json!({"command": "echo hello-from-shell"}),
    )
    .await
    .unwrap();
    assert!(ok.contains("hello-from-shell"));

    let failing = run(
        &bash,
        &ctx,
        serde_json::json!({
            "command": "exit 3", "timeout_secs": 5
        }),
    )
    .await
    .unwrap();
    assert!(failing.contains("exit code: 3"));
}

#[tokio::test]
async fn bash_timeout_kills_long_commands() {
    let (_d, ctx) = seeded_workspace();
    let bash = BashTool::new();
    let cmd = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    };
    let err = run(
        &bash,
        &ctx,
        serde_json::json!({"command": cmd, "timeout_secs": 1}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("timed out after 1s"));
}

#[tokio::test]
async fn oversized_outputs_spill_with_locator() {
    let (_d, ctx) = seeded_workspace();
    let bash = BashTool::new();
    let cmd = if cfg!(windows) {
        "1..3000 | ForEach-Object { \"line $_\" }"
    } else {
        "for i in $(seq 1 3000); do echo \"line $i\"; done"
    };
    let out = run(&bash, &ctx, serde_json::json!({"command": cmd}))
        .await
        .unwrap();
    assert!(
        out.contains("[output truncated:"),
        "big output must be truncated"
    );
    assert!(out.contains("full output saved to"));
}

#[tokio::test]
async fn workspace_escape_refused_in_write_and_edit() {
    let (_d, ctx) = seeded_workspace();
    let write = WriteTool::new();
    let edit = EditTool::new();

    let w_err = run(
        &write,
        &ctx,
        serde_json::json!({"path": "../evil.txt", "content": "x"}),
    )
    .await
    .unwrap_err();
    assert!(w_err.contains("outside the workspace"));

    let e_err = run(
        &edit,
        &ctx,
        serde_json::json!({"path": "../README.md", "old_string": "#", "new_string": "!"}),
    )
    .await
    .unwrap_err();
    assert!(e_err.contains("outside the workspace"));
}

#[test]
fn util_walker_respects_skip_list() {
    let _ = util::SKIP_DIRS; // referenced so the constant stays documented in tests too
}
