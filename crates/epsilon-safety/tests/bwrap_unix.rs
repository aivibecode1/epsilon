//! Unix confinement verification (M13) — the runtime half of the M10 criterion.
//!
//! These tests are `#[cfg(unix)]`: they compile nowhere else and run on any Linux box
//! with bubblewrap installed (`bwrap --version`), giving CI a path to close the
//! "Linux confinement verified" acceptance item without touching Windows.
//!
//! Scope: process-level behavior of the [`BwrapSandbox`] seam — isolation actually
//! applies (no host /tmp leakage, read-only root), commands still succeed inside.

#![cfg(unix)]

use epsilon_safety::{BwrapSandbox, ConfinedCommand, ConfinementPolicy, Sandbox};

fn sandbox() -> Option<BwrapSandbox> {
    let path = which_bwrap()?;
    let sb = BwrapSandbox::new(path);
    sb.available().then_some(sb)
}

fn which_bwrap() -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join("bwrap"))
        .find(|p| p.is_file())
}

fn policy() -> ConfinementPolicy {
    ConfinementPolicy::default()
}

/// Basic sanity: an echo survives confinement with output intact.
#[tokio::test]
async fn echo_runs_under_bubblewrap() {
    let Some(sb) = sandbox() else {
        eprintln!("skipping: bubblewrap not installed");
        return;
    };
    let outcome = sb
        .run(
            ConfinedCommand {
                program: "sh".into(),
                args: vec!["-c".into(), "echo bwrap-ok".into()],
                cwd: std::env::temp_dir(),
                ..Default::default()
            },
            &policy(),
            None,
        )
        .await
        .expect("confined sh -c echo");
    assert_eq!(outcome.exit_code, Some(0));
    assert!(outcome.stdout.contains("bwrap-ok"), "{outcome:?}");
    assert_eq!(outcome.backend, "bwrap");
}

/// The unshared mount namespace must isolate tmpfs /tmp from the HOST temp dir. The probe
/// marker lives in a fresh directory OUTSIDE the confined command's workspace, so the only
/// way `sh` could see it is through the host mount (an actual isolation failure). Earlier
/// versions of this test planted the marker inside /tmp AND set cwd to /tmp, which made the
/// workspace bind itself expose the marker by design and failed on any runner whose
/// TMPDIR was /tmp.
#[tokio::test]
async fn host_tmp_is_not_visible_inside_sandbox() {
    let Some(sb) = sandbox() else {
        eprintln!("skipping: bubblewrap not installed");
        return;
    };
    // Workspace: its own scratch dir. Marker: a sibling dir that is NOT under it.
    let ws = std::env::temp_dir().join(format!("eps-bwrap-isolated-ws-{}", std::process::id()));
    std::fs::create_dir_all(&ws).expect("create workspace dir");
    let _ws_guard = DirGuard(&ws);

    let outside = std::env::temp_dir().join(format!("eps-bwrap-marker-{}", std::process::id()));
    std::fs::write(&outside, "host-only").expect("plant marker");
    let _cleanup = MarkerGuard(&outside);

    let script = format!(
        "if [ -f '{}' ]; then echo SEEN; else echo ISOLATED; fi",
        outside.display()
    );
    let outcome = sb
        .run(
            ConfinedCommand {
                program: "sh".into(),
                args: vec!["-c".into(), script],
                cwd: ws.clone(),
                ..Default::default()
            },
            &policy(),
            None,
        )
        .await
        .expect("probe run");
    assert!(
        outcome.stdout.contains("ISOLATED"),
        "host tmp leaked into sandbox: {outcome:?}"
    );
}

/// Removes a planted directory tree on drop.
struct DirGuard<'a>(&'a std::path::Path);
impl Drop for DirGuard<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0);
    }
}

/// Root is bound read-only: writes to `/` must fail inside the sandbox while the
/// workspace bind (cwd here) stays writable.
#[tokio::test]
async fn root_readonly_workspace_writable() {
    let Some(sb) = sandbox() else {
        eprintln!("skipping: bubblewrap not installed");
        return;
    };
    let ws = std::env::temp_dir().join(format!("eps-bwrap-ws-{}", std::process::id()));
    std::fs::create_dir_all(&ws).expect("workspace dir");
    struct WsGuard(PathBuf);
    impl Drop for WsGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _guard = WsGuard(ws.clone());

    let outcome = sb
        .run(
            ConfinedCommand {
                program: "sh".into(),
                args: vec![
                    "-c".into(),
                    "if touch /eps-denied 2>/dev/null; then echo ROOT-WRITABLE; else echo ROOT-RO; fi; \
                     if touch ./ws-write-ok 2>/dev/null; then echo WS-WRITABLE; else echo WS-RO; fi"
                        .into(),
                ],
                cwd: ws.clone(),
            ..Default::default()
            },
            &policy(),
            None,
        )
        .await
        .expect("isolation probe");
    assert!(outcome.stdout.contains("ROOT-RO"), "{outcome:?}");
    assert!(outcome.stdout.contains("WS-WRITABLE"), "{outcome:?}");
}

struct MarkerGuard<'a>(&'a std::path::Path);
impl Drop for MarkerGuard<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}
use std::path::PathBuf;

// ---- F11: one deadline for the whole run, and cancellation kills what it started ----

/// A command that outlives the wall clock must come back as `Timeout`, not hang.
///
/// Before F11 the bwrap backend ignored `wall_timeout_secs` entirely (`_policy`), so a
/// confined `sleep 30` waited the full 30 seconds whatever the policy said. The control for
/// this is the same shape on the unconfined path, which has always honoured its timeout.
#[tokio::test]
async fn a_confined_run_that_outlives_the_wall_clock_returns_timeout() {
    let Some(sb) = sandbox() else {
        eprintln!("skipping: bubblewrap not installed");
        return;
    };
    let mut p = policy();
    p.wall_timeout_secs = 1;

    let started = std::time::Instant::now();
    let err = sb
        .run(
            ConfinedCommand {
                program: "sh".into(),
                args: vec!["-c".into(), "sleep 30".into()],
                cwd: std::env::temp_dir(),
                ..Default::default()
            },
            &p,
            None,
        )
        .await
        .expect_err("a 1s budget cannot cover sleep 30");

    assert!(
        matches!(err, epsilon_safety::SandboxError::Timeout { seconds: 1 }),
        "the policy's own number comes back, so the caller can report it: {err:?}"
    );
    // The deadline is a promise about wall time, not just about the returned error. Two
    // seconds of slack absorbs scheduler noise on a loaded CI box without letting a real
    // regression — waiting the full 30s — pass.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "settled in {:?}, which is not a timeout",
        started.elapsed()
    );
}

/// The case a wait-only deadline cannot catch: the direct child exits, but a descendant keeps
/// stdout open. Bounding only `child.wait()` returns promptly and then blocks forever in the
/// pipe drain, so the caller never learns the run finished.
///
/// The shell backgrounds a 30s sleep, so `sh` itself exits immediately with status 0 while the
/// pipe is still held. The assertion is that `run` still returns — promptly, and as a timeout
/// once the deadline elapses.
#[tokio::test]
async fn a_descendant_holding_stdout_cannot_outlive_the_deadline() {
    let Some(sb) = sandbox() else {
        eprintln!("skipping: bubblewrap not installed");
        return;
    };
    let mut p = policy();
    p.wall_timeout_secs = 2;

    let started = std::time::Instant::now();
    let settled = sb
        .run(
            ConfinedCommand {
                program: "sh".into(),
                args: vec!["-c".into(), "sleep 30 & echo child-exited".into()],
                cwd: std::env::temp_dir(),
                ..Default::default()
            },
            &p,
            None,
        )
        .await;

    assert!(
        started.elapsed() < std::time::Duration::from_secs(6),
        "settled in {:?} — a held pipe is exactly the hang this test exists for",
        started.elapsed()
    );
    match settled {
        // Either outcome is acceptable; hanging is not. A tree that dies with the deadline may
        // report the timeout, and one whose namespace tore down the descendant first may
        // report the child's real status. What must not happen is no answer at all.
        Ok(outcome) => assert_eq!(outcome.exit_code, Some(0), "{outcome:?}"),
        Err(err) => assert!(
            matches!(err, epsilon_safety::SandboxError::Timeout { .. }),
            "if it failed, it failed by timing out: {err:?}"
        ),
    }
}

/// Cancellation is dropping the future. With no `kill_on_drop`, that leaves a confined tree
/// running with nobody watching — and the user who pressed the key sees the app return to
/// idle while a process keeps consuming the machine.
///
/// The probe writes a marker on a delay; the test aborts early and then waits long enough
/// for a surviving process to have written it. A process that was killed leaves no marker.
/// This is the only assertion here that needs real elapsed time, and it is bounded by the
/// sleep it gives itself.
#[tokio::test]
async fn cancelling_a_confined_run_does_not_leave_it_running() {
    let Some(sb) = sandbox() else {
        eprintln!("skipping: bubblewrap not installed");
        return;
    };
    let ws = std::env::temp_dir().join("eps-f11-cancel");
    std::fs::create_dir_all(&ws).unwrap();
    let marker = ws.join("survivor.txt");
    let _guard = MarkerGuard(&marker);
    let _ = std::fs::remove_file(&marker);

    let mut p = policy();
    p.wall_timeout_secs = 60; // far beyond the test: the point is to cancel, not to time out

    // `select!`, not `tokio::spawn`: the future borrows `sb` and `p`, so it is not `'static`
    // and cannot be spawned. The cancellation is identical either way — when the macro
    // returns, the un-selected future is dropped, which is precisely what "the user pressed
    // stop" means for this code path.
    let mut confined = Box::pin(sb.run(
        ConfinedCommand {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 3; touch survivor.txt".into()],
            cwd: ws.clone(),
            ..Default::default()
        },
        &p,
        None,
    ));
    tokio::select! {
        _ = &mut confined => {
            eprintln!("note: the run finished before the cancel; the assertion below is weaker than intended");
        }
        _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {
            // the confined future is dropped right here
        }
    }
    drop(confined);

    // Long enough for the abandoned process to have written the marker if it survived.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    assert!(
        !marker.exists(),
        "the confined tree outlived the cancellation: {} was written",
        marker.display()
    );
}
