//! The file list behind `@` completion.
//!
//! `@` offers the user a path to reference. If it offers one the model's own read tool
//! would refuse, the harness has made a promise it cannot keep — so the *ignore answer*
//! here is `epsilon_core::ignore`, the same matcher M99 put in core for the tools. There
//! is exactly one answer to "is this file part of the workspace", and this module does not
//! get a second opinion.
//!
//! What it does own is the traversal, and that is a deliberate difference rather than an
//! oversight: this walk is budgeted for typing latency — a few hundred rows, capped depth,
//! stopping early — while a tool walk must see everything it was asked about. The rules
//! about *what to skip* mirror `epsilon_tools::util::walk_inner`, including the two that
//! are easy to get wrong: a hidden **file** stays visible (`.env` is content, and the read
//! tools must be able to see it), while a hidden **directory** is skipped unconditionally
//! — the `.gitignore` negation that is supposed to be the escape hatch cannot work through
//! `IgnoreSet::is_ignored`, in this walk or the tool's (see `walk_inner`'s comment).
//!
//! If a third consumer of this walk ever appears, the walk belongs in core beside
//! `IgnoreSet` — two copies of "what to skip" is one more than the rule allows.

use std::path::Path;

/// How many `.gitignore` files a completion walk will read before giving up on finding
/// more. Matches the tool walk's cap in spirit and is smaller in fact: completion is
/// interactive, and reading a thousand ignore files to rank a keystroke is not a trade
/// this path makes.
pub const AT_IGNORE_FILES: usize = 64;

/// Rows the completion list may collect before it stops looking.
pub const AT_ROW_BUDGET: usize = 200;

/// Depth cap, matching the tool walk so `@` cannot offer a path deeper than the tools
/// would reach.
const AT_MAX_DEPTH: usize = 16;

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
    let mut out = Vec::new();
    walk_inner(root, root, 0, ignore, budget, &mut out);
    out
}

fn walk_inner(
    root: &Path,
    dir: &Path,
    depth: usize,
    ignore: &epsilon_core::ignore::IgnoreSet,
    budget: &mut usize,
    out: &mut Vec<String>,
) {
    if depth > AT_MAX_DEPTH || *budget == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // Sorted so the same workspace always produces the same list: an unstable order makes
    // the popup jump while the user types, and makes a test flaky rather than wrong.
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        if *budget == 0 {
            return;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let is_dir = path.is_dir();

        // Build directories first: a universe of generated files is not a preference, and
        // listing it makes `@` useless exactly in the repositories that need it most.
        if is_dir && epsilon_core::ignore::skip_dir(&name) {
            continue;
        }
        // A hidden directory is skipped, always; a hidden FILE is content and stays.
        //
        // The tool walk writes this check as `name.starts_with('.') && !ignore.is_ignored(..)`,
        // with a comment promising that a `.gitignore` negation can re-include a hidden
        // directory. It cannot, in either walk: `is_ignored` resolves ancestors and returns
        // **false** for a directory that was ignored and then re-included — the same answer
        // it gives for a directory no rule mentions — so that extra clause changes the
        // outcome only in the case where the very next check skips the entry anyway. It is
        // dead logic, and copying it here would copy the illusion that goes with it. Making
        // the escape hatch real needs a matcher query able to tell "no rule" from
        // "re-included"; until then this walk matches what the tools actually do rather than
        // what they say.
        if is_dir && name.starts_with('.') {
            continue;
        }
        if ignore.is_ignored(&relative, is_dir) {
            continue;
        }

        if is_dir {
            walk_inner(root, &path, depth + 1, ignore, budget, out);
        } else {
            out.push(relative);
            *budget -= 1;
        }
    }
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
