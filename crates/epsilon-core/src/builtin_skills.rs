//! Skills that ship inside the Epsilon binary, and are installed only on request.
//!
//! # Why this module exists
//!
//! The skills under `references/` and in other harnesses are useful, but a user
//! who installs Epsilon should not have to find them, copy a directory, and
//! guess which ones are worth keeping. This module carries a small, curated set
//! with two properties the rest of the harness can rely on:
//!
//! * **Opt-in.** Nothing here is discovered, advertised, or loadable until the
//!   user runs `/skill <name> on`. A built-in that silently entered every
//!   prompt would spend context on a procedure the user did not ask for.
//! * **Ordinary once enabled.** Enabling writes a normal
//!   `<config>/builtin-skills/<name>/SKILL.md`. Discovery, the catalog, the
//!   `skill` tool, `/skills`, and `/reload` then see a normal skill. There is no
//!   second code path for loading a built-in, which is what keeps the catalog
//!   and the loader from disagreeing.
//!
//! The installed directory is separate from `<config>/skills` so Epsilon can
//! remove a pristine copy without touching a skill the user wrote. Removal
//! refuses when the installed file differs from the bundled text: a user who
//! edited their copy owns it.
//!
//! # Adding one
//!
//! Put the body in `src/builtin_skills/<name>.md`, add an entry to
//! [`BUILTIN_SKILLS`], and keep the description one line. The tests enforce the
//! name rules, the description limit, and that every asset parses as a skill
//! after rendering.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// One skill bundled with the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinSkill {
    /// The skill name, matching the `<name>.md` asset and the installed directory.
    pub name: &'static str,
    /// One line shown in the catalog and in `/skills`.
    pub description: &'static str,
    /// Why the skill earns catalogue space, shown by `/skills` before it is enabled.
    pub why: &'static str,
    /// The Markdown body, without frontmatter.
    pub body: &'static str,
    /// Attribution for a bundled third-party skill. `None` for Epsilon-authored ones.
    pub credit: Option<&'static str>,
    /// Extra files the skill's body refers to, as `(relative path, contents)`.
    ///
    /// The installer writes these under the skill directory. A body that points at
    /// `references/audit-playbook.md` must have that file beside it, or enabling the
    /// skill produces a broken instruction — worse than not bundling it.
    pub files: &'static [(&'static str, &'static str)],
}

/// Every built-in skill, in the order `/skills` presents them.
///
/// The selection is deliberately small. A skill pays for itself only when it
/// encodes a procedure the model would otherwise improvise differently on every
/// run; a long catalogue of generic advice costs context and changes nothing.
pub const BUILTIN_SKILLS: &[BuiltinSkill] = &[
    BuiltinSkill {
        name: "verify-before-claim",
        description: "Check a claim against the code, command output, or a frame before stating it.",
        why: "A confident unchecked claim is the most expensive failure a coding agent has.",
        body: include_str!("builtin_skills/verify-before-claim.md"),
        credit: None,
        files: &[],
    },
    BuiltinSkill {
        name: "onboard-codebase",
        description: "Map an unfamiliar repository before changing it: entry points, tests, contracts.",
        why: "Prevents README-level guesses and edits made in the wrong layer.",
        body: include_str!("builtin_skills/onboard-codebase.md"),
        credit: None,
        files: &[],
    },
    BuiltinSkill {
        name: "make-a-change",
        description: "Make the smallest correct change and prove it with the narrowest relevant test.",
        why: "Stops drive-by rewrites and unproven fixes from becoming the next defect.",
        body: include_str!("builtin_skills/make-a-change.md"),
        credit: None,
        files: &[],
    },
    BuiltinSkill {
        name: "review-a-change",
        description: "Review a change twice: first against the request, then against the implementation.",
        why: "Catches work that is correct in form but answers a different request.",
        body: include_str!("builtin_skills/review-a-change.md"),
        credit: None,
        files: &[],
    },
    BuiltinSkill {
        name: "two-stage-review",
        description: "Judge a finished change for spec compliance, then for implementation quality.",
        why: "A quality pass over the wrong change is work spent on the wrong problem.",
        body: include_str!("builtin_skills/two-stage-review.md"),
        credit: None,
        files: &[],
    },
    BuiltinSkill {
        name: "ponytail",
        description: "Prefer the smallest working solution; ask whether a thing needs to exist before building it.",
        why: "Deletes speculative code instead of adding it, on any coding task.",
        body: include_str!("builtin_skills/ponytail.md"),
        credit: Some("Ponytail by DietrichGebert · MIT"),
        files: &[],
    },
    BuiltinSkill {
        name: "improve",
        description: "Survey a codebase and write self-contained plans for another executor; never edit source.",
        why: "Turns an audit into handoff plans a cheaper model can execute.",
        body: include_str!("builtin_skills/improve/SKILL.md"),
        credit: Some("improve by shadcn · MIT"),
        files: &[
            (
                "references/audit-playbook.md",
                include_str!("builtin_skills/improve/references/audit-playbook.md"),
            ),
            (
                "references/closing-the-loop.md",
                include_str!("builtin_skills/improve/references/closing-the-loop.md"),
            ),
            (
                "references/plan-template.md",
                include_str!("builtin_skills/improve/references/plan-template.md"),
            ),
        ],
    },
    BuiltinSkill {
        name: "ux-triage",
        description: "Dogfood a user-facing surface as a hard persona and separate friction from defects.",
        why: "Finds usability problems that correctness tests cannot see.",
        body: include_str!("builtin_skills/ux-triage.md"),
        credit: None,
        files: &[],
    },
    BuiltinSkill {
        name: "write-a-commit",
        description: "Turn a finished change into a clear, reviewable commit without losing work.",
        why: "A commit outlives the session; a careless one is permanent history.",
        body: include_str!("builtin_skills/write-a-commit.md"),
        credit: None,
        files: &[],
    },
    BuiltinSkill {
        name: "debug-a-failure",
        description: "Find a defect's layer before changing code: reproduce, isolate, prove, then fix.",
        why: "Guessing at the wrong layer turns one bug into several.",
        body: include_str!("builtin_skills/debug-a-failure.md"),
        credit: None,
        files: &[],
    },
    BuiltinSkill {
        name: "write-a-handoff",
        description: "Write a handoff that lets the next session resume without rediscovering state.",
        why: "Context windows end; decisions and open work should not vanish with them.",
        body: include_str!("builtin_skills/write-a-handoff.md"),
        credit: None,
        files: &[],
    },
    BuiltinSkill {
        name: "reverse-engineer-anything",
        description: "Use REA to inspect binaries, apps and sites, and report evidence honestly.",
        why: "Adds real binary, Electron and web inspection without pretending its findings are certain.",
        body: include_str!("builtin_skills/reverse-engineer-anything.md"),
        credit: None,
        files: &[],
    },
];

/// All built-ins, as a slice. Kept as a function so callers do not depend on the
/// static's spelling and tests have one thing to iterate.
pub fn all() -> &'static [BuiltinSkill] {
    BUILTIN_SKILLS
}

/// Find a built-in by name, case-insensitively.
///
/// A user who types `/skill Verify-Before-Claim on` meant the same skill as
/// `/skill verify-before-claim on`; rejecting it would be a papercut with no
/// safety value. The on-disk name is always the lower-case one.
pub fn find(name: &str) -> Option<&'static BuiltinSkill> {
    let wanted = name.trim();
    BUILTIN_SKILLS
        .iter()
        .find(|skill| skill.name.eq_ignore_ascii_case(wanted))
}

/// The directory built-ins are installed under for a given config directory.
///
/// Deliberately not `<config>/skills`: that directory belongs to the user, and
/// Epsilon must be able to distinguish a copy it may remove from a skill the
/// user wrote or edited.
pub fn root_in(config_dir: &Path) -> PathBuf {
    config_dir.join("builtin-skills")
}

/// Render the `SKILL.md` contents for one built-in.
///
/// The description is double-quoted because several descriptions contain a
/// colon. The frontmatter parser used by discovery strips one pair of matching
/// quotes; it does not implement the rest of YAML, so keep new descriptions free
/// of double quotes and backslashes.
pub fn rendered(skill: &BuiltinSkill) -> String {
    format!(
        "---\nname: {}\ndescription: \"{}\"\n---\n\n{}\n",
        skill.name,
        skill.description,
        skill.body.trim_end()
    )
}

/// Where one installed built-in lives.
fn file_in(config_dir: &Path, name: &str) -> PathBuf {
    root_in(config_dir).join(name).join("SKILL.md")
}

/// Whether a built-in is installed in this config directory.
pub fn installed(name: &str, config_dir: &Path) -> bool {
    find(name).is_some_and(|skill| file_in(config_dir, skill.name).is_file())
}

/// Install a built-in, returning the path of the `SKILL.md` that now exists.
///
/// Reinstalling an identical copy is a success: `/skill <name> on` must be
/// idempotent. A copy whose bytes differ is refused rather than overwritten —
/// the user may have edited it, and a settings-like write must not eat that
/// work. Missing files are filled in, so a user who deleted one bundled
/// reference gets it back without losing edits to the others.
pub fn install(name: &str, config_dir: &Path) -> Result<PathBuf, String> {
    let skill = find(name).ok_or_else(|| format!("no built-in skill named '{name}' is bundled"))?;
    let main = file_in(config_dir, skill.name);
    for (path, body) in expected_files(skill, config_dir) {
        if path.is_file() {
            let existing = std::fs::read_to_string(&path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            if existing != body {
                return Err(format!(
                    "{} already exists and differs from the bundled copy; \
                     edit or remove it yourself, then enable it again",
                    path.display()
                ));
            }
        }
    }
    for (path, body) in expected_files(skill, config_dir) {
        if path.is_file() {
            continue;
        }
        let dir = path
            .parent()
            .ok_or_else(|| "install path has no parent".to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        write_atomic(&path, &body)?;
    }
    Ok(main)
}

/// Every file an installed built-in owns: the rendered `SKILL.md` first, then
/// the files its body refers to.
fn expected_files(skill: &BuiltinSkill, config_dir: &Path) -> Vec<(PathBuf, String)> {
    let dir = root_in(config_dir).join(skill.name);
    let mut files = vec![(dir.join("SKILL.md"), rendered(skill))];
    for (relative, body) in skill.files {
        files.push((dir.join(relative), (*body).to_string()));
    }
    files
}

/// Stage then rename, so an interrupted write never leaves a half file that
/// discovery would report as unreadable on every launch.
fn write_atomic(path: &Path, body: &str) -> Result<(), String> {
    let staging = unique_staging(path);
    std::fs::write(&staging, body).map_err(|e| format!("write {}: {e}", staging.display()))?;
    if let Err(e) = std::fs::rename(&staging, path) {
        let _ = std::fs::remove_file(&staging);
        return Err(format!("replace {}: {e}", path.display()));
    }
    Ok(())
}

/// Remove a pristine installed copy. Returns `true` when a file was removed.
///
/// An edited copy is left in place and reported. Deleting it would lose the
/// user's work; pretending it was removed would make `/skills` lie. Every owned
/// file is checked before any is deleted, so a half-removed skill cannot be the
/// result of a refusal on the third file.
pub fn remove(name: &str, config_dir: &Path) -> Result<bool, String> {
    let skill = find(name).ok_or_else(|| format!("no built-in skill named '{name}' is bundled"))?;
    let main = file_in(config_dir, skill.name);
    if !main.exists() {
        return Ok(false);
    }
    let files = expected_files(skill, config_dir);
    for (path, body) in &files {
        if !path.exists() {
            continue;
        }
        let existing =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if &existing != body {
            return Err(format!(
                "{} has been edited, so it was left in place; remove it yourself to disable it",
                path.display()
            ));
        }
    }
    for (path, _) in &files {
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| format!("remove {}: {e}", path.display()))?;
        }
    }
    // Child directories (`references/`) and then the skill directory are ours if
    // empty. A user-added file makes `remove_dir` fail, and leaving the directory
    // is correct: the extra file is not ours to delete.
    if let Some(dir) = main.parent() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                if e.path().is_dir() {
                    let _ = std::fs::remove_dir(e.path());
                }
            }
        }
        let _ = std::fs::remove_dir(dir);
    }
    Ok(true)
}

/// A staging name that cannot collide between threads in one process.
fn unique_staging(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "SKILL.md".into());
    name.push(format!(".tmp.{}.{n}", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills;

    fn temp_config() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp config dir")
    }

    #[test]
    fn names_are_unique_valid_and_have_one_line_descriptions() {
        let mut seen = std::collections::BTreeSet::new();
        for skill in all() {
            assert!(seen.insert(skill.name), "duplicate name: {}", skill.name);
            assert!(
                skill.name.len() <= 64
                    && skill
                        .name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                    && !skill.name.starts_with('-')
                    && !skill.name.ends_with('-')
                    && !skill.name.contains("--"),
                "invalid skill name: {}",
                skill.name
            );
            assert!(
                !skill.description.contains('\n'),
                "{} description must be one line",
                skill.name
            );
            assert!(
                !skill.description.contains('"') && !skill.description.contains('\\'),
                "{} description is embedded in double quotes without escaping",
                skill.name
            );
            assert!(
                skill.description.chars().count() <= 1024,
                "{} description is over the spec limit",
                skill.name
            );
            assert!(!skill.why.trim().is_empty(), "{} has no why", skill.name);
            assert!(
                !skill.body.trim().is_empty(),
                "{} has an empty body",
                skill.name
            );
            if let Some(credit) = skill.credit {
                assert!(
                    !credit.trim().is_empty() && !credit.contains('\n'),
                    "{} has a malformed credit",
                    skill.name
                );
            }
            for (relative, body) in skill.files {
                assert!(
                    !relative.starts_with('/')
                        && !relative.contains("..")
                        && !relative.contains('\\'),
                    "{} file path must stay inside its skill dir: {relative}",
                    skill.name
                );
                assert!(
                    !body.trim().is_empty(),
                    "{} file {relative} is empty",
                    skill.name
                );
            }
        }
    }

    #[test]
    fn an_installed_builtin_is_discovered_as_an_ordinary_skill() {
        let tmp = temp_config();
        let path = install("verify-before-claim", tmp.path()).expect("install");
        assert!(path.is_file());
        let roots = [root_in(tmp.path())];
        let found = skills::discover(&roots);
        let skill = found
            .iter()
            .find(|s| s.name == "verify-before-claim")
            .expect("discovered");
        assert_eq!(
            skill.description,
            find("verify-before-claim").unwrap().description
        );
        assert!(!skill.disable_model_invocation);
        // The rendered file opens with frontmatter and the body follows it.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.starts_with("---\nname: verify-before-claim\n"));
        assert!(raw.contains("description: \""));
    }

    #[test]
    fn nothing_is_installed_until_the_user_asks() {
        let tmp = temp_config();
        assert!(!root_in(tmp.path()).exists());
        assert!(skills::discover(&[root_in(tmp.path())]).is_empty());
    }

    #[test]
    fn installing_twice_is_idempotent() {
        let tmp = temp_config();
        let first = install("make-a-change", tmp.path()).expect("first");
        let second = install("make-a-change", tmp.path()).expect("second");
        assert_eq!(first, second);
    }

    #[test]
    fn an_edited_copy_is_never_overwritten_or_removed() {
        let tmp = temp_config();
        let path = install("write-a-commit", tmp.path()).expect("install");
        std::fs::write(&path, "user notes").expect("edit");
        let refused = install("write-a-commit", tmp.path()).expect_err("must refuse overwrite");
        assert!(refused.contains("differs"), "{refused}");
        let kept = remove("write-a-commit", tmp.path()).expect_err("must refuse delete");
        assert!(kept.contains("edited"), "{kept}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user notes");
    }

    #[test]
    fn bundled_reference_files_install_and_are_removed_with_the_skill() {
        let tmp = temp_config();
        install("improve", tmp.path()).expect("install");
        let dir = root_in(tmp.path()).join("improve");
        for relative in [
            "references/audit-playbook.md",
            "references/closing-the-loop.md",
            "references/plan-template.md",
        ] {
            assert!(
                dir.join(relative).is_file(),
                "missing bundled file {relative}"
            );
        }
        assert!(remove("improve", tmp.path()).expect("remove"));
        assert!(!dir.exists(), "the owned reference files went with it");
    }

    #[test]
    fn an_edited_reference_file_blocks_removal_without_deleting_anything() {
        let tmp = temp_config();
        install("improve", tmp.path()).expect("install");
        let dir = root_in(tmp.path()).join("improve");
        let edited = dir.join("references/plan-template.md");
        std::fs::write(&edited, "my own template").expect("edit");
        let kept = remove("improve", tmp.path()).expect_err("must refuse delete");
        assert!(kept.contains("edited"), "{kept}");
        assert!(dir.join("SKILL.md").is_file(), "the skill stayed intact");
        assert_eq!(std::fs::read_to_string(&edited).unwrap(), "my own template");
    }

    #[test]
    fn a_missing_bundled_file_is_repaired_without_touching_the_rest() {
        let tmp = temp_config();
        install("improve", tmp.path()).expect("install");
        let dir = root_in(tmp.path()).join("improve");
        let removed = dir.join("references/closing-the-loop.md");
        std::fs::remove_file(&removed).expect("delete one reference");
        install("improve", tmp.path()).expect("repair");
        assert!(removed.is_file(), "the missing file came back");
    }

    #[test]
    fn a_pristine_copy_can_be_removed_and_removal_is_idempotent() {
        let tmp = temp_config();
        install("debug-a-failure", tmp.path()).expect("install");
        assert!(remove("debug-a-failure", tmp.path()).expect("remove"));
        assert!(!remove("debug-a-failure", tmp.path()).expect("second remove"));
        assert!(!root_in(tmp.path()).join("debug-a-failure").exists());
    }

    #[test]
    fn case_insensitive_lookup_uses_the_on_disk_name() {
        let skill = find("  VERIFY-BEFORE-CLAIM ").expect("found");
        assert_eq!(skill.name, "verify-before-claim");
        assert!(find("not-a-skill").is_none());
    }
}
