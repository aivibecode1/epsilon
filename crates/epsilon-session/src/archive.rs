//! Archiving a session file: hiding it from listings without deleting it.
//!
//! ## What this is for
//!
//! Deletion ([`crate::delete`]) is permanent on machines with no `trash` binary — which is every
//! Windows machine this project is developed on. A reader who wants a session out of the way but
//! not gone has no verb for that: the browser lists everything on disk, and the only removal is
//! the unrecoverable one. That gap is parity row S-108 (DSH's `session.archive`).
//!
//! Archiving moves the file into an `archived/` subdirectory of the sessions directory it lives
//! in. Nothing else changes: the file keeps its name, its id and its history, and unarchiving
//! moves it back.
//!
//! ## Why a move, and why listings need no filter
//!
//! [`crate::discover::list_sessions`] reads one directory non-recursively and only takes
//! `.jsonl` files, so a file under `archived/` is invisible to every existing reader with no
//! code changed at any call site: `--continue` ([`crate::discover::latest_session`]), the
//! per-workspace union ([`crate::discover::list_sessions_for_cwd`]), and both front ends'
//! browsers all stop offering it. An archive that instead kept the file in place behind a flag
//! would need a flag every reader honours — a filter nobody can forget — and a sidecar nobody
//! can lose. A move is the filter: the directory *is* the state.
//!
//! ## What this deliberately does not do
//!
//! DSH keeps archived rows in place, marked, unopenable with an explanation, behind a
//! show-archived filter with undo on a toast. The window answers the same need with less
//! machinery: archived rows are listed in place under an `Archived` heading and choosing one
//! restores it (see the sessions pane). There is no separate filter state to lose and no toast
//! surface to build — this project has no toasts, and a notice in the transcript is the
//! surface that persists.
//!
//! `ponytail:` a move through `fs::rename`, not a copy-then-delete and not a trash integration.
//! The file stays on the same volume inside the same sessions root, so a rename is atomic and
//! cannot leave half a session behind. **Ceiling: no cross-volume moves, no trash. Upgrade
//! trigger: a request to archive across volumes.**

use std::path::{Path, PathBuf};

/// The subdirectory of a sessions directory that holds archived sessions.
///
/// Files directly under it are archived; everything else in the sessions
/// directory is live. [`crate::discover::list_sessions`] never descends, so
/// this one name is the whole of the archive state.
pub const ARCHIVED_DIR: &str = "archived";

/// Where archived sessions of `dir` live.
pub fn archived_dir(dir: &Path) -> PathBuf {
    dir.join(ARCHIVED_DIR)
}

/// Move the session file at `path` into its directory's `archived/` subdirectory.
///
/// Returns the file's new path. Errors when the file is missing (a listing
/// read a moment ago is not a promise), when it is already archived, or when
/// an archived file of the same name is already there — a clobber would be a
/// silent delete, which is the one thing this module must never do.
pub fn archive_session_file(path: &Path) -> Result<PathBuf, String> {
    // Divergence from `delete_session_file`'s shape, and the reason is the
    // direction: a delete that finds nothing is a race worth naming, and so
    // is an archive — archiving a file that already left would otherwise
    // report a move that never happened.
    if !path.exists() {
        return Err(format!("no such session file: {}", path.display()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("cannot archive {}: no parent directory", path.display()))?;
    // Archiving an archived file would nest `archived/archived/` — a second
    // move that reads as progress and is really a misfiled session.
    if parent.file_name().is_some_and(|n| n == ARCHIVED_DIR) {
        return Err(format!("already archived: {}", path.display()));
    }
    let dest_dir = archived_dir(parent);
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("cannot archive {}: {e}", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("cannot archive {}: no file name", path.display()))?;
    let dest = dest_dir.join(name);
    // Same-volume renames replace on Unix and refuse on Windows; neither is
    // acceptable here, because either way a session is lost — refused on one
    // platform, silently replaced on the other. Refusing everywhere is the
    // one rule both platforms can share.
    if dest.exists() {
        return Err(format!("already archived: {}", dest.display()));
    }
    // Same volume by construction (a subdirectory of the file's own
    // directory), so the rename is atomic: the listing sees the file in one
    // place or the other, never in neither.
    std::fs::rename(path, &dest).map_err(|e| format!("cannot archive {}: {e}", path.display()))?;
    Ok(dest)
}

/// Move an archived session file back into the live sessions directory.
///
/// Only files directly under an `archived/` directory are accepted: anything
/// else is not archived, and saying so names the caller's mistake rather than
/// moving a live file beside itself.
pub fn unarchive_session_file(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("no such session file: {}", path.display()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("cannot unarchive {}: no parent directory", path.display()))?;
    // Only files directly under an `archived/` directory are archived. A
    // live file unarchived beside itself would be a copy wearing the wrong
    // verb — and a move onto itself is at best a no-op that reports success.
    if parent.file_name().is_none_or(|n| n != ARCHIVED_DIR) {
        return Err(format!("not an archived session: {}", path.display()));
    }
    let home = parent
        .parent()
        .ok_or_else(|| format!("cannot unarchive {}: no home directory", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("cannot unarchive {}: no file name", path.display()))?;
    let dest = home.join(name);
    if dest.exists() {
        return Err(format!(
            "a live session already exists at {}",
            dest.display()
        ));
    }
    std::fs::rename(path, &dest)
        .map_err(|e| format!("cannot unarchive {}: {e}", path.display()))?;
    Ok(dest)
}

/// Every archived session in `dir`, newest first. A directory with no archive
/// — including one that does not exist — lists nothing.
pub fn list_archived(dir: &Path) -> Vec<crate::discover::SessionSummary> {
    crate::discover::list_sessions(&archived_dir(dir))
}

/// Every archived session belonging to `cwd`, newest first: the per-cwd
/// directory's archive plus the flat root's archive filtered by header cwd —
/// the same union [`crate::discover::list_sessions_for_cwd`] builds over the
/// live files, so an archived session is found wherever its live self was.
pub fn list_archived_for_cwd(root: &Path, cwd: &Path) -> Vec<crate::discover::SessionSummary> {
    use crate::discover::SessionSummary;
    let want = cwd.to_string_lossy().into_owned();
    let mut out: Vec<SessionSummary> = list_archived(&crate::time::session_dir_for(root, cwd));
    out.extend(
        list_archived(root)
            .into_iter()
            .filter(|s| s.cwd.is_empty() || s.cwd == want),
    );
    out.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| b.path.cmp(&a.path))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::{latest_session, list_sessions, list_sessions_for_cwd};
    use crate::store::SessionStore;

    fn planted_in(dir: &Path, file: &str, cwd: &Path) -> PathBuf {
        let path = dir.join(file);
        let store = SessionStore::open_in(&path, cwd).expect("a store opens");
        drop(store);
        assert!(path.exists(), "the fixture is on disk");
        path
    }

    fn planted(dir: &Path, file: &str) -> PathBuf {
        planted_in(dir, file, dir)
    }

    #[test]
    fn archive_moves_the_file_out_of_the_listing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = planted(dir, "doomed.jsonl");

        let dest = archive_session_file(&path).expect("archiving a live session works");

        assert_eq!(dest, archived_dir(dir).join("doomed.jsonl"));
        assert!(!path.exists(), "the live file is gone");
        assert!(dest.exists(), "and the archived copy is on disk");
        assert!(
            list_sessions(dir).is_empty(),
            "archived sessions are not listed"
        );
        let archived = list_archived(dir);
        assert_eq!(archived.len(), 1, "but they are listed as archived");
        assert_eq!(archived[0].path, dest);
    }

    #[test]
    fn archive_of_a_missing_file_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = archive_session_file(&tmp.path().join("gone.jsonl")).unwrap_err();
        assert!(
            err.contains("no such session"),
            "a vanished file is named, not archived: {err}"
        );
    }

    #[test]
    fn archiving_twice_is_an_error_not_a_clobber() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = planted(dir, "twice.jsonl");
        let dest = archive_session_file(&path).expect("first archive works");

        let again = archive_session_file(&dest).unwrap_err();
        assert!(
            again.contains("already archived"),
            "an archived file archived again says so: {again}"
        );
        let ghost = archive_session_file(&path).unwrap_err();
        assert!(
            ghost.contains("no such session"),
            "the moved-away live path is missing, not archivable: {ghost}"
        );
        assert!(
            dest.exists(),
            "and the one archived copy survived both attempts"
        );
    }

    #[test]
    fn unarchive_round_trips_back_into_the_listing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = planted(dir, "back.jsonl");
        let dest = archive_session_file(&path).expect("archive works");

        let home = unarchive_session_file(&dest).expect("unarchive works");

        assert_eq!(home, path);
        assert!(home.exists(), "the live file is back");
        assert_eq!(list_sessions(dir).len(), 1, "and listed as live again");
        assert!(list_archived(dir).is_empty(), "and no longer archived");
    }

    #[test]
    fn unarchive_of_a_live_file_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = planted(dir, "live.jsonl");

        let err = unarchive_session_file(&path).unwrap_err();
        assert!(
            err.contains("not an archived session"),
            "a live file is not unarchivable: {err}"
        );
        assert!(path.exists(), "and it is untouched");
    }

    #[test]
    fn archived_sessions_are_invisible_to_continue() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = planted(dir, "quiet.jsonl");
        archive_session_file(&path).expect("archive works");

        assert_eq!(
            latest_session(dir, None),
            None,
            "an archived session must not be continued"
        );
        assert!(
            list_sessions_for_cwd(dir, dir).is_empty(),
            "nor listed for its workspace"
        );
    }

    #[test]
    fn archived_for_cwd_mirrors_the_live_union() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let cwd = Path::new("/w");
        let per_cwd = crate::time::session_dir_for(root, cwd);
        std::fs::create_dir_all(&per_cwd).unwrap();

        // One archived session in each layout, like the live union's M217 test.
        let legacy = planted_in(root, "legacy.jsonl", cwd);
        archive_session_file(&legacy).expect("legacy archives");
        let fresh = planted_in(&per_cwd, "fresh.jsonl", cwd);
        archive_session_file(&fresh).expect("fresh archives");
        // Someone else's archived session, in the same flat root.
        let other = planted_in(root, "other.jsonl", Path::new("/elsewhere"));
        archive_session_file(&other).expect("other archives");

        let rows = list_archived_for_cwd(root, cwd);
        assert_eq!(rows.len(), 2, "both layouts, and no stowaway: {rows:?}");
        assert!(
            rows.iter()
                .all(|s| s.cwd == "/w" || s.path.starts_with(&per_cwd)),
            "the flat file is this workspace's, the per-cwd file needs no header: {rows:?}"
        );
    }
}
