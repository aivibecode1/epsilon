//! The one piece of git state the TUI reads: the current branch.
//!
//! This module exists because it is the *only* thing that survived the footer
//! redesign. The old `footer.rs` held a complete second footer — its own
//! `format_tokens`, its own cache-rate formula, `FooterStats`, `context_parts`,
//! `stats_parts` — and when `ui/` replaced it, everything stayed behind, `pub`, so
//! `clippy -D warnings` could not see that nothing called it (G17). Deleting the
//! module and keeping the one live function is the honest version: a name that says
//! what it does, one caller, one test.
//!
//! The branch is read straight from `.git/HEAD` rather than by shelling out to git:
//! the footer draws ~30 times a second, and a subprocess per frame is not a trade
//! this file is willing to make. `App::cached_branch` adds a two-second TTL on top.

use std::path::Path;

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
