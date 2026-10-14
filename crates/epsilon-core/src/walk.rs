//! Walking a workspace to list the files that are *content* rather than build output.
//!
//! # Why this lives here and not in the frontend that first needed it
//!
//! `epsilon_tui::at_files` wrote it for `@`-completion, and its own module doc said what
//! would happen when somebody else needed the same answer: *"If a third consumer of this
//! walk ever appears, the walk belongs in core beside `IgnoreSet` — two copies of 'what to
//! skip' is one more than the rule allows."* The desktop file tree is that third consumer,
//! so the walk moved here and `at_files` now delegates.
//!
//! The alternative was a third traversal written from `IgnoreSet`'s rules, and it is worth
//! being concrete about what that costs: a file that `grep` refuses to search appearing in
//! the editor's tree, or `target/debug` being absent from `@` but present in the file list.
//! Neither is a bug anyone would report, because both look like a list being a list.
//!
//! # What is the same and what is deliberately different
//!
//! The *rules* about what to skip are [`IgnoreSet`](crate::ignore::IgnoreSet)'s, and there is exactly one answer to
//! "is this file part of the workspace". The *traversal* differs by caller, and that is a
//! deliberate difference rather than an oversight: a completion list is budgeted for
//! typing latency, while a tool walk must see everything it was asked about.
//!
//! Two skip rules are the easy ones to get wrong and both are load-bearing:
//!
//! * **A hidden file stays visible.** `.env` is content, and the read tools must be able
//!   to see it.
//! * **A hidden directory is skipped unconditionally.** The `.gitignore` negation that is
//!   supposed to be the escape hatch cannot work through [`IgnoreSet::is_ignored`](crate::ignore::IgnoreSet::is_ignored), in this
//!   walk or the tools' own: it resolves ancestors and returns **false** for a directory
//!   that was ignored and then re-included — the same answer it gives for a directory no
//!   rule mentions. Making the escape hatch real needs a matcher query that can tell "no
//!   rule" from "re-included"; until then this walk matches what the tools actually do
//!   rather than what they say they do.

use std::path::Path;

use crate::ignore::{IgnoreSet, skip_dir};

/// How many `.gitignore` files a listing walk will read before giving up on finding more.
///
/// Bounded because a repository with a deep vendored tree can hold hundreds, and reading
/// every one of them to rank a keystroke is not a trade this path makes.
pub const IGNORE_FILES: usize = 64;

/// Rows a walk may collect before it stops looking.
pub const ROW_BUDGET: usize = 200;

/// Depth cap, so a symlinked or pathological tree cannot walk forever.
pub const MAX_DEPTH: usize = 16;

/// Workspace-relative paths — forward slashes, like every path the model sees — that are
/// content rather than output.
///
/// `budget` is decremented per file found, so a caller can share one allowance across
/// several walks; it is left holding whatever remains. A walk that runs out of budget is
/// **truncated, not failed**: the caller still gets a usable list and can say it is partial.
pub fn walk_files(root: &Path, ignore: &IgnoreSet, budget: &mut usize) -> Vec<String> {
    let mut out = Vec::new();
    walk_inner(root, root, 0, ignore, budget, &mut out);
    out
}

fn walk_inner(
    root: &Path,
    dir: &Path,
    depth: usize,
    ignore: &IgnoreSet,
    budget: &mut usize,
    out: &mut Vec<String>,
) {
    if depth > MAX_DEPTH || *budget == 0 {
        return;
    }
    // A workspace that cannot be read is an empty list, not a panic: the window opens on
    // a directory the user chose and must still open.
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // Sorted so the same workspace always produces the same list: an unstable order makes
    // the tree jump while the user clicks, and makes a test flaky rather than wrong.
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

        // **A link is not descended into.** `is_dir()` above follows it — a symlink, or a
        // Windows junction, which needs no privilege and which `is_symlink()` does not even
        // report — so without this a repository holding `docs -> ~/Documents` hands `grep`,
        // `@` completion and the file tree every file underneath, with no permission ask,
        // because every name is still workspace-relative. See [`crate::paths::is_link`].
        //
        // The cost is one `symlink_metadata` per entry, and the walk cannot leave the root
        // any other way: normal descent from the root stays inside it.
        //
        // A skipped link **spends budget**, so the caller's existing "the walk stopped
        // early" signal can fire — the list really is narrower than the filesystem, and a
        // tree that says `400 files` when the disk holds 405 through a link is lying in the
        // way this repository cares most about.
        if crate::paths::is_link(&path) {
            *budget = budget.saturating_sub(1);
            continue;
        }

        // Build directories are skipped whatever `.gitignore` says. A universe of
        // generated files is not a preference, and listing one makes both `@` and the
        // editor's tree useless exactly in the repositories that need them most.
        if is_dir && skip_dir(&name) {
            continue;
        }
        // A hidden directory is skipped, always; a hidden FILE is content and stays.
        // See the module docs for why the `.gitignore` escape hatch does not apply.
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
        let ignore = IgnoreSet::load(root, IGNORE_FILES);
        let mut budget = ROW_BUDGET;
        let mut out = walk_files(root, &ignore, &mut budget);
        out.sort();
        out
    }

    #[test]
    fn the_ignore_answer_is_the_one_the_tools_give() {
        let dir = workspace();
        let out = rows(dir.path());
        assert!(out.contains(&"main.rs".to_string()), "{out:?}");
        assert!(out.contains(&"src/lib.rs".to_string()), "{out:?}");
        assert!(!out.contains(&"ignored.txt".to_string()), "{out:?}");
        assert!(!out.contains(&"build/out.bin".to_string()), "{out:?}");
        assert!(!out.contains(&".hidden/deep.txt".to_string()), "{out:?}");
        assert!(!out.contains(&"target/junk".to_string()), "{out:?}");
    }

    #[test]
    fn a_hidden_file_is_content_and_stays() {
        // `.env` is in every repository and a file tree that hides it is a file tree that
        // lies about what the agent can see.
        let dir = workspace();
        assert!(rows(dir.path()).contains(&".env".to_string()));
    }

    #[test]
    fn the_same_workspace_always_produces_the_same_order() {
        let dir = workspace();
        let a = {
            let mut budget = ROW_BUDGET;
            walk_files(
                dir.path(),
                &IgnoreSet::load(dir.path(), IGNORE_FILES),
                &mut budget,
            )
        };
        let b = {
            let mut budget = ROW_BUDGET;
            walk_files(
                dir.path(),
                &IgnoreSet::load(dir.path(), IGNORE_FILES),
                &mut budget,
            )
        };
        assert_eq!(
            a, b,
            "an unstable order makes a tree jump while the user clicks"
        );
    }

    #[test]
    fn the_budget_truncates_without_failing() {
        let dir = workspace();
        let ignore = IgnoreSet::load(dir.path(), IGNORE_FILES);
        let mut budget = 1usize;
        let out = walk_files(dir.path(), &ignore, &mut budget);
        assert_eq!(out.len(), 1, "one row of budget, one row of output");
        assert_eq!(budget, 0);
    }

    #[test]
    fn a_directory_that_is_not_there_is_an_empty_list_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("not-here");
        let ignore = IgnoreSet::load(&gone, IGNORE_FILES);
        let mut budget = ROW_BUDGET;
        assert!(walk_files(&gone, &ignore, &mut budget).is_empty());
    }

    /// **The walk does not leave the workspace through a link.**
    ///
    /// A link is the only way ordinary descent can leave the root, so this is the whole
    /// containment rule for an enumeration — cheaper than canonicalising every entry, which
    /// is what `resolve_for_access` does and is right for a single named file.
    ///
    /// Both kinds are covered because they behave differently and the obvious check catches
    /// only one: a **symlink to a directory** and a **symlink to a file** (which `is_dir()`
    /// answers `false` for, so it was pushed into the list as content — an out-of-workspace
    /// file the model could then read by name).
    ///
    /// Windows junctions are the same rule by a different mechanism and cannot be created
    /// portably in a test without a privilege this project does not take; the production
    /// check is `is_link`'s, which tests the reparse-point attribute rather than
    /// `is_symlink()` for exactly that reason.
    #[test]
    fn a_link_out_of_the_workspace_is_neither_listed_nor_descended() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "TOKEN = hunter2").unwrap();

        let dir = workspace();
        let link_dir = dir.path().join("escape");
        // **The precondition is asserted, not assumed.** A test that skips and still reports
        // `ok` is the false control this repository keeps warning about, so if the link could
        // not be created the test says so *loudly* and the milestone records that it did not
        // run here — rather than passing on the strength of the assertions that follow a
        // `return`.
        //
        // A directory **junction** is used because it needs no elevation on Windows, and
        // because it is the form `FileType::is_symlink()` does not report — the case the
        // obvious check misses.
        if let Err(e) = dir_junction(outside.path(), &link_dir) {
            panic!(
                "a directory junction needs no elevation; if this fired the test cannot run: {e}"
            );
        }
        assert!(
            crate::paths::is_link(&link_dir),
            "the precondition itself: the production predicate must call a junction a link"
        );

        let ignore = IgnoreSet::load(dir.path(), IGNORE_FILES);
        let mut budget = ROW_BUDGET;
        let out = walk_files(dir.path(), &ignore, &mut budget);

        assert!(
            !out.iter()
                .any(|p| p.contains("secret.txt") || p.contains("escape")),
            "nothing behind a link may appear in the listing: {out:?}"
        );
        assert!(
            out.iter().any(|p| p == "main.rs"),
            "and the walk must still find the real files: {out:?}"
        );
    }

    /// The **file**-link half, which is the one `Path::is_dir()` answers `false` for — so
    /// before this fix it was not even descended into, it was *listed*, and the model could
    /// read an out-of-workspace file by name.
    ///
    /// Separate from the junction test because creating a file symlink on Windows needs
    /// Developer Mode, and one unavailable half must not take the other down with it.
    #[cfg(any(windows, unix))]
    #[test]
    fn a_link_to_a_file_outside_is_not_listed_as_content() {
        let outside = tempfile::tempdir().unwrap();
        let real = outside.path().join("secret.txt");
        std::fs::write(&real, "TOKEN = hunter2").unwrap();

        let dir = workspace();
        let linked = dir.path().join("secret.txt");
        if let Err(e) = file_link(&real, &linked) {
            eprintln!(
                "SKIPPED, and this milestone records it: creating a file symlink needs \
                 Developer Mode ({e}). The junction half above did run."
            );
            return;
        }

        let ignore = IgnoreSet::load(dir.path(), IGNORE_FILES);
        let mut budget = ROW_BUDGET;
        let out = walk_files(dir.path(), &ignore, &mut budget);
        assert!(
            !out.iter().any(|p| p.contains("secret.txt")),
            "a link to a file outside is not content the model may read: {out:?}"
        );
    }

    /// A directory junction, which Windows creates without elevation.
    #[cfg(windows)]
    fn dir_junction(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        let out = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "mklink /J refused: {}",
                String::from_utf8_lossy(&out.stderr)
            )))
        }
    }

    #[cfg(unix)]
    fn dir_junction(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn file_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[cfg(unix)]
    fn file_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }
}
