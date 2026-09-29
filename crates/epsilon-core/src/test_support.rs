//! Test-only fixtures shared across the workspace's unit tests.
//!
//! # Why a directory link, and why a hand-rolled one
//!
//! Symlink containment cannot be tested with a function-local helper on Windows:
//! `std::os::windows::fs::symlink_dir` needs `SeCreateSymbolicLinkPrivilege`, which a
//! stock developer machine does not hold, so the honest test would skip itself on the
//! platform where the bug was reported.
//!
//! A **junction** needs no privilege, needs no elevation, and is followed by
//! `std::fs::canonicalize` exactly like a symlink — so the test exercises the real
//! resolution path on the machine it runs on rather than asserting that a skip
//! happened. It is created the one way Windows allows without a token, `mklink /J`.
//!
//! The function returns `false` rather than panicking when the platform refuses, and
//! every caller says so out loud. A silent skip is the failure mode this repository
//! has paid for more than once: a green test that asserted nothing.
//!
//! It is a public module rather than a private one so `epsilon-tools` can build the
//! same fixture its own containment tests need: a second copy of "how do I make a
//! link on this platform" is a second answer to maintain, and the two would drift.

use std::path::Path;

/// Create a directory link at `link` pointing at `target`. `false` when the OS refused.
///
/// On Unix this is an ordinary directory symlink. On Windows it is a junction, which
/// needs no privilege; the `mklink` shell verb is the documented way to make one.
pub fn dir_link(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return true;
        }
        mklink(target, link)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        false
    }
}

/// Create a **file** link at `link` pointing at `target`. `false` when the OS refused.
///
/// The file-shaped twin of [`dir_link`], and refused by the same privilege that
/// refuses a file symlink on Windows. It exists so a caller that needs the file
/// shape can say out loud that it was skipped, instead of quietly testing
/// nothing and reporting a pass.
pub fn file_link(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        // No `mklink` fallback here, deliberately. `/J` is the junction form and
        // takes a directory; the bare form is a file symlink and needs the same
        // `SeCreateSymbolicLinkPrivilege` that just refused. There is no unprivileged
        // route to a file link, so the honest answer is to say the platform refused.
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        false
    }
}

/// `mklink /J`, the junction form of a directory link.
///
/// A borrowed `Command` needs owned storage for every argument, and the target may
/// contain spaces (`C:\Users\Someone\My Agent Space\…`) — which is exactly the case
/// that must work, so the verb and its operands are passed as separate argv entries
/// and only the verb itself goes through the shell.
#[cfg(windows)]
fn mklink(target: &Path, link: &Path) -> bool {
    let (Some(dir), Some(name)) = (link.parent(), link.file_name()) else {
        return false;
    };
    let dir = dir.to_string_lossy().into_owned();
    let name = name.to_string_lossy().into_owned();
    let target = target.to_string_lossy().into_owned();
    std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&name)
        .arg(&target)
        .current_dir(&dir)
        // Stdin explicitly, like every other spawn in this repository. This module
        // is compiled into the release binary, and `mklink` inherits the harness's
        // stdin — which in the TUI is the console the user is typing into. The
        // M197 guard (`no_production_spawn_leaves_stdin_unset`) caught exactly this
        // omission the first time it ran against it, which is the guard working:
        // the same defect class, found in the new code by the test written for the
        // old instance of it.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
