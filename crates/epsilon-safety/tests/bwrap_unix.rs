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
            },
            &policy(),
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
            },
            &policy(),
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
            },
            &policy(),
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
