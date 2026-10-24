//! Path resolution shared by every crate that must confine a path to a workspace.
//!
//! This started in `epsilon-tools` (where the tools needed it) and moved here (M74) when the
//! context-reference expander needed the same rule: one implementation, so a file the tools
//! consider inside the workspace can never be one the expander considers outside it.
//!
//! # Two functions, and the difference is the whole point
//!
//! * [`lexically_normalize`] answers "where does this spelling point, ignoring the
//!   filesystem". It is what a **planner key** wants: a scheduling identity must be
//!   cheap, stable, and independent of whether the file exists yet.
//! * [`resolve_for_access`] answers "may we touch this", and it follows symlinks.
//!   It is what every **enforcement** site wants.
//!
//! Using the first where the second is meant is how a workspace boundary stops being a
//! boundary, and using the second where the first is meant makes the planner do
//! filesystem syscalls and change its answer when a symlink is retargeted.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Where one raw path argument resolved, and whether it may be touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The **lexically** normalized path.
    ///
    /// Lexical on purpose, and identical to what [`lexically_normalize`] returns.
    /// Callers that use this as a scheduling key must not have their identity change
    /// when a symlink is retargeted, and callers that use it to open a file have
    /// already consulted [`Resolution::inside`].
    pub path: PathBuf,
    /// Whether the path is inside the workspace **after following symlinks**.
    pub inside: bool,
    /// The canonical path, whenever the filesystem could establish one: the
    /// resolved spelling of [`Resolution::path`], with every symlink on the way
    /// followed.
    ///
    /// `None` in exactly the two cases [`resolve_for_access`] documents — the root
    /// was unresolvable, or the path itself could not be resolved. A caller
    /// judging anything about the *file* rather than about the spelling must read
    /// this: a name-based check run against `path` alone lets a link with an
    /// innocent name decide for itself whether it is a secret.
    pub real: Option<PathBuf>,
    /// The final component carried a spelling Windows resolves to a **different file**
    /// (ledger A33): a trailing dot or space, or a `:` alternate-data-stream suffix.
    ///
    /// `Some` means the spelling cannot be judged by reading it, so the path is refused.
    /// It is a separate field rather than a special value of [`Self::inside`] because
    /// "you may not touch this" and "this is outside your workspace" are different
    /// sentences, and a refusal that lies about which one it is teaches users that the
    /// messages mean nothing.
    pub deceptive: bool,
}

impl Resolution {
    /// Where a path that spelled its way inside the workspace actually pointed.
    ///
    /// The difference between `Refused: 'link' resolves outside the workspace` and
    /// a message that says *why*: the user asked about a file with an innocent
    /// name, and the refusal has to name where it really went. `None` for every
    /// other outcome — a path that stayed inside has nothing to disclose, and one
    /// refused for being lexically outside never resolved at all.
    pub fn escaped_to(&self) -> Option<&Path> {
        if self.inside {
            None
        } else {
            self.real.as_deref()
        }
    }
}

/// Resolve `raw` against `root`, normalizing `.`/`..` WITHOUT touching the filesystem,
/// then report whether the result stayed inside the workspace.
///
/// Lexical normalization only: see [`resolve_for_access`] for the version that also
/// follows symlinks, and for when you want that one.
pub fn lexically_normalize(root: &Path, raw: &str) -> (PathBuf, bool) {
    let candidate = Path::new(raw);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let mut normalized = PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::ParentDir => {
                if !normalized.pop() {
                    // Escaping above the root at join time: mark outside immediately.
                    return (joined, false);
                }
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    let inside = normalized.starts_with(root);
    (normalized, inside)
}

/// Resolve `raw` for **access**, following symlinks before deciding containment.
///
/// The lexical answer is necessary and not sufficient. A repository is untrusted input
/// the moment a user opens one, and a checked-in
/// `secrets -> /home/user/.aws` is the whole attack in one line: the spelling is inside
/// the workspace, so no `external_dir` ask is ever raised, and the read/write tool
/// follows it to the real thing. `write` is `Allow` by default, so that is a silent
/// write outside the workspace — with no prompt, because every layer that would have
/// asked agreed the path was fine.
///
/// So: canonicalize the deepest prefix of the path that exists, re-append the
/// components that do not exist yet, and compare *that* against the canonicalized root.
/// A file that does not exist yet is the normal case for `write`, and a path that
/// cannot be canonicalized at all fails closed.
///
/// # The one case that keeps the old answer
///
/// When the **root** itself cannot be canonicalized, the symlink answer falls back to
/// the lexical one. That is not a hole: a root that cannot be resolved has no directory
/// behind it, so there is nothing inside it for a symlink to escape *from* — every read
/// and every write under it fails with `ENOENT` regardless of what this returns. The
/// alternative, reporting every path as outside, would turn an ordinary
/// "the directory is gone" into a confinement error and teach users that the message
/// means nothing.
pub fn resolve_for_access(root: &Path, raw: &str) -> Resolution {
    // **A33: a spelling Windows resolves to a different file is refused before anything
    // else looks at it.**
    //
    // `.env.`, `.env ` and `.env::$DATA` are not `.env`. On Windows they *open* `.env`:
    // Win32 strips a trailing dot and a trailing space, and `::$DATA` is the unnamed
    // alternate data stream. So a name-based policy — `is_sensitive`'s exact-name list,
    // the `^.*\.env$` rule, the `@` expander — evaluated them, decided they were an
    // ordinary file, and returned `Rule::new("read", "*", Allow)`. The whole defence for
    // the model-facing tools is those rules, and every one of them was bypassable by
    // adding a character to the end of the name.
    //
    // **Windows only, and that is the technically correct scope rather than a
    // convenience.** On POSIX none of these are stripped: `.env.` is a genuinely
    // different file, and refusing it would break a legitimate path. The rule exists
    // because of what the *open* does, so it belongs where the stripping happens.
    //
    // Here, before policy evaluation and before any syscall, because that is the whole
    // point: a choke point above every consumer rather than a fix in each of them.
    if deceptive_name_reason(Path::new(raw)).is_some() {
        return Resolution {
            path: Path::new(raw).to_path_buf(),
            inside: false,
            real: None,
            deceptive: true,
        };
    }
    let (path, lexically_inside) = lexically_normalize(root, raw);
    if !lexically_inside {
        return Resolution {
            path,
            inside: false,
            real: None,
            deceptive: false,
        };
    }
    let real_root = match canonical(root) {
        Ok(real) => real,
        // The root is unresolvable — see the doc comment. The lexical answer stands.
        Err(_) => {
            return Resolution {
                path,
                inside: true,
                real: None,
                deceptive: false,
            };
        }
    };
    let real_path = match canonical_existing_prefix(&path) {
        Ok(real) => real,
        // The root resolved but the path did not: a permission error, a symlink
        // loop, or a component that exists and cannot be read. Fail closed.
        Err(_) => {
            return Resolution {
                path,
                inside: false,
                real: None,
                deceptive: false,
            };
        }
    };
    let inside = real_path.starts_with(&real_root);
    Resolution {
        path,
        inside,
        real: Some(real_path),
        deceptive: false,
    }
}

/// What a final component's spelling resolves to on Windows, if it is not the name.
///
/// `Some` names the reason so a refusal can say it, and both the gate and the message read
/// **this** function rather than each deciding: a check that lives in one place and a
/// message that lives in another is the same class as two copies of a rule.
///
/// See the A33 block in [`resolve_for_access`] for why this exists and why it is Windows-only.
pub fn deceptive_name_reason(raw: &Path) -> Option<&'static str> {
    if !cfg!(windows) {
        return None;
    }
    // A drive-relative path like `C:` has no file name; nothing to judge.
    let name = raw.file_name()?.to_str()?;
    if name.contains(':') {
        // `::$DATA` is the *unnamed* stream — the spelling that reads exactly like the
        // file. A named stream (`file:secret`) is also not the file, and is also refused.
        return Some("an NTFS alternate data stream");
    }
    if name.ends_with('.') {
        return Some("a trailing dot, which Windows strips before opening");
    }
    if name.ends_with(' ') {
        return Some("a trailing space, which Windows strips before opening");
    }
    None
}

/// Whether `path` is a **link** rather than a real file or directory.
///
/// # Why this exists, given that [`resolve_for_access`] already follows links
///
/// Because the two answer different questions, and a *walk* was using the wrong one.
///
/// `resolve_for_access` is the enforcement primitive: "may we touch this path?", answered by
/// canonicalising and comparing against the root. It costs a syscall per path, which is right
/// for a boundary check on one named file and wrong for enumerating five thousand.
///
/// Enumeration does not need it. A walk that descends from the root through **real**
/// directories cannot leave the root — the only way out is a link, because that is the only
/// thing on the way that points somewhere else. So the cheap and sufficient rule for a walker
/// is: **do not descend into a link.**
///
/// # What that was worth, before this existed
///
/// Both walkers classified entries with `Path::is_dir()`, which calls `fs::metadata` and
/// therefore **follows links**, then recursed into the answer. A repository containing
/// `docs -> C:\Users\<you>\Documents` (or `link -> ~/.aws`) had a plain `grep` walk it, print
/// the matching lines into the tool result, the transcript and the durable session log — with
/// **no permission ask**, because `gate_subject` saw only a workspace-relative name and no
/// `external_dir` capability was ever raised. Nothing was out of place from the harness's point
/// of view; every path it handled began with the workspace.
///
/// The project's own review named this in 2026-09 and it was never implemented:
/// `audit/evidence/f09-path-policy-map.md:26,29` — *"do not follow discovered
/// links/reparse directories implicitly."*
///
/// # Why the Windows branch is not `is_symlink()`
///
/// A **junction** is a reparse point that `FileType::is_symlink()` reports as an ordinary
/// directory, and creating one needs no privilege — so it is both the case that matters most
/// on this platform and the one the obvious check misses. The attribute test covers symlinks
/// and junctions together, from the standard library, with no new dependency.
pub fn is_link(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        // A path that cannot be stat'd is not a link, and the walker's own read already
        // decided what to do with it. Answering `false` here keeps this a predicate and
        // not a second error path.
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// A canonical path as a person should read it.
///
/// `std::fs::canonicalize` on Windows returns the **verbatim** form: the same path
/// with a `\\?\` prefix, which is what the syscall layer wants and what a sentence
/// must not contain. A refusal that says "it is a link to \\?\C:\Users\…\id_rsa"
/// is technically true and useless to the person who has to act on it.
///
/// The prefix is stripped, and a verbatim UNC path is rewritten to the ordinary
/// `\\server\share` form rather than left as `\\?\UNC\server\share`. Non-Windows
/// canonical paths are already the readable spelling and pass through untouched.
pub fn display_canonical(path: &Path) -> String {
    let s = path.display().to_string();
    let Some(rest) = s.strip_prefix(r"\\?\") else {
        return s;
    };
    match rest.strip_prefix(r"UNC\") {
        Some(server_share) => format!(r"\\{server_share}"),
        None => rest.to_string(),
    }
}

/// Canonicalize `path` itself.
fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// Canonicalize the deepest existing prefix of `path`, re-appending the components
/// that do not exist yet.
///
/// This is what makes a *new* file judgeable: `write` to `src/new.rs` where `src`
/// exists must resolve to `<real root>/src/new.rs`, and only the directory needs to be
/// on disk. Walking up one component at a time also means a symlink anywhere in the
/// existing part of the path is followed, which is the entire point.
fn canonical_existing_prefix(path: &Path) -> std::io::Result<PathBuf> {
    let mut tail: Vec<OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        match std::fs::canonicalize(&cursor) {
            Ok(real) => {
                let mut out = real;
                for part in tail.iter().rev() {
                    out.push(part);
                }
                return Ok(out);
            }
            // Only a *missing* prefix is skippable. `NotADirectory` is the same
            // situation with a file in the middle: macOS returns ENOTDIR for
            // `blocked/nested.txt` when `blocked` is a file, while Windows returns
            // NotFound. Walking up in both cases lets the eventual write fail at
            // create_dir_all instead of misreporting an in-workspace path as an escape.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                let Some(name) = cursor.file_name().map(OsString::from) else {
                    return Err(e);
                };
                tail.push(name);
                let Some(parent) = cursor.parent().map(Path::to_path_buf) else {
                    return Err(e);
                };
                cursor = parent;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::dir_link;

    #[test]
    fn lexical_traversal_is_contained() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let (p, inside) = lexically_normalize(&root, "a/../b.txt");
        assert!(inside);
        assert_eq!(p, root.join("b.txt"));

        let (_, inside) = lexically_normalize(&root, "../outside.txt");
        assert!(!inside);

        let elsewhere = if cfg!(windows) {
            r"C:\elsewhere\x"
        } else {
            "/elsewhere/x"
        };
        let (resolved, inside) = lexically_normalize(&root, elsewhere);
        assert!(!inside, "absolute path outside the root must be flagged");
        assert_eq!(resolved, Path::new(elsewhere));
    }

    #[test]
    fn a_file_that_does_not_exist_yet_is_still_inside() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("src")).unwrap();
        // The whole point of the prefix walk: `write` creates this file, so
        // canonicalizing the leaf would fail for the most common write there is.
        let r = resolve_for_access(&root, "src/brand_new.rs");
        assert!(r.inside, "{r:?}");
        assert_eq!(r.path, root.join("src").join("brand_new.rs"));
        assert_eq!(r.escaped_to(), None);
        // The expectation is canonicalized too, and not by string surgery: on
        // Windows `canonicalize` answers the `\\?\` verbatim form, so a test that
        // spells the expectation with `join` disagrees with the code while both
        // are right. `inside` is unaffected either way — it compares two
        // canonicalized paths, which is why this test passed at all.
        let real_root = std::fs::canonicalize(&root).unwrap();
        assert_eq!(r.real, Some(real_root.join("src").join("brand_new.rs")));
    }

    #[test]
    fn a_canonical_path_reads_the_way_a_person_would_write_it() {
        // The verbatim prefix is correct for the syscall layer and wrong in a
        // message, and the message is where this path is shown.
        assert_eq!(
            display_canonical(Path::new(r"\\?\C:\work\creds")),
            r"C:\work\creds"
        );
        assert_eq!(
            display_canonical(Path::new(r"\\?\UNC\srv\share\creds")),
            r"\\srv\share\creds"
        );
        assert_eq!(display_canonical(Path::new("/home/u/.aws")), "/home/u/.aws");
    }

    #[test]
    fn a_symlink_pointing_outside_is_reported_with_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("id_rsa"), "PRIVATE KEY").unwrap();
        if !dir_link(&outside, &root.join("link")) {
            // The OS refused to create a link; the other tests in this file and the
            // integration tests cover what can be covered here.
            eprintln!("skipped: this platform would not create a directory link");
            return;
        }

        let r = resolve_for_access(&root, "link/id_rsa");
        assert!(
            !r.inside,
            "a symlink out of the workspace must be outside: {r:?}"
        );
        let target = r
            .escaped_to()
            .expect("the refusal must be able to name where it actually pointed");
        let real_outside = std::fs::canonicalize(&outside).unwrap();
        assert!(
            target.starts_with(&real_outside) || target.ends_with("id_rsa"),
            "the reported target should be the real file: {target:?}"
        );
    }

    #[test]
    fn a_link_that_stays_inside_is_still_allowed() {
        // The fix must not be "refuse anything reached through a link": a workspace
        // that links its own subdirectories is ordinary, and breaking it would train
        // users to work around the boundary.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::write(root.join("real/a.rs"), "x").unwrap();
        if !dir_link(&root.join("real"), &root.join("link")) {
            eprintln!("skipped: this platform would not create a directory link");
            return;
        }
        let r = resolve_for_access(&root, "link/a.rs");
        assert!(r.inside, "{r:?}");
    }

    #[test]
    fn lexical_traversal_still_wins_over_the_filesystem() {
        // `..` out of the root is refused before any symlink is consulted, so a link
        // cannot be used to make `..` look safe.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let r = resolve_for_access(&root, "../root/../etc/passwd");
        assert!(!r.inside, "{r:?}");
    }

    #[test]
    fn a_file_in_the_middle_is_still_a_lexical_inside_path() {
        // A file used as a directory is a failed write, not an escape. macOS returns
        // ENOTDIR here and Windows returns NotFound; both must keep the lexical answer.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("blocked"), "not a directory").unwrap();
        let r = resolve_for_access(&root, "blocked/nested.txt");
        assert!(r.inside, "{r:?}");
        assert!(r.escaped_to().is_none(), "{r:?}");
    }

    /// **A33: a spelling Windows resolves to a different file never reaches policy.**
    ///
    /// `.env.`, `.env ` and `.env::$DATA` are each a *different string* from `.env` and
    /// each a *different file* on Windows: Win32 strips a trailing dot and a trailing
    /// space, and `::$DATA` names the unnamed alternate data stream. So every name-based
    /// check — `is_sensitive`'s exact list, the `^.*\.env$` rule, the `@` expander —
    /// read them, decided "ordinary file", and fell through to `Rule::new("read", "*",
    /// Allow)`. The whole defence for the model-facing tools is those rules, and each one
    /// was bypassable by appending a character.
    ///
    /// **Control:** remove the `deceptive_name_reason` early return and every assertion
    /// below inverts — the path resolves, `inside` is true, and the name compares unequal
    /// to `.env` at every layer that mattered.
    #[test]
    #[cfg(windows)]
    fn a_name_windows_resolves_elsewhere_is_refused_before_policy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".env"), "SECRET=hunter2").unwrap();

        for spelling in [".env.", ".env ", ".env::$DATA", ".env:stream"] {
            let r = resolve_for_access(&root, spelling);
            assert!(
                !r.inside,
                "{spelling:?} must not resolve as an ordinary workspace file: {r:?}"
            );
            assert!(
                r.deceptive,
                "{spelling:?} must be refused for what it IS, so the message can say so"
            );
            assert!(
                r.escaped_to().is_none(),
                "{spelling:?} did not go anywhere -- refusing it as an escape would be a lie"
            );
            assert!(
                deceptive_name_reason(Path::new(spelling)).is_some(),
                "and the reason must be available to the refusal message"
            );
        }
    }

    /// The other direction, which is what stops the rule becoming a denial of service:
    /// **an ordinary name is untouched.** Everything in this list is a file a developer
    /// writes, and a boundary that refuses them trains users to work around the boundary.
    #[test]
    fn an_ordinary_name_is_not_touched_by_the_spelling_rule() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        for name in [".env", ".envrc", "env.example", "a.env.bak", "main.rs"] {
            std::fs::write(root.join(name), "x").unwrap();
            let r = resolve_for_access(&root, name);
            assert!(
                r.inside,
                "{name:?} is an ordinary file and must stay usable: {r:?}"
            );
            assert!(
                !r.deceptive,
                "{name:?} is not a spelling Windows resolves elsewhere"
            );
        }
    }

    /// Windows-only by construction, and worth pinning: on POSIX `.env.` is a **different
    /// real file**, not an alias for `.env`, so refusing it there would break a legitimate
    /// path for no security gain. The rule follows the behaviour of the open, not a habit.
    #[cfg(not(windows))]
    #[test]
    fn posix_keeps_the_dot_because_there_it_is_another_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".env."), "x").unwrap();
        let r = resolve_for_access(&root, ".env.");
        assert!(r.inside, "{r:?}");
        assert!(!r.deceptive);
    }
}
