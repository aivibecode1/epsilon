//! Git state the TUI reads: the current branch and the working-tree statuses
//! painted in the explorer.
//!
//! The branch is read from `.git/HEAD` rather than by shelling out to git: the
//! footer draws ~30 times a second, and a subprocess per frame is not a trade
//! this module is willing to make. `App::cached_branch` adds a two-second TTL.
//! Working-tree status is a subprocess, but only on the explorer's slower poll.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// The working-tree state the explorer paints beside a path.
///
/// Deliberately a small vocabulary: a file tree needs to say "changed", "new",
/// "gone" and "conflict" without reproducing git's two-column index/worktree
/// model, which is a status viewer, not a directory tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Staged as a new file.
    Added,
    /// Tracked and changed.
    Modified,
    /// Removed from the working tree.
    Deleted,
    /// Renamed or copied.
    Renamed,
    /// Not tracked by git.
    Untracked,
    /// An unresolved merge conflict.
    Conflict,
}

impl Status {
    /// The one-column marker drawn at the right edge of a tree row.
    pub fn marker(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "?",
            Self::Conflict => "!",
        }
    }
}

/// One `git status --porcelain=v1 -z` working-tree snapshot, keyed by
/// workspace-relative path.
///
/// Empty is the honest answer for a non-repository, a missing `git`, or an
/// unreadable result: the tree still works, it just has no status column.
///
/// **The executable is resolved explicitly and repository fsmonitor hooks are
/// disabled.** A visible tree polls `git status` without the user issuing a
/// command, so a workspace must not be able to choose the binary (a planted
/// `git.exe` on Windows' current-directory search) or run a `core.fsmonitor`
/// command from its own config. Only an absolute `git` found on the user's PATH
/// is accepted; a relative or empty PATH entry is skipped.
pub fn status(workspace: &Path) -> BTreeMap<String, Status> {
    let Some(git) = git_program() else {
        return BTreeMap::new();
    };
    let out = Command::new(git)
        .args(["-c", "core.fsmonitor=false", "-C"])
        .arg(workspace)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output();
    match out {
        Ok(out) if out.status.success() => parse_porcelain(&out.stdout),
        _ => BTreeMap::new(),
    }
}

/// Find `git` by absolute PATH entry only.
///
/// `Command::new("git")` lets the platform search the process's current
/// directory before PATH on Windows; a repository containing `git.exe` would
/// then execute when the explorer opened. This helper returns only a path under
/// an absolute PATH entry, and `status` invokes that absolute path.
fn git_program() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    git_program_in(&path)
}

/// The pure half of [`git_program`], testable without mutating the process PATH.
fn git_program_in(path: &std::ffi::OsStr) -> Option<std::path::PathBuf> {
    for dir in std::env::split_paths(path) {
        if !dir.is_absolute() {
            continue;
        }
        for name in if cfg!(windows) {
            ["git.exe", "git"]
        } else {
            ["git", "git.exe"]
        } {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Parse the NUL-separated porcelain stream. Separated from [`status`] so the
/// mapping is testable without a repository or a `git` binary.
pub(crate) fn parse_porcelain(bytes: &[u8]) -> BTreeMap<String, Status> {
    let mut out = BTreeMap::new();
    let mut records = bytes.split(|byte| *byte == 0);
    while let Some(record) = records.next() {
        if record.len() < 4 {
            continue;
        }
        let (x, y) = (record[0], record[1]);
        if x == b' ' && y == b' ' {
            continue;
        }
        let path = String::from_utf8_lossy(&record[3..]).replace('\\', "/");
        if path.is_empty() {
            continue;
        }
        out.insert(path, classify(x, y));
        // A rename/copy record is followed by the original path as its own
        // NUL field; consume it so it cannot be mistaken for a second change.
        if matches!(x, b'R' | b'C') {
            let _ = records.next();
        }
    }
    out
}

fn classify(x: u8, y: u8) -> Status {
    if x == b'?' {
        return Status::Untracked;
    }
    if matches!((x, y), (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D')) {
        return Status::Conflict;
    }
    if matches!(x, b'R' | b'C') || y == b'R' {
        return Status::Renamed;
    }
    if matches!(x, b'A' | b'C') || y == b'A' {
        return Status::Added;
    }
    if matches!(x, b'D') || y == b'D' {
        return Status::Deleted;
    }
    Status::Modified
}

/// Directories that contain at least one changed path. A collapsed directory
/// still gets a marker, so a change cannot hide inside a fold.
pub fn changed_dirs(status: &BTreeMap<String, Status>) -> BTreeSet<String> {
    let mut dirs = BTreeSet::new();
    for path in status.keys() {
        let mut rest = path.as_str();
        while let Some(at) = rest.rfind('/') {
            rest = &rest[..at];
            if rest.is_empty() {
                break;
            }
            dirs.insert(rest.to_string());
        }
    }
    dirs
}

/// Current git branch read straight from `<dir>/.git/HEAD`
/// (`ref: refs/heads/<name>`); a detached HEAD yields the short sha; anything else
/// yields `None`.
///
/// Cheap enough to call per draw; callers may add a TTL.
///
/// Two shapes deliberately come back as `None` rather than as text. A linked worktree
/// keeps a *file* at `.git` holding `gitdir: ../elsewhere`, and a submodule keeps
/// something similar; neither is a branch, and the old implementation truncated that
/// pointer to seven characters and displayed `gitdir:` in the footer. Resolving the
/// pointer is not done here — it would mean following a path the caller did not ask
/// about, and "no branch" is the honest answer for a workspace this function cannot
/// resolve.
pub fn branch(workspace: &Path) -> Option<String> {
    let head = std::fs::read_to_string(workspace.join(".git").join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(rest) = head.strip_prefix("ref: refs/heads/") {
        return Some(rest.to_string());
    }
    // Detached HEAD: a full object name. ASCII hex is also what makes the slice below
    // safe, so it is checked rather than assumed.
    if head.len() >= 40 && head.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(head[..7].to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_ref_is_read_from_head() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(
            dir.path().join(".git").join("HEAD"),
            "ref: refs/heads/feature/x\n",
        )
        .unwrap();
        assert_eq!(branch(dir.path()).as_deref(), Some("feature/x"));
    }

    #[test]
    fn a_detached_head_is_the_short_sha() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(
            dir.path().join(".git").join("HEAD"),
            "3f2a1b9c4d5e6f708192a3b4c5d6e7f8091a2b3c\n",
        )
        .unwrap();
        assert_eq!(branch(dir.path()).as_deref(), Some("3f2a1b9"));
    }

    #[test]
    fn no_git_directory_is_not_a_branch() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(branch(dir.path()), None);
    }

    #[test]
    fn porcelain_status_maps_to_the_tree_vocabulary() {
        let records = b"M  src/lib.rs\0?? notes.txt\0A  new.rs\0D  gone.rs\0R  moved.rs\0old.rs\0";
        let got = parse_porcelain(records);
        assert_eq!(got.get("src/lib.rs"), Some(&Status::Modified));
        assert_eq!(got.get("notes.txt"), Some(&Status::Untracked));
        assert_eq!(got.get("new.rs"), Some(&Status::Added));
        assert_eq!(got.get("gone.rs"), Some(&Status::Deleted));
        assert_eq!(got.get("moved.rs"), Some(&Status::Renamed));
        assert!(!got.contains_key("old.rs"), "the rename source is consumed");
    }

    #[test]
    fn a_changed_file_marks_every_parent_directory() {
        let mut status = BTreeMap::new();
        status.insert("src/nested/deep.rs".to_string(), Status::Modified);
        let dirs = changed_dirs(&status);
        assert!(dirs.contains("src"));
        assert!(dirs.contains("src/nested"));
        assert!(!dirs.contains("src/nested/deep.rs"));
    }

    #[test]
    fn git_program_ignores_relative_path_entries() {
        assert_eq!(
            git_program_in(std::ffi::OsStr::new(".")),
            None,
            "a relative PATH entry must never become the executable"
        );
    }

    #[test]
    fn git_program_returns_an_absolute_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "git.exe" } else { "git" };
        let candidate = dir.path().join(name);
        std::fs::write(&candidate, "").unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        let found = git_program_in(&path).expect("the absolute candidate is found");
        assert!(found.is_absolute());
        assert_eq!(found, candidate);
    }

    #[test]
    fn a_worktree_pointer_is_not_a_branch() {
        // A linked worktree keeps a FILE at `.git` holding `gitdir: ../elsewhere`.
        // The old code truncated it to seven characters and printed `gitdir:` in the
        // footer; "no branch" is the honest answer for a pointer this function does
        // not follow.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(
            dir.path().join(".git").join("HEAD"),
            "gitdir: ../elsewhere\n",
        )
        .unwrap();
        assert_eq!(branch(dir.path()), None);

        // Short junk is not a sha either.
        std::fs::write(dir.path().join(".git").join("HEAD"), "not-a-sha\n").unwrap();
        assert_eq!(branch(dir.path()), None);
    }
}
