//! The whole loop against a real child process: spawn, handshake, document sync, publish.
//!
//! Unit tests can prove the report's shape and the ladder of decisions; only a spawned process can
//! prove the framing, the handshake, the wait-for-publish and the broken-server bookkeeping. The
//! server is `lsp_fixture_server` from this crate — a real LSP process whose diagnostics rule is
//! "one error per `TODO_BAD`", so the assertions are about behaviour the test controls rather than
//! about a specific toolchain being installed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use epsilon_lsp::{DiagnosticsService, ServiceConfig};

/// A config whose "rust-analyzer" is the fixture, with the start-log wired when asked.
fn fixture_config(log: Option<&Path>) -> ServiceConfig {
    let mut cfg = ServiceConfig::default();
    // The fixture is the server for `.rs` files (the rust-analyzer definition is what select_server
    // returns for that extension; only the executable is replaced).
    cfg.commands.insert(
        "rust-analyzer",
        PathBuf::from(env!("CARGO_BIN_EXE_lsp_fixture_server")),
    );
    cfg.publish_timeout = Duration::from_secs(10);
    cfg.handshake_timeout = Duration::from_secs(10);
    if let Some(path) = log {
        // Given to the child, not to this process: two tests running at once must not fight over
        // one environment variable.
        cfg.env.push((
            "EPSILON_LSP_FIXTURE_LOG".to_string(),
            path.display().to_string(),
        ));
    }
    cfg
}

/// A definition whose fixture variant refuses to handshake.
fn refusing_def() -> &'static epsilon_lsp::ServerDef {
    Box::leak(Box::new(epsilon_lsp::ServerDef {
        id: "rust-analyzer",
        name: "fixture (refuses)",
        extensions: &["rs"],
        command: "lsp_fixture_server",
        args: &["--refuse"],
        root_markers: &["Cargo.toml"],
    }))
}

/// A definition whose fixture variant handshakes and then never publishes.
fn silent_def() -> &'static epsilon_lsp::ServerDef {
    Box::leak(Box::new(epsilon_lsp::ServerDef {
        id: "rust-analyzer",
        name: "fixture (silent)",
        extensions: &["rs"],
        command: "lsp_fixture_server",
        args: &["--silent"],
        root_markers: &["Cargo.toml"],
    }))
}

/// A workspace with a Cargo.toml marker and one `.rs` file whose text the caller chooses.
fn workspace(text: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let file = src.join("main.rs");
    std::fs::write(&file, text).unwrap();
    (dir, file)
}

#[tokio::test]
async fn an_edit_that_broke_the_file_comes_back_as_a_bounded_block() {
    let (_dir, file) = workspace("fn main() {\n    let x = 1;\n    TODO_BAD\n}\n");
    let service = DiagnosticsService::new(fixture_config(None));
    let text = std::fs::read_to_string(&file).unwrap();

    let block = service
        .after_change(&file, &text)
        .await
        .expect("the server reports an error");
    assert!(block.contains("[lsp] main.rs: 1 error"), "{block}");
    assert!(
        block.contains("ERROR [3:5] found a TODO_BAD marker"),
        "{block}"
    );
    service.shutdown().await;
}

#[tokio::test]
async fn a_clean_file_produces_no_block_at_all() {
    let (_dir, file) = workspace("fn main() {}\n");
    let service = DiagnosticsService::new(fixture_config(None));
    assert_eq!(service.after_change(&file, "fn main() {}\n").await, None);
    service.shutdown().await;
}

#[tokio::test]
async fn a_second_change_reuses_the_running_server_and_is_seen() {
    let (_dir, file) = workspace("fn main() {}\n");
    let log = tempfile::NamedTempFile::new().unwrap();
    let service = DiagnosticsService::new(fixture_config(Some(log.path())));

    // First change: clean.
    assert_eq!(service.after_change(&file, "fn main() {}\n").await, None);
    // Second change: the same client must send `didChange`, and the new error must come back.
    let block = service
        .after_change(&file, "fn main() {\n    TODO_BAD\n}\n")
        .await
        .expect("the second change is reported");
    assert!(block.contains("ERROR [2:5]"), "{block}");

    let starts = std::fs::read_to_string(log.path()).unwrap_or_default();
    assert_eq!(
        starts.lines().count(),
        1,
        "one server for both changes, not two: {starts}"
    );
    service.shutdown().await;
}

#[tokio::test]
async fn a_server_that_never_publishes_is_silence_not_a_hang() {
    // A real language server mid-index publishes nothing for a while; the wait must elapse into
    // "nothing to say" rather than an error or a hang.
    let (_dir, file) = workspace("TODO_BAD\n");
    let mut cfg = fixture_config(None);
    cfg.def_override = Some(silent_def());
    cfg.publish_timeout = Duration::from_millis(300);
    let service = DiagnosticsService::new(cfg);
    assert_eq!(service.after_change(&file, "TODO_BAD\n").await, None);
    service.shutdown().await;
}

#[tokio::test]
async fn a_server_that_refuses_to_handshake_is_broken_once_and_never_retried() {
    let (_dir, file) = workspace("TODO_BAD\n");
    let log = tempfile::NamedTempFile::new().unwrap();
    let mut cfg = fixture_config(Some(log.path()));
    cfg.def_override = Some(refusing_def());
    let service = DiagnosticsService::new(cfg);

    assert_eq!(service.after_change(&file, "TODO_BAD\n").await, None);
    assert_eq!(service.after_change(&file, "TODO_BAD\n").await, None);
    assert_eq!(service.after_change(&file, "TODO_BAD\n").await, None);

    let starts = std::fs::read_to_string(log.path()).unwrap_or_default();
    assert_eq!(
        starts.lines().count(),
        1,
        "a server that cannot handshake is not respawned per edit: {starts}"
    );
    service.shutdown().await;
}
