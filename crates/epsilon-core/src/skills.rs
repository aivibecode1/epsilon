//! Agent Skills spec support (ADR-015): SKILL.md directories with YAML-lite frontmatter,
//! progressive disclosure — catalog (name + description) in the prompt, body on demand.

use std::path::{Path, PathBuf};

use epsilon_contracts::tool::ToolFailure;

/// Name rules per the spec (pi parity): <=64 chars, lowercase a-z/0-9/hyphen,
/// no leading/trailing/consecutive hyphens.
const MAX_NAME: usize = 64;
/// Description limit per the spec.
const MAX_DESC: usize = 1024;

/// One discovered skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Validated skill name (directory name unless frontmatter overrides).
    pub name: String,
    /// One-line purpose shown in the catalog.
    pub description: String,
    /// Directory containing SKILL.md.
    pub dir: PathBuf,
    /// When true, only slash-command style invocation may load it (not model choice).
    pub disable_model_invocation: bool,
}

impl Skill {
    /// Full SKILL.md contents minus the frontmatter block.
    pub fn load_body(&self) -> Result<String, ToolFailure> {
        let raw = std::fs::read_to_string(self.dir.join("SKILL.md"))
            .map_err(|e| ToolFailure::msg(format!("cannot read skill '{}': {e}", self.name)))?;
        Ok(strip_frontmatter(&raw))
    }
}

/// Parse `---` delimited frontmatter into key/value pairs (flat strings only).
/// Returns the pairs and the body AFTER the closing marker.
fn parse_frontmatter(raw: &str) -> (Vec<(String, String)>, String) {
    if raw.lines().next().map(str::trim) != Some("---") {
        return (Vec::new(), raw.to_string());
    }
    let mut pairs = Vec::new();
    let mut consumed = 1usize; // opening ---
    let mut lines = raw.lines();
    lines.next();
    for line in lines.by_ref() {
        consumed += 1;
        if line.trim() == "---" {
            let body: Vec<&str> = raw.lines().skip(consumed).collect();
            return (pairs, body.join("\n"));
        }
        if let Some((k, v)) = line.split_once(':') {
            pairs.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    (Vec::new(), raw.to_string()) // unterminated frontmatter: treat whole file as body
}

fn strip_frontmatter(raw: &str) -> String {
    parse_frontmatter(raw).1
}

/// What discovery found, and what it could not look at.
///
/// **Why this type exists.** Both ways `discover` used to lose information were silent, and
/// a silent failure is worse than a missing feature — a missing one does not lie. An
/// unreadable root read as an empty one, and a shadowed global skill read as an ordinary
/// one. `Vec<Skill>` could express neither fact, so both were dropped on the floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// The skills that passed every gate, sorted by name as [`discover`] sorts them.
    pub skills: Vec<Skill>,
    /// One line per thing discovery could not do, or chose not to do, in the order it
    /// happened. **Empty means discovery genuinely completed** — which is not the same
    /// claim as "there was nothing to report", and is the distinction the old signature
    /// could not express.
    pub notices: Vec<String>,
}

/// Discover skills across root directories (each scanned one level deep for `SKILL.md`).
///
/// **Name collisions: last root wins, which is NOT what pi does.** pi keeps the
/// *first* skill registered under a name and records a collision diagnostic naming
/// both paths (`pi-0.85.1/packages/coding-agent/src/core/skills.ts:432-448`: the
/// `set` is in the `else` of `if (existing)`, so a later duplicate is reported and
/// discarded). This implementation replaces the earlier entry, so a project-level
/// skill overrides a global one of the same name.
///
/// The divergence is deliberate and recorded in `audit/03-port-plan.md` (capability
/// matrix, "Skill discovery/frontmatter/collisions"). An earlier version of this
/// comment claimed the behaviour was "mirroring pi", which was false in both
/// directions — pi neither lets the later skill win nor stays silent.
///
/// **pi's collision diagnostic now exists, in a sibling.** pi reports `winnerPath` and
/// `loserPath` (`skills.ts:434-443`); [`discover_with_notices`] names both paths and says
/// which one won, and `epsilon-runtime`'s composition path pushes the line into the same
/// startup-notice sink its other diagnostics use — so the terminal and the desktop both
/// read it. This wrapper keeps the `Vec<Skill>` signature because a caller that has no
/// surface to print on should not be forced to invent one.
pub fn discover(roots: &[PathBuf]) -> Vec<Skill> {
    discover_with_notices(roots).skills
}

/// Discover skills and report what could not be discovered — the channel [`discover`]
/// lacked. See W11 in `audit/evidence/port-gap-recheck-2026-09-21.md`.
///
/// # What earns a notice
///
/// One rule: a notice is for something a person wrote, or has on disk, that this run did
/// not use. Everything below is that case, and each one was a silent loss:
///
/// * **a root that could not be read** — named, with the reason. A root that is simply
///   *absent* is not a notice: most workspaces have no `.epsilon/skills` at all, and a
///   channel that speaks on every launch is a channel people learn to stop reading.
/// * **an entry the directory could not hand over**, and **a `SKILL.md` that is there but
///   cannot be read** — the latter is not the same as a directory with no `SKILL.md`,
///   which is a skills root holding something else and is not a defect.
/// * **a skill the name or description rules reject** — the author wrote it and it will
///   never load. Silence here is what makes "my skill does nothing" unfixable.
/// * **a name collision** — both paths and the winner. See [`discover`] for why the later
///   root wins; that behaviour is unchanged, it is now only announced.
///
/// What deliberately does **not** produce one: a plain file in a skills root, and a
/// directory that is not a skill. Those are ordinary trees, and a notice per unrelated
/// entry would bury the ones that matter.
pub fn discover_with_notices(roots: &[PathBuf]) -> Discovered {
    let mut out: Vec<Skill> = Vec::new();
    let mut notices: Vec<String> = Vec::new();
    for root in roots {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) => {
                if !root_is_absent(root, &e) {
                    notices.push(format!("skills: cannot read root {}: {e}", root.display()));
                }
                continue;
            }
        };
        for entry in entries {
            let e = match entry {
                Ok(e) => e,
                Err(e) => {
                    // An entry the directory could not hand over is something discovery
                    // could not do, which is the whole subject of this function.
                    notices.push(format!(
                        "skills: cannot read an entry under {}: {e}",
                        root.display()
                    ));
                    continue;
                }
            };
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let skill_file = dir.join("SKILL.md");
            let raw = match std::fs::read_to_string(&skill_file) {
                Ok(raw) => raw,
                Err(e) => {
                    // Absent is "not a skill, not a problem"; present and unreadable is a
                    // skill somebody wrote that will silently not load.
                    if e.kind() != std::io::ErrorKind::NotFound {
                        notices.push(format!("skills: cannot read {}: {e}", skill_file.display()));
                    }
                    continue;
                }
            };
            let (fm, _) = parse_frontmatter(&raw);
            let get = |k: &str| fm.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());

            let dir_name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let name = get("name").unwrap_or(dir_name);
            if !valid_name(&name) {
                notices.push(format!(
                    "skills: ignoring {}: '{name}' is not a valid skill name (lowercase a-z, \
                     0-9 and hyphens, no leading, trailing or doubled hyphen, at most \
                     {MAX_NAME} characters)",
                    dir.display()
                ));
                continue;
            }
            let description = get("description").unwrap_or_default();
            if description.is_empty() {
                notices.push(format!(
                    "skills: ignoring {}: no description, and the description is the only line \
                     the catalog shows before a skill is invoked",
                    dir.display()
                ));
                continue;
            }
            if description.len() > MAX_DESC {
                notices.push(format!(
                    "skills: ignoring {}: description is {} bytes, over the {MAX_DESC}-byte limit",
                    dir.display(),
                    description.len()
                ));
                continue;
            }
            let skill = Skill {
                name,
                description,
                dir,
                disable_model_invocation: invocation_flag(get("disable-model-invocation")),
            };
            // Shadowing: replace the earlier-root skill with the same name. This is a
            // deliberate divergence from pi (see the doc comment above): pi keeps the
            // first and reports a collision instead. `replace` hands the old entry back so
            // the notice can name both paths without cloning one first.
            if let Some(slot) = out.iter_mut().find(|s| s.name == skill.name) {
                let shadowed = std::mem::replace(slot, skill);
                notices.push(format!(
                    "skills: '{}' at {} shadows the same name at {}; the later root wins",
                    slot.name,
                    slot.dir.display(),
                    shadowed.dir.display()
                ));
            } else {
                out.push(skill);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Discovered {
        skills: out,
        notices,
    }
}

/// Whether a root that failed to open is the ordinary "there are none here" case.
///
/// Absent is not broken, which is why a notice per missing directory would be noise. A
/// **link that reaches nothing** is the opposite: the skills are on disk and cannot be
/// read, and `read_dir` cannot tell it from a root that was never there. The link check is
/// the only thing standing between a dangling `.epsilon/skills` and silence.
///
/// [`crate::paths::is_link`] is the shared predicate rather than `FileType::is_symlink()`,
/// because on Windows the answer that matters is the junction — the reparse point that
/// needs no privilege and that `is_symlink()` does not report.
fn root_is_absent(root: &Path, err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::NotFound && !crate::paths::is_link(root)
}

/// Read a frontmatter boolean the way YAML spells them.
///
/// # Why this exists, and why it fails **closed**
///
/// This used to be `get("disable-model-invocation").map(|v| v == "true").unwrap_or(false)` —
/// which meant a skill whose frontmatter said `disable-model-invocation: yes`, or `True`, or
/// `on`, was parsed as **model-invocable**. A user's own opt-out silently did nothing, on a
/// field whose entire purpose is to withhold a skill from the model.
///
/// That is the shape `AGENTS.md` §6 forbids: an unrecognised value on a capability must be
/// `Ask`, never `Allow`. So an unrecognised spelling now means **the restriction applies**.
///
/// **Why not DSH's stronger answer.** DeepSeek Harness accepts the same YAML boolean set and, on
/// a rejected spelling, **drops the whole skill with a warning**. That is the better rule, and
/// [`discover_with_notices`] is now the channel that could carry the warning — but dropping the
/// skill is a different behaviour from loading it with the restriction applied, and changing what
/// loads is not this function's business. What is left is the safe direction: the worst outcome
/// is a skill a person can still invoke by name, rather than a skill the author asked to withhold
/// from the model and now cannot use at all.
///
/// Absent means `false`: the key was not written, so there is no restriction to honour.
fn invocation_flag(value: Option<String>) -> bool {
    let Some(raw) = value else { return false };
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => true,
        "false" | "no" | "off" | "0" => false,
        // Unrecognised: honour the restriction rather than ignore it.
        _ => true,
    }
}

/// Catalog text for the prompt CONTEXT tier: one line per skill the model may
/// load (progressive disclosure — bodies cost nothing until invoked).
///
/// **Only model-loadable skills appear.** A skill marked
/// `disable-model-invocation` is a procedure the user drives deliberately.
/// Advertising it would leave the model holding a name it is not allowed to use,
/// which is worse than not knowing it: an instruction it cannot follow invites a
/// guess, and then an apology. The `skill` tool filters identically, so the
/// catalog and the tool agree about what exists.
pub fn catalog(skills: &[Skill]) -> String {
    let loadable = skills
        .iter()
        .filter(|skill| !skill.disable_model_invocation)
        .collect::<Vec<_>>();
    if loadable.is_empty() {
        return String::new();
    }
    let mut s = String::from("# Skills\nLoad a skill's full instructions before following them.\n");
    for sk in loadable {
        s.push_str(&format!("- {}: {}\n", sk.name, sk.description));
    }
    s
}

fn valid_name(name: &str) -> bool {
    name.len() <= MAX_NAME
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write_skill(root: &Path, dir: &str, fm: &str, body: &str) {
        let d = root.join(dir);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {dir}\ndescription: {fm}\n---\n{body}"),
        )
        .unwrap();
    }

    fn skill(name: &str, disable_model_invocation: bool) -> Skill {
        Skill {
            name: name.into(),
            description: format!("{name} does a thing"),
            dir: std::path::PathBuf::from(name),
            disable_model_invocation,
        }
    }

    #[test]
    fn the_catalog_lists_only_what_the_model_may_load() {
        // Advertising a skill the model cannot open is worse than hiding it: the
        // instruction cannot be followed, so the model guesses and then apologises.
        let skills = vec![skill("open", false), skill("user-only", true)];
        let text = catalog(&skills);
        assert!(text.contains("open"), "{text}");
        assert!(
            !text.contains("user-only"),
            "a skill the model may not load must not be advertised: {text}"
        );
    }

    #[test]
    fn a_catalog_of_nothing_loadable_is_empty_rather_than_a_bare_heading() {
        let text = catalog(&[skill("user-only", true)]);
        assert!(
            text.is_empty(),
            "an empty Skills section is noise: {text:?}"
        );
        assert!(catalog(&[]).is_empty());
    }

    #[test]
    fn discovers_validates_and_shadows_by_priority_root_order() {
        let global = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();

        write_skill(global.path(), "commit-craft", "Craft commits.", "body-v1");
        write_skill(
            project.path(),
            "commit-craft",
            "Project override.",
            "body-v2",
        );
        write_skill(global.path(), "Bad_Name", "invalid name chars", "x");
        // missing description -> invalid
        let d = global.path().join("no-desc");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), "---\nname: no-desc\n---\nbody").unwrap();

        let skills = discover(&[global.path().to_path_buf(), project.path().to_path_buf()]);
        assert_eq!(
            skills.len(),
            1,
            "invalid skills skipped; same-name shadowed"
        );
        assert_eq!(skills[0].name, "commit-craft");
        assert_eq!(skills[0].description, "Project override.");
        assert_eq!(skills[0].load_body().unwrap().trim(), "body-v2");
    }

    #[test]
    fn catalog_lists_name_and_description_only() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "test-first", "Reproduce then fix.", "long body");
        let cat = catalog(&discover(&[dir.path().to_path_buf()]));
        assert!(cat.contains("- test-first: Reproduce then fix."));
        assert!(
            !cat.contains("long body"),
            "progressive disclosure: no bodies in catalog"
        );
    }

    #[test]
    fn frontmatter_edge_cases() {
        assert_eq!(
            strip_frontmatter("no frontmatter here"),
            "no frontmatter here"
        );
        let (fm, body) = parse_frontmatter("---\nname: x\n---\nBODY");
        assert_eq!(fm.len(), 1);
        assert_eq!(body.trim(), "BODY");
    }

    /// Write a `SKILL.md` with arbitrary frontmatter lines, so a test can write a spelling a
    /// typed helper would never produce.
    fn write_raw(root: &Path, dir: &str, extra: &str) {
        let d = root.join(dir);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("SKILL.md"),
            format!(
                "---\nname: {dir}\ndescription: a thing\n{extra}---\nbody\n",
                extra = extra
            ),
        )
        .unwrap();
    }

    fn discovered_flag(root: &Path, dir: &str, extra: &str) -> Option<bool> {
        write_raw(root, dir, extra);
        discover(&[root.to_path_buf()])
            .into_iter()
            .find(|s| s.dir.file_name().is_some_and(|n| n == dir))
            .map(|s| s.disable_model_invocation)
    }

    /// Every YAML spelling of "true" must be honoured, not just the lowercase one.
    ///
    /// This is the regression. The field used to be read as `v == "true"`, so
    /// `disable-model-invocation: yes` — valid YAML, and how most people write it — parsed as
    /// **false**, and a skill the owner had explicitly withheld from the model became
    /// model-invocable.
    #[test]
    fn every_yaml_spelling_of_true_disables_model_invocation() {
        for spelling in [
            "disable-model-invocation: true",
            "disable-model-invocation: True",
            "disable-model-invocation: TRUE",
            "disable-model-invocation: yes",
            "disable-model-invocation: on",
            "disable-model-invocation: 1",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let flag = discovered_flag(dir.path(), "held-back", &format!("{spelling}\n"));
            assert_eq!(
                flag,
                Some(true),
                "`{spelling}` is a true value in YAML and must disable model invocation"
            );
        }
    }

    #[test]
    fn an_explicit_false_still_leaves_the_skill_model_invocable() {
        for spelling in [
            "disable-model-invocation: false",
            "disable-model-invocation: no",
            "disable-model-invocation: off",
            "disable-model-invocation: 0",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let flag = discovered_flag(dir.path(), "open-to-model", &format!("{spelling}\n"));
            assert_eq!(
                flag,
                Some(false),
                "`{spelling}` is an explicit false and must not withhold the skill"
            );
        }
    }

    /// An unrecognised spelling fails **closed**.
    ///
    /// The key is present, so someone wrote something intended to restrict the skill. We cannot
    /// know what, and `AGENTS.md` §6 is explicit that an unrecognised value on a capability is
    /// `Ask`, never `Allow` — so the restriction applies. The old behaviour did the opposite.
    #[test]
    fn an_unrecognised_spelling_fails_closed_rather_than_open() {
        for spelling in [
            "disable-model-invocation: perhaps",
            "disable-model-invocation:",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let flag = discovered_flag(dir.path(), "mystery", &format!("{spelling}\n"));
            assert_eq!(
                flag,
                Some(true),
                "`{spelling}` is not a boolean we recognise, so the restriction must apply"
            );
        }
    }

    /// No key at all is not a restriction, and must not invent one.
    #[test]
    fn an_absent_key_leaves_the_skill_model_invocable() {
        let dir = tempfile::tempdir().unwrap();
        let flag = discovered_flag(dir.path(), "ordinary", "");
        assert_eq!(flag, Some(false));
    }

    /// **The defect this whole channel exists for: an unreadable root read as an empty one.**
    ///
    /// `discover` used to `let Ok(entries) = read_dir(root) else { continue }`, so the user was
    /// told they had no skills and the reason they had none was never shown — the exact shape
    /// `AGENTS.md` §2.7 calls worse than a missing feature.
    ///
    /// A root that is a **file** is the portable way to make `read_dir` fail for a reason other
    /// than absence: a permission error needs an ACL and a dangling link needs a privilege, and
    /// a test that needs either of those is a test that quietly asserts nothing.
    #[test]
    fn a_root_that_cannot_be_read_is_named_with_its_reason_instead_of_reading_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_root = dir.path().join("skills");
        std::fs::write(&not_a_root, "a file where a skills root should be\n").unwrap();

        let found = discover_with_notices(std::slice::from_ref(&not_a_root));
        assert!(
            found.skills.is_empty(),
            "there are genuinely no skills here; what is wrong is that nothing said so: {found:?}"
        );
        assert_eq!(found.notices.len(), 1, "one thing went wrong: {found:?}");

        let named = not_a_root.display().to_string();
        let notice = &found.notices[0];
        let prefix = format!("skills: cannot read root {named}: ");
        let reason = notice.strip_prefix(prefix.as_str()).unwrap_or_else(|| {
            panic!("the notice must name the root AND say what went wrong; got {notice:?}")
        });
        assert!(
            !reason.trim().is_empty(),
            "a notice that names a root and no reason is the defect again: {notice:?}"
        );
    }

    /// **The negative control for the test above.**
    ///
    /// A root that does not exist is the ordinary case — most workspaces have no skills
    /// directory at all — so it must stay silent. If it announced itself, the unreadable-root
    /// test would pass for the wrong reason, and a channel that speaks on every launch is a
    /// channel people learn to stop reading.
    #[test]
    fn a_root_that_is_simply_absent_is_not_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let found = discover_with_notices(&[dir.path().join("no-such-skills")]);
        assert!(found.skills.is_empty());
        assert!(
            found.notices.is_empty(),
            "discovery genuinely completed and found nothing; that is not a problem to report: \
             {found:?}"
        );
    }

    /// **The other defect: a shadowed global skill used to pass without a word.**
    ///
    /// Last-root-wins is a recorded divergence from pi and is unchanged; what changes is that it
    /// is now announced with both paths, because a user who shadows a global skill and is told
    /// nothing cannot tell that they shadowed anything.
    #[test]
    fn a_collision_names_both_paths_and_says_which_one_won() {
        let global = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_skill(global.path(), "commit-craft", "Craft commits.", "body-v1");
        write_skill(
            project.path(),
            "commit-craft",
            "Project override.",
            "body-v2",
        );

        let found =
            discover_with_notices(&[global.path().to_path_buf(), project.path().to_path_buf()]);
        assert_eq!(found.skills.len(), 1, "one name, one winner: {found:?}");
        assert_eq!(
            found.skills[0].dir,
            project.path().join("commit-craft"),
            "the recorded divergence stands: the later root wins"
        );
        assert_eq!(found.notices.len(), 1, "one collision, one line: {found:?}");

        let notice = &found.notices[0];
        for path in [
            global.path().join("commit-craft"),
            project.path().join("commit-craft"),
        ] {
            assert!(
                notice.contains(&path.display().to_string()),
                "the notice must name {} so a person can open both; got {notice:?}",
                path.display()
            );
        }
        assert!(
            notice.contains("commit-craft") && notice.contains("later root wins"),
            "the notice must name the skill and say which side won; got {notice:?}"
        );
    }

    /// The wrapper keeps the old answer exactly, and a tree of well-formed skills says nothing.
    ///
    /// This is what stops the channel being a tax: the notices are only worth reading if a
    /// healthy launch produces none.
    #[test]
    fn a_clean_tree_is_silent_and_discover_still_returns_the_same_skills() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "beta", "Second.", "b");
        write_skill(dir.path(), "alpha", "First.", "a");
        let roots = [dir.path().to_path_buf()];

        let found = discover_with_notices(&roots);
        assert!(
            found.notices.is_empty(),
            "two well-formed skills and nothing to report: {found:?}"
        );
        assert_eq!(
            found.skills,
            discover(&roots),
            "the wrapper must not change the answer"
        );
        assert_eq!(
            found
                .skills
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"],
            "still sorted by name"
        );
    }

    /// A skill the rules reject is announced; a directory that is not a skill is not.
    ///
    /// Both halves matter. Silence is what makes "my skill does nothing" unfixable, and a
    /// notice for every unrelated directory in the root is what makes people ignore the
    /// channel.
    #[test]
    fn a_rejected_skill_is_announced_and_a_directory_that_is_not_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "Bad_Name", "invalid name chars", "x");
        let no_desc = dir.path().join("no-desc");
        std::fs::create_dir_all(&no_desc).unwrap();
        std::fs::write(no_desc.join("SKILL.md"), "---\nname: no-desc\n---\nbody").unwrap();
        let not_a_skill = dir.path().join("notes");
        std::fs::create_dir_all(&not_a_skill).unwrap();
        std::fs::write(not_a_skill.join("README.md"), "about the repo\n").unwrap();

        let found = discover_with_notices(&[dir.path().to_path_buf()]);
        assert!(found.skills.is_empty(), "all three are rejected: {found:?}");
        assert_eq!(
            found.notices.len(),
            2,
            "the two written skills, and nothing for `notes`, which is a directory: {found:?}"
        );
        assert!(
            found
                .notices
                .iter()
                .any(|n| n.contains("Bad_Name") && n.contains("skill name")),
            "a name the rules reject must be named and explained: {found:?}"
        );
        assert!(
            found
                .notices
                .iter()
                .any(|n| n.contains("no-desc") && n.contains("description")),
            "a skill with no description never reaches the catalog; say so: {found:?}"
        );
    }

    /// A `SKILL.md` that is **there** and cannot be read is a different thing from a directory
    /// that has none.
    ///
    /// Invalid UTF-8 is the portable way to produce that: the file exists, `read_to_string`
    /// refuses it, and before the notices channel the author's skill silently did not load.
    #[test]
    fn a_skill_md_that_is_there_but_unreadable_is_announced() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("SKILL.md"), [0xff, 0xfe, 0x00]).unwrap();

        let found = discover_with_notices(&[dir.path().to_path_buf()]);
        assert!(
            found.skills.is_empty(),
            "nothing loads from an unreadable file: {found:?}"
        );
        assert_eq!(found.notices.len(), 1, "{found:?}");
        assert!(
            found.notices[0].contains("broken"),
            "the notice must name the file it could not read: {found:?}"
        );
    }

    /// **A root that is a link to nothing is not "absent".**
    ///
    /// `read_dir` answers `NotFound` for a dangling link and for a root that was never there,
    /// so without the link check the most confusing case — skills on disk, unreachable — is
    /// the one that stays silent. The link is made while its target exists, because a junction
    /// to a missing directory cannot be created at all on Windows.
    #[test]
    fn a_root_that_is_a_link_to_nothing_is_announced_rather_than_read_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real-skills");
        write_skill(&target, "linked", "Loaded through the link.", "body");
        let link = dir.path().join("linked-skills");
        assert!(
            crate::test_support::dir_link(&target, &link),
            "the platform refused to create a directory link, so this test asserted nothing"
        );

        // The control: a live link finds the skill and reports nothing.
        let live = discover_with_notices(std::slice::from_ref(&link));
        assert_eq!(
            live.skills.len(),
            1,
            "a working link is not a problem: {live:?}"
        );
        assert!(live.notices.is_empty(), "{live:?}");

        std::fs::remove_dir_all(&target).unwrap();
        // Pin the platform fact the branch depends on. If a future platform answers
        // something else here, the test says so instead of passing on another path.
        let err = std::fs::read_dir(&link).expect_err("the link now reaches nothing");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::NotFound,
            "this is the error the link check exists to tell apart from an absent root: {err}"
        );

        let dangling = discover_with_notices(std::slice::from_ref(&link));
        assert!(
            dangling.skills.is_empty(),
            "nothing is reachable: {dangling:?}"
        );
        assert_eq!(
            dangling.notices.len(),
            1,
            "one thing went wrong: {dangling:?}"
        );
        assert!(
            dangling.notices[0].contains(&link.display().to_string()),
            "the notice must name the root: {dangling:?}"
        );
    }
}
