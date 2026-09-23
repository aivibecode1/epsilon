//! The `--import` command, driven through the real binary.
//!
//! The store's tests cover the conversion; this file covers the two things only the *command*
//! can be wrong about. First, that `--resume` on a current pi session — the state the user is
//! actually in when they hit this, because their pi file is the newest session in the folder —
//! ends in a sentence they can act on: the diagnosis, then the exact command. A refusal a user
//! cannot act on is half a feature. Second, that running that command does what the refusal
//! promised: a new session beside the pi file, and the pi file byte-identical.
//!
//! The store tests cannot catch either: they call `import_pi_session` directly, so a CLI that
//! spelled the flag wrong, printed nothing, or started a turn instead of exiting would pass
//! every one of them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The binary under test, as cargo built it for this test run.
fn epsilon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_epsilon"))
}

/// A pi **v2** session file — the shape this build refuses in place: a header, then entries with
/// `id`/`parentId`, the kind at the top level and pi's own ISO `timestamp`.
fn write_pi_v2_file(dir: &Path) -> PathBuf {
    let path = dir.join("pi-session.jsonl");
    let body = [
        r#"{"type":"session","version":3,"id":"ses_pi","timestamp":"2025-12-08T22:41:05.000Z","cwd":"/tmp/pi","provider":"anthropic","modelId":"claude-opus-4-5"}"#,
        r#"{"type":"message","id":"e1","parentId":null,"timestamp":"2025-12-08T22:41:05.306Z","message":{"role":"user","content":[{"type":"text","text":"walk me through main.ts"}],"timestamp":1765233665292}}"#,
        r#"{"type":"message","id":"e2","parentId":"e1","timestamp":"2025-12-08T22:41:09.389Z","message":{"role":"assistant","content":[{"type":"text","text":"reading it now"}],"provider":"anthropic","model":"claude-opus-4-5","stopReason":"endTurn","timestamp":1765233665294}}"#,
    ]
    .join("\n");
    std::fs::write(&path, format!("{body}\n")).expect("write the pi file");
    path
}

/// `--import` on a pi file: exit 0, a summary naming the copy, and a source that has not moved.
#[test]
fn import_writes_a_copy_beside_the_pi_file_and_leaves_it_alone() {
    let dir = tempfile::tempdir().expect("tmp");
    let source = write_pi_v2_file(dir.path());
    let before = std::fs::read(&source).expect("read the pi file");

    let out = epsilon()
        .arg("--import")
        .arg(&source)
        .output()
        .expect("run epsilon --import");

    assert!(
        out.status.success(),
        "--import exits 0: {:?}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The summary names what a user needs: how much came over, where it landed, and that their
    // own file is untouched. `stderr` rather than `stdout` because the import is diagnostics,
    // not the reply a pipe is waiting for.
    assert!(
        stderr.contains("imported 2 entries into"),
        "the summary counts the entries: {stderr}"
    );
    assert!(
        stderr.contains("pi-session.jsonl.epsilon.jsonl"),
        "the summary names the copy: {stderr}"
    );
    assert!(
        stderr.contains("the source was not modified"),
        "the summary promises what the test below checks: {stderr}"
    );
    assert!(
        stderr.contains("delete it yourself"),
        "and hands the original back to the user rather than deleting it: {stderr}"
    );
    assert_eq!(
        std::fs::read(&source).expect("read the pi file again"),
        before,
        "the pi file is byte-identical after the import"
    );
    assert!(
        dir.path().join("pi-session.jsonl.epsilon.jsonl").exists(),
        "the copy is where the summary said it would be"
    );
}

/// `--resume` on a current pi session says what the file is *and* what to run.
///
/// The store's message for this file is true and useless: line 2 "is not an entry, a header or a
/// legacy entry — a shape this build does not know". Naming the shape and the next command is the
/// whole difference between a dead end and a feature, and it is the sentence a user reads first
/// (their pi file is usually the newest session in the folder).
#[test]
fn resume_on_a_pi_session_names_the_file_and_the_command_that_converts_it() {
    let dir = tempfile::tempdir().expect("tmp");
    let source = write_pi_v2_file(dir.path());

    let out = epsilon()
        .arg("--resume")
        .arg(&source)
        .output()
        .expect("run epsilon --resume");

    assert!(
        !out.status.success(),
        "--resume still refuses a file it cannot open"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("is a current pi session"),
        "the diagnosis names what the file is: {stderr}"
    );
    assert!(
        stderr.contains(&format!("epsilon --import {}", source.display())),
        "…and prints the exact command to run, path included: {stderr}"
    );
    assert!(
        stderr.contains("leaves the pi file exactly as it is"),
        "…and says what that command will do to their file: {stderr}"
    );
    assert!(
        !dir.path().join("pi-session.jsonl.epsilon.jsonl").exists(),
        "a refusal imports nothing — the user runs the command when they choose to"
    );
}

/// `--import` on a file that is not there fails with the path, not a panic or a silent zero.
#[test]
fn import_of_a_missing_file_names_the_path() {
    let dir = tempfile::tempdir().expect("tmp");
    let missing = dir.path().join("nope.jsonl");

    let out = epsilon()
        .arg("--import")
        .arg(&missing)
        .output()
        .expect("run epsilon --import");

    assert!(!out.status.success(), "a missing file is a failure");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no such file") && stderr.contains("nope.jsonl"),
        "the message names the path the user typed: {stderr}"
    );
}
