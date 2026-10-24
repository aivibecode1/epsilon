//! M76: `epsilon doctor`'s gathering, run against real files.
//!
//! The unit tests in `doctor.rs` cover the rendering. These cover the part that can be wrong in a
//! way a user would feel: whether the report actually reflects the workspace it was pointed at —
//! the session that exists, the instructions that exist, the lease that is held.
//!
//! The model/credential lines are deliberately **not** asserted here: they read the machine's
//! real global config and auth store, which a test must not depend on (and must not fake, because
//! a fake would prove nothing about what a user sees).

use std::path::Path;

use epsilon_cli::doctor::{Inputs, gather};
use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::message::{Message, ToolResultMessage, UserMessage};
use epsilon_contracts::session_entry::EntryKind;
use epsilon_session::SessionStore;

fn inputs(ws: &Path) -> Inputs<'_> {
    Inputs {
        workspace: ws,
        model_arg: None,
        confinement: &epsilon_tools::Confinement::Auto,
        mcp_server_count: 0,
        real_feature: true,
    }
}

/// A workspace that looks like a project the agent has run in: a git dir, instructions, sessions.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::create_dir_all(dir.path().join(".epsilon/sessions")).unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "project rules").unwrap();
    dir
}

/// A session file whose header points at `ws`, with one user turn and one failed tool result.
fn seed_session(ws: &Path, name: &str) -> std::path::PathBuf {
    let path = ws.join(".epsilon/sessions").join(name);
    let mut store = SessionStore::open_in(&path, ws).unwrap();
    store
        .append(EntryKind::Message {
            message: Message::User(UserMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                content: vec![epsilon_contracts::content::UserContent::Text {
                    text: "run the tests".into(),
                }],
            }),
            api_request_sha256: None,
        })
        .unwrap();
    store
        .append(EntryKind::Message {
            message: Message::ToolResult(ToolResultMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "call_1".into(),
                tool_name: "bash".into(),
                content: vec![ContentBlock::text("cargo test failed: 3 assertions")],
                details: serde_json::json!({}),
                usage: None,
                is_error: true,
            }),
            api_request_sha256: None,
        })
        .unwrap();
    // The lease is released here, so the report sees a free session — which is the normal state
    // for a session nobody is running.
    drop(store);
    path
}

/// **A diagnostic must not mutate the state it reports.** `doctor` used to
/// call the launch loader twice; the first call recorded first sight and the
/// second read hash-without-memory, so a first-run workspace was reported
/// untrusted and the next launch prompted.
#[test]
fn the_report_does_not_record_project_trust() {
    let dir = workspace();
    std::fs::write(
        dir.path().join(".epsilon/config.toml"),
        "[model]\nid = \"deepseek-flash\"\n",
    )
    .unwrap();
    assert!(!dir.path().join(".epsilon/trust.json").exists());
    let f = gather(&inputs(dir.path()));
    assert!(
        f.project_trusted.unwrap_or(false),
        "first sight is the launch's Trusted row, not Changed"
    );
    assert!(
        !dir.path().join(".epsilon/trust.json").exists(),
        "doctor must not create a trust file"
    );
}

#[test]
fn the_report_describes_the_workspace_it_was_pointed_at() {
    let dir = workspace();
    seed_session(dir.path(), "tui.jsonl");

    let f = gather(&inputs(dir.path()));

    assert!(f.is_git_repo, "the .git dir is there");
    assert!(
        f.instructions.iter().any(|(n, _)| n == "AGENTS.md"),
        "the root instructions are found: {:?}",
        f.instructions
    );
    assert!(
        f.session_newest.is_some(),
        "the session file is found: {:?}",
        f.session_newest
    );
    assert_eq!(f.session_entries, Some(2), "two entries were appended");
    assert!(
        f.recent_failures
            .iter()
            .any(|r| r.contains("cargo test failed")),
        "the failed tool result is listed: {:?}",
        f.recent_failures
    );
    assert!(
        f.session_lease.is_none(),
        "no lock file, no lease: {:?}",
        f.session_lease
    );
    // The tool list is real work: the built-in registry with the effective confinement.
    assert!(f.tools.contains(&"bash".to_string()), "{:?}", f.tools);
    assert!(f.tools.contains(&"edit".to_string()), "{:?}", f.tools);
}

#[test]
fn a_held_lease_is_a_blocker_with_a_nonzero_exit() {
    let dir = workspace();
    let session = seed_session(dir.path(), "tui.jsonl");
    // What a live process leaves behind: the lock sibling with its identity line (M61).
    std::fs::write(
        session.with_extension("jsonl.lock"),
        "pid=99999 host=test\n",
    )
    .unwrap();

    let f = gather(&inputs(dir.path()));

    assert_eq!(
        f.session_lease.as_deref(),
        Some("pid=99999 host=test"),
        "the holder is reported verbatim"
    );
    assert!(
        f.blockers.iter().any(|b| b.contains("write lease")),
        "a held lease is what a user most often hits after a crash: {:?}",
        f.blockers
    );
    assert_eq!(f.exit_code(), 1, "blockers are a non-zero exit");
}

/// The lease lookup reads `<session>.lock` — the sibling name M61's store writes. Pinned here
/// because a wrong name would silently report every session as free.
#[test]
fn the_lease_is_read_from_the_lock_file_beside_the_session() {
    let dir = workspace();
    let session = seed_session(dir.path(), "tui.jsonl");
    let lock = std::path::PathBuf::from(format!("{}.lock", session.display()));
    std::fs::write(&lock, "pid=4242\n").unwrap();

    let f = gather(&inputs(dir.path()));
    assert_eq!(
        f.session_lease.as_deref(),
        Some("pid=4242"),
        "the lock sibling is what a held session looks like on disk"
    );

    // And removing it frees the session again: nothing is cached between gathers.
    std::fs::remove_file(&lock).unwrap();
    let after = gather(&inputs(dir.path()));
    assert!(after.session_lease.is_none(), "{:?}", after.session_lease);
}

#[test]
fn a_workspace_without_sessions_says_so_instead_of_guessing() {
    let dir = tempfile::tempdir().unwrap();
    // No .git, no instructions, no session dir at all.
    let f = gather(&inputs(dir.path()));
    assert!(!f.is_git_repo);
    assert!(f.instructions.is_empty());
    assert!(f.session_newest.is_none());
    assert!(f.session_lease.is_none());
    assert!(f.recent_failures.is_empty());
    assert_eq!(f.session_entries, None, "nothing to count");
    let out = f.render();
    assert!(out.contains("git repository     no"), "{out}");
    assert!(out.contains("newest             none"), "{out}");
    assert!(
        !out.contains("recent failures"),
        "an empty session list must not print an empty section: {out}"
    );
}

#[test]
fn a_stale_lock_file_is_still_reported_because_liveness_cannot_be_known() {
    // The honest limit: a lock file whose pid is long gone looks exactly like a live one from
    // here. The report says "held" and tells the user how to clear it rather than pretending to
    // know the process is alive.
    let dir = workspace();
    let session = seed_session(dir.path(), "tui.jsonl");
    std::fs::write(session.with_extension("jsonl.lock"), "").unwrap();

    let f = gather(&inputs(dir.path()));
    let out = f.render();
    assert!(out.contains("held (holder not recorded)"), "{out}");
    assert!(
        f.blockers
            .iter()
            .any(|b| b.contains("remove the .lock file")),
        "the escape hatch is named: {:?}",
        f.blockers
    );
}

/// The command line itself: both spellings must reach the report, and the exit code must be the
/// verdict rather than a crash. This runs the real binary, because the failure it guards against
/// (`epsilon doctor` dying with "unknown argument") is exactly a parsing bug in the binary.
#[test]
fn the_binary_accepts_both_spellings_and_exits_with_a_verdict() {
    let exe = env!("CARGO_BIN_EXE_epsilon");
    let empty = tempfile::tempdir().unwrap();
    for args in [vec!["doctor"], vec!["--doctor"]] {
        let out = std::process::Command::new(exe)
            .args(&args)
            .current_dir(empty.path())
            .output()
            .expect("the binary runs");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains("epsilon doctor"),
            "{args:?} must print the report, got: {text}"
        );
        for section in ["build", "model", "sandbox", "workspace", "session", "tools"] {
            assert!(
                text.contains(section),
                "{args:?} is missing '{section}': {text}"
            );
        }
        // 0 = no blockers, 1 = blockers. Anything else (2 = usage, or a signal) is a bug.
        assert!(
            matches!(out.status.code(), Some(0) | Some(1)),
            "{args:?} exited {:?} — the verdict is 0 or 1, never a usage dump",
            out.status.code()
        );
    }
}

/// The session-level rung has to be *reachable*: a mode the CLI cannot parse is a mode the
/// ladder can never be asked for (port plan item 15).
#[test]
fn the_binary_accepts_the_read_only_sandbox_rung_and_refuses_invention() {
    let exe = env!("CARGO_BIN_EXE_epsilon");
    let empty = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(exe)
            .args(args)
            .current_dir(empty.path())
            .output()
            .expect("the binary runs")
    };

    // `--demo` runs the scripted loop with tool registration, so this exercises the whole
    // `--sandbox read-only` path rather than only the argument parser.
    let ok = run(&["--sandbox", "read-only", "--demo"]);
    let text = String::from_utf8_lossy(&ok.stdout);
    assert!(
        ok.status.success(),
        "--sandbox read-only --demo exited {:?}: {}",
        ok.status.code(),
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(
        text.contains("=== run start ==="),
        "the demo actually ran: {text}"
    );

    // An unknown rung is a usage error that names the vocabulary, never a silent default.
    let bad = run(&["--sandbox", "readonly-ish", "--demo"]);
    let err = String::from_utf8_lossy(&bad.stderr);
    assert!(!bad.status.success(), "an invented mode must not run");
    assert!(
        err.contains("auto|read-only|off"),
        "the error names the vocabulary: {err}"
    );
}
