//! The credential command's user-visible contract, through the real binary.
//!
//! This test writes a credential — so it **must** never see the real home. It
//! sets both `HOME` and `USERPROFILE` to a temp dir and asserts the file landed
//! there before reading it. The earlier manual verification of this warning
//! used a PowerShell `$home` variable that collided with the read-only `$HOME`
//! and wrote the real profile; the explicit `home_dir` name and the path
//! assertion below are the mechanical guard against repeating that.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(home: &std::path::Path, args: &[&str], stdin: Option<&str>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_epsilon"));
    command
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command.spawn().expect("the binary runs");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(input.as_bytes())
            .expect("write the key");
    }
    child.wait_with_output().expect("the command finishes")
}

#[test]
fn auth_login_warns_and_stores_only_under_the_temp_home() {
    let home_dir = tempfile::tempdir().expect("temp home");
    let login = run(
        home_dir.path(),
        &["auth", "login", "deepseek"],
        Some("sk-test-only\n"),
    );
    assert!(
        login.status.success(),
        "login failed: {}",
        String::from_utf8_lossy(&login.stderr)
    );
    let stderr = String::from_utf8_lossy(&login.stderr);
    assert!(
        stderr.contains("plaintext"),
        "the warning must say the file is plaintext: {stderr}"
    );
    assert!(
        stderr.contains("environment variable"),
        "the warning must name the alternative: {stderr}"
    );

    let auth_path = home_dir
        .path()
        .join(".config")
        .join("epsilon")
        .join("auth.json");
    let raw = std::fs::read_to_string(&auth_path).expect("auth.json landed under the temp home");
    assert!(raw.contains("sk-test-only"), "the key was stored: {raw}");

    let list = run(home_dir.path(), &["auth", "list"], None);
    assert!(list.status.success());
    let stdout = String::from_utf8_lossy(&list.stdout);
    let stderr = String::from_utf8_lossy(&list.stderr);
    assert!(stdout.contains("deepseek"), "{stdout}");
    assert!(
        !stdout.contains("sk-test-only") && !stderr.contains("sk-test-only"),
        "auth list must not print the full key: {stdout} {stderr}"
    );
    assert!(
        stderr.contains("plaintext"),
        "auth list repeats the plaintext note: {stderr}"
    );
}
