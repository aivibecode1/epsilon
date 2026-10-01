//! The file list behind `@` completion.
//!
//! `@` offers the user a path to reference. If it offers one the model's own read tool
//! would refuse, the harness has made a promise it cannot keep — so the *ignore answer*
//! here is `epsilon_core::ignore`, the same matcher M99 put in core for the tools. There
//! is exactly one answer to "is this file part of the workspace", and this module does not
//! get a second opinion.
//!
//! # The walk moved to core at M236, as this module said it would
//!
//! The doc this file used to carry ended: *"If a third consumer of this walk ever appears,
//! the walk belongs in core beside `IgnoreSet` — two copies of 'what to skip' is one more
//! than the rule allows."* The desktop file tree was the third consumer, so the walk is now
//! [`epsilon_core::walk::walk_files`] and this module is the thin front-end wrapper.
//!
//! It stays rather than being deleted because `@` is a **capped, latency-budgeted** list —
//! 200 rows, 64 ignore files — and those numbers are a property of completion, not of the
//! walk. The editor's tree wants a different budget from the same traversal, which is
//! exactly why one traversal and two budgets is the right shape.
//!
//! What is deliberately different from a tool walk, and why, is written down in
//! `epsilon_core::walk`: a hidden **file** stays visible (`.env` is content, and the read
//! tools must be able to see it), while a hidden **directory** is skipped unconditionally,
//! because the `.gitignore` negation that should be the escape hatch cannot work through
//! `IgnoreSet::is_ignored` — in this walk or the tool's.

use std::path::Path;

/// How many `.gitignore` files a completion walk will read before giving up on finding
/// more. Completion is interactive, and reading a thousand ignore files to rank a
/// keystroke is not a trade this path makes.
pub const AT_IGNORE_FILES: usize = epsilon_core::walk::IGNORE_FILES;

/// Rows the completion list may collect before it stops looking.
pub const AT_ROW_BUDGET: usize = epsilon_core::walk::ROW_BUDGET;

/// Workspace-relative paths (forward slashes, like every path the model sees) that `@`
/// may offer.
///
/// `budget` is decremented per file found, so a caller can share one allowance across
/// several walks; it is left holding whatever remains.
pub fn walk(
    root: &Path,
    ignore: &epsilon_core::ignore::IgnoreSet,
    budget: &mut usize,
) -> Vec<String> {
    epsilon_core::walk::walk_files(root, ignore, budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("lib.rs"), "").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored.txt\n/build/\n").unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "secret").unwrap();
        std::fs::create_dir_all(dir.path().join("build")).unwrap();
        std::fs::write(dir.path().join("build").join("out.bin"), "").unwrap();
        std::fs::create_dir_all(dir.path().join(".hidden")).unwrap();
        std::fs::write(dir.path().join(".hidden").join("deep.txt"), "").unwrap();
        std::fs::write(dir.path().join(".env"), "KEY=1").unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target").join("junk"), "").unwrap();
        dir
    }

    fn rows(root: &Path) -> Vec<String> {
        let ignore = epsilon_core::ignore::IgnoreSet::load(root, AT_IGNORE_FILES);
        let mut budget = AT_ROW_BUDGET;
        let mut out = walk(root, &ignore, &mut budget);
        out.sort();
        out
    }

    #[test]
    fn the_list_agrees_with_the_ignore_matcher_the_tools_use() {
        let dir = workspace();
        let listed = rows(dir.path());
        assert!(listed.contains(&"main.rs".to_string()), "{listed:?}");
        assert!(listed.contains(&"src/lib.rs".to_string()), "{listed:?}");
        assert!(
            !listed.contains(&"ignored.txt".to_string()),
            "a gitignored file must not be offered: {listed:?}"
        );
        assert!(
            !listed.iter().any(|p| p.starts_with("build/")),
            "an ignored directory must not be descended into: {listed:?}"
        );
        assert!(
            !listed.iter().any(|p| p.starts_with("target/")),
            "a build directory is skipped by name whatever the .gitignore says: {listed:?}"
        );
        assert!(
            !listed.iter().any(|p| p.starts_with(".hidden/")),
            "a hidden directory stays out unless a negation re-includes it: {listed:?}"
        );
        assert!(
            listed.contains(&".env".to_string()),
            "a hidden FILE is content and the tools can read it: {listed:?}"
        );
    }

    #[test]
    fn a_hidden_directory_is_skipped_even_when_a_negation_names_it() {
        // This asserts the limitation, not a feature, and it is here because the tool
        // walk's comment claims the opposite. `is_ignored` returns false both for "a rule
        // ignored this and a later one re-included it" and for "no rule mentions it", so a
        // walk cannot tell the two apart and must skip hidden directories unconditionally.
        // The day a matcher query can distinguish them, this test is what should fail.
        let dir = workspace();
        std::fs::write(dir.path().join(".gitignore"), ".hidden/\n!.hidden/\n").unwrap();
        let listed = rows(dir.path());
        assert!(
            !listed.iter().any(|p| p.starts_with(".hidden/")),
            "the escape hatch is not real in the matcher: {listed:?}"
        );
        // The matcher itself does re-include the directory — the walk is what cannot use it.
        let ignore = epsilon_core::ignore::IgnoreSet::load(dir.path(), AT_IGNORE_FILES);
        assert!(
            !ignore.is_ignored(".hidden", true),
            "a re-included directory is not ignored; that is exactly why the walk cannot \
             distinguish it from an unmentioned one"
        );
    }

    #[test]
    fn the_budget_stops_the_walk_without_failing_it() {
        let dir = workspace();
        let ignore = epsilon_core::ignore::IgnoreSet::load(dir.path(), AT_IGNORE_FILES);
        let mut budget = 1usize;
        let out = walk(dir.path(), &ignore, &mut budget);
        assert_eq!(out.len(), 1, "one row of budget, one row of output");
        assert_eq!(budget, 0);
    }

    #[test]
    fn a_missing_workspace_is_an_empty_list_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("not-here");
        let ignore = epsilon_core::ignore::IgnoreSet::load(&gone, AT_IGNORE_FILES);
        let mut budget = AT_ROW_BUDGET;
        assert!(walk(&gone, &ignore, &mut budget).is_empty());
    }
}
