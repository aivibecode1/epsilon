//! The skill tool: load the full instructions of one skill.
//!
//! # Why this tool exists
//!
//! A skill is a directory with a `SKILL.md` in it. The harness discovers those
//! directories and puts a *catalog* in the model's context — name and one-line
//! description only, because pasting every skill body into every prompt would
//! cost more context than the conversation it is meant to save. That design only
//! works if the model can then ask for the body of the one skill it needs.
//!
//! It previously could not. The agent contract instructed the model to "load a
//! skill's body via the `skill` tool", no such tool existed in the registry, and
//! the catalog printed no path — so the model learned a skill existed and had no
//! way to open it. This tool is that missing half.
//!
//! # Design
//!
//! * **The caller owns discovery.** Skills are files on disk, discovered by
//!   whatever trust policy the frontend applies; this tool receives an already
//!   resolved list and never touches the filesystem to find one. That keeps the
//!   trust gate in one place.
//! * **Model-invocation is filtered here, not at the call site.** A skill marked
//!   `disable-model-invocation` is a slash-command-style procedure that the user
//!   drives; the model may not pull it in on its own. Filtering at construction
//!   means a disabled skill cannot be loaded even by a model that guesses its
//!   name.
//! * **A wrong name is answered with the real list.** The failure mode that costs
//!   the most is a model retrying names it has already guessed wrong; naming the
//!   available skills ends that loop in one round trip.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// One skill the model is allowed to load.
///
/// Deliberately not the core `Skill` type: this crate sits below the crate that
/// owns discovery, so the frontend maps its own structure into this one. The
/// direction of the dependency is the reason the two are separate types rather
/// than one shared one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadableSkill {
    /// The name the model must pass back.
    pub name: String,
    /// One-line purpose, shown when the model asks for something unavailable.
    pub description: String,
    /// Directory containing `SKILL.md` — or, for a flat `<name>.md` skill, the
    /// parent skills root itself. Discovery sets this (its `Skill::dir` documents
    /// the two shapes); the loader below reads whichever shape is on disk.
    pub dir: PathBuf,
}

/// Arguments for the skill tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillArgs {
    /// The skill's name, exactly as the catalog lists it.
    pub name: String,
}

/// The tool description. Kept in a file like every other built-in so the wording
/// is reviewable next to the others rather than buried in a `format!`.
pub const SKILL_DESC: &str = include_str!("../descriptions/skill.txt");

/// Max bundled files sampled into a skill response (opencode `skill.ts:42`).
///
/// A capped *list*, never content: the names make bundled scripts and
/// references discoverable without paying their bytes every load.
pub const SKILL_FILE_LIST_LIMIT: usize = 10;

/// Sample up to [`SKILL_FILE_LIST_LIMIT`] non-`SKILL.md` paths under `dir`.
///
/// Reuses the tool [`crate::util::walk_files`] walker so ignore rules, hidden-dir
/// skipping and caps match every other listing — the sample is what the walker
/// would have shown first, minus the skill body itself (opencode
/// `skill.ts:37` finds `!**/SKILL.md`). Paths are forward-slash relative to
/// `dir`, matching the base-dir guidance the envelope carries.
fn bundle_sample(dir: &std::path::Path) -> Vec<String> {
    crate::util::walk_files(dir)
        .into_iter()
        .filter_map(|rel| {
            let s = rel.to_string_lossy().replace('\\', "/");
            if s == "SKILL.md" || s.ends_with("/SKILL.md") {
                None
            } else {
                Some(s)
            }
        })
        .take(SKILL_FILE_LIST_LIMIT)
        .collect()
}

/// Loads one skill body on request.
pub struct SkillTool {
    descriptor: ToolDescriptor,
    /// Skills the model may load, already filtered and sorted by name.
    skills: Arc<[LoadableSkill]>,
}

impl SkillTool {
    /// Build the tool over the skills the model is permitted to load.
    ///
    /// The list is sorted by name so the catalog and the not-found message are
    /// stable across runs — a list whose order changes between calls makes a
    /// model's earlier reasoning about it unreliable.
    pub fn new(skills: Vec<LoadableSkill>) -> Self {
        let mut skills = skills;
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            descriptor: ToolDescriptor {
                name: "skill".into(),
                label: "Skill".into(),
                description: SKILL_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(SkillArgs))
                    .unwrap_or_else(|_| serde_json::json!({"type": "object"})),
                // Loading is a read of one file and touches nothing else, so two
                // calls in one batch cannot interfere.
                execution_mode: ExecutionMode::Parallel,
            },
            skills: skills.into(),
        }
    }

    /// The skills this tool will load, in the order it reports them.
    pub fn names(&self) -> Vec<&str> {
        self.skills.iter().map(|s| s.name.as_str()).collect()
    }

    fn find(&self, name: &str) -> Option<&LoadableSkill> {
        // Case-insensitive, because a model that saw `Ratatui-TUI` in a catalog
        // and typed `ratatui-tui` is not making a mistake worth a round trip.
        self.skills
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The list an unknown name is answered with.
    fn available(&self) -> String {
        self.skills
            .iter()
            .map(|s| format!("- {}: {}", s.name, s.description))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: SkillArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'skill': {e}")))?;
        let wanted = args.name.trim();
        if wanted.is_empty() {
            return Err(ToolFailure::msg(
                "skill requires the skill's name; the catalog in your context lists them",
            ));
        }
        let Some(skill) = self.find(wanted) else {
            // Naming the options is the difference between the model correcting
            // itself and the model guessing again.
            return Err(ToolFailure::msg(format!(
                "no model-loadable skill named '{}'. Available:\n{}",
                wanted,
                self.available()
            )));
        };

        let bundle = skill.dir.join("SKILL.md");
        // `dir` is the bundle directory — or, for a flat `<name>.md` skill, its
        // parent root. The bundle shape is tried first so every skill that loads today
        // loads byte-identical tomorrow; the flat fallback only fires where no bundle
        // exists. The containment check keeps the name inside the root: `join` with
        // an absolute or `..`-carrying name would otherwise escape it, and the name
        // arrives from model input via `find`, not from validated discovery.
        let is_bundle = bundle.is_file();
        let flat = skill.dir.join(format!("{}.md", skill.name));
        let path = if is_bundle {
            bundle
        } else if flat.parent().is_some_and(|parent| parent == skill.dir) && flat.is_file() {
            flat
        } else {
            // Neither shape: report the bundle path, which is the shape discovery
            // promises and the one every skill on disk today uses.
            skill.dir.join("SKILL.md")
        };
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            ToolFailure::msg(format!(
                "cannot read skill '{}' at {}: {e}",
                skill.name,
                path.display()
            ))
        })?;
        // Frontmatter is harness metadata (`name`, `description`, the
        // model-invocation flag). The model already has those from the catalog;
        // repeating them wastes the context the skill's body is meant to use.
        let body = strip_frontmatter(&raw);
        if body.trim().is_empty() {
            return Err(ToolFailure::msg(format!(
                "skill '{}' has an empty body — nothing to load",
                skill.name
            )));
        }

        let chars = body.chars().count();
        // Bundle skills sample their directory (opencode `skill.ts:35-59`):
        // base-dir guidance plus up to 10 non-SKILL.md names in a
        // `<skill_files>` envelope, reusing the tool walker. Flat skills list
        // none — their `dir` is the parent root, and sampling it would leak
        // unrelated files. Content loading above is untouched.
        let (suffix, files, base_dir) = if is_bundle {
            let files = bundle_sample(&skill.dir);
            let base = skill.dir.display().to_string();
            let mut envelope = format!(
                "\n\nBase directory for this skill: {base}\n\
                 Relative paths in this skill (e.g., scripts/, reference/) \
                 are relative to this base directory.\n\
                 Note: file list is sampled.\n\n<skill_files>\n"
            );
            for f in &files {
                envelope.push_str(&format!("<file>{f}</file>\n"));
            }
            envelope.push_str("</skill_files>");
            (envelope, files, Some(base))
        } else {
            (String::new(), Vec::new(), None)
        };
        let mut text = body;
        text.push_str(&suffix);
        let mut out = ToolOutput::text(text);
        out.details = serde_json::json!({
            "skill": skill.name,
            "path": path.display().to_string(),
            "chars": chars,
            "files": files,
            "base_dir": base_dir,
        });
        Ok(out)
    }
}

/// Remove a leading `---` delimited frontmatter block.
///
/// Tolerant on purpose: a file that opens with `---` but never closes it is
/// returned whole rather than truncated to nothing, because a skill body that
/// silently vanishes is worse than one that opens with a stray line.
fn strip_frontmatter(raw: &str) -> String {
    let trimmed = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let Some(rest) = trimmed.strip_prefix("---") else {
        return raw.to_string();
    };
    let Some(end) = rest.find("\n---") else {
        return raw.to_string();
    };
    rest[end + 4..].trim_start_matches(['\r', '\n']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::content::ContentBlock;
    use std::sync::Arc;

    fn write_skill(dir: &std::path::Path, name: &str, body: &str) {
        let dir = dir.join(name);
        std::fs::create_dir_all(&dir).expect("skill dir");
        std::fs::write(dir.join("SKILL.md"), body).expect("skill body");
    }

    fn tool_with(skills: Vec<(&str, &str, PathBuf)>) -> SkillTool {
        SkillTool::new(
            skills
                .into_iter()
                .map(|(name, description, dir)| LoadableSkill {
                    name: name.into(),
                    description: description.into(),
                    dir,
                })
                .collect(),
        )
    }

    async fn run(tool: &SkillTool, name: &str) -> Result<ToolOutput, ToolFailure> {
        let ctx = ToolContext {
            session_id: "t".into(),
            observation_scope: Arc::new(()),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir(),
            token: Default::default(),
            task_host: None,
            goal_host: None,
        };
        tool.execute(
            ToolInvocation {
                tool_call_id: "tc".into(),
                args: serde_json::json!({ "name": name }),
            },
            ctx,
            Arc::new(|_| {}),
        )
        .await
    }

    /// The text the model would actually receive.
    fn text_of(out: &ToolOutput) -> String {
        out.content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn message_of(err: &ToolFailure) -> String {
        err.message.clone()
    }

    #[tokio::test]
    async fn a_known_skill_returns_its_body_without_the_frontmatter() {
        let tmp = tempfile::tempdir().expect("tmp");
        write_skill(
            tmp.path(),
            "alpha",
            "---\nname: alpha\ndescription: does a thing\n---\n\n# Alpha\n\nStep one.\n",
        );
        let tool = tool_with(vec![("alpha", "does a thing", tmp.path().join("alpha"))]);
        let out = run(&tool, "alpha").await.expect("loads");
        let text = text_of(&out);
        assert!(text.contains("Step one."), "{text}");
        assert!(
            !text.contains("description:"),
            "frontmatter is harness metadata, not skill material: {text}"
        );
        assert_eq!(out.details["skill"], "alpha");
    }

    #[tokio::test]
    async fn a_flat_file_skill_loads_from_name_dot_md_beside_the_root() {
        // Discovery reports a flat `<name>.md` skill with `dir` set to its parent root
        // (there is no skill directory), so the tool must read `<dir>/<name>.md` when
        // no `<dir>/SKILL.md` exists — otherwise every flat skill discovers and none loads.
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(
            tmp.path().join("alpha.md"),
            "---\nname: alpha\ndescription: does a thing\n---\n\n# Alpha\n\nStep one.\n",
        )
        .expect("flat skill body");
        let tool = tool_with(vec![("alpha", "does a thing", tmp.path().to_path_buf())]);
        let out = run(&tool, "alpha").await.expect("loads");
        let text = text_of(&out);
        assert!(text.contains("Step one."), "{text}");
        assert!(
            !text.contains("description:"),
            "frontmatter is harness metadata, not skill material: {text}"
        );
        assert_eq!(out.details["skill"], "alpha");
    }

    #[tokio::test]
    async fn an_unknown_name_is_answered_with_the_real_list() {
        let tmp = tempfile::tempdir().expect("tmp");
        write_skill(tmp.path(), "alpha", "body");
        let tool = tool_with(vec![("alpha", "does a thing", tmp.path().join("alpha"))]);
        let err = run(&tool, "beta").await.expect_err("unknown skill");
        let message = message_of(&err);
        assert!(message.contains("beta"), "{message}");
        assert!(
            message.contains("alpha") && message.contains("does a thing"),
            "the failure has to name the options or the model guesses again: {message}"
        );
    }

    #[tokio::test]
    async fn a_name_differing_only_in_case_or_padding_still_loads() {
        let tmp = tempfile::tempdir().expect("tmp");
        write_skill(tmp.path(), "Ratatui-TUI", "body");
        let tool = tool_with(vec![("Ratatui-TUI", "d", tmp.path().join("Ratatui-TUI"))]);
        assert!(run(&tool, "ratatui-tui").await.is_ok());
        assert!(run(&tool, "  RATATUI-TUI  ").await.is_ok());
    }

    #[tokio::test]
    async fn an_empty_name_is_rejected_with_guidance() {
        let tool = tool_with(Vec::new());
        let err = run(&tool, "   ").await.expect_err("empty name");
        assert!(message_of(&err).contains("catalog"), "{}", message_of(&err));
    }

    #[tokio::test]
    async fn an_empty_body_is_a_failure_rather_than_a_silent_blank() {
        let tmp = tempfile::tempdir().expect("tmp");
        write_skill(tmp.path(), "alpha", "---\nname: alpha\n---\n");
        let tool = tool_with(vec![("alpha", "d", tmp.path().join("alpha"))]);
        let err = run(&tool, "alpha").await.expect_err("empty body");
        assert!(
            message_of(&err).contains("empty body"),
            "{}",
            message_of(&err)
        );
    }

    #[tokio::test]
    async fn an_unclosed_frontmatter_block_is_returned_whole() {
        // A body that silently vanished would be the worse failure.
        let tmp = tempfile::tempdir().expect("tmp");
        write_skill(tmp.path(), "alpha", "---\nname: alpha\nno closing fence");
        let tool = tool_with(vec![("alpha", "d", tmp.path().join("alpha"))]);
        let out = run(&tool, "alpha").await.expect("loads");
        assert!(
            text_of(&out).contains("no closing fence"),
            "{}",
            text_of(&out)
        );
    }

    #[test]
    fn names_are_reported_in_a_stable_order() {
        let tmp = tempfile::tempdir().expect("tmp");
        let tool = tool_with(vec![
            ("zeta", "z", tmp.path().join("zeta")),
            ("alpha", "a", tmp.path().join("alpha")),
            ("mu", "m", tmp.path().join("mu")),
        ]);
        assert_eq!(tool.names(), vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn the_description_discourages_speculative_loads() {
        // Each load costs context for the rest of the session, so the tool text
        // has to say so rather than reading as a free lookup.
        let lower = SKILL_DESC.to_lowercase();
        assert!(lower.contains("skill"), "{SKILL_DESC}");
        assert!(
            lower.contains("speculative") || lower.contains("just in case"),
            "the description must discourage speculative loads: {SKILL_DESC}"
        );
    }

    #[tokio::test]
    async fn bundle_skill_lists_sampled_files_and_base_dir() {
        // opencode `skill.ts:35-59`: ripgrep find (limit 10, excluding SKILL.md)
        // + `<skill_files>` envelope + base-dir guidance. A skill with bundled
        // scripts is undiscoverable without it.
        let tmp = tempfile::tempdir().expect("tmp");
        write_skill(
            tmp.path(),
            "alpha",
            "---\nname: alpha\n---\n\n# Alpha\n\nDo it.\n",
        );
        let bundle = tmp.path().join("alpha");
        std::fs::create_dir_all(bundle.join("scripts")).expect("scripts dir");
        std::fs::write(bundle.join("scripts/run.sh"), "#!/bin/sh\necho hi\n").expect("script");
        std::fs::write(bundle.join("reference.md"), "# ref\n").expect("ref");
        let tool = tool_with(vec![("alpha", "d", bundle.clone())]);
        let out = run(&tool, "alpha").await.expect("loads");
        let text = text_of(&out);
        assert!(
            text.contains(&bundle.display().to_string()),
            "base dir must be named: {text}"
        );
        assert!(text.contains("<skill_files>"), "{text}");
        assert!(
            text.contains("scripts/run.sh") || text.contains("scripts\\run.sh"),
            "bundled scripts must be listed: {text}"
        );
        let files_section = text
            .split("<skill_files>")
            .nth(1)
            .expect("envelope")
            .split("</skill_files>")
            .next()
            .unwrap_or("");
        assert!(
            !files_section.contains("SKILL.md"),
            "the list samples non-SKILL.md files only: {files_section}"
        );
    }

    #[tokio::test]
    async fn flat_skill_lists_no_files() {
        // A flat `<name>.md` skill has no bundle dir — `dir` is the parent
        // root, so listing it would leak unrelated files. It must list none.
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(
            tmp.path().join("alpha.md"),
            "---\nname: alpha\n---\n\n# Alpha\n\nDo it.\n",
        )
        .expect("flat skill body");
        std::fs::write(tmp.path().join("unrelated.txt"), "noise").expect("noise");
        let tool = tool_with(vec![("alpha", "d", tmp.path().to_path_buf())]);
        let out = run(&tool, "alpha").await.expect("loads");
        let text = text_of(&out);
        assert!(
            !text.contains("<file>"),
            "a flat skill has no bundle files to sample: {text}"
        );
    }

    #[tokio::test]
    async fn bundle_file_list_is_capped_at_ten() {
        // opencode `skill.ts:42` caps the sample at 10. Eleven-plus files
        // must still yield at most 10 entries.
        let tmp = tempfile::tempdir().expect("tmp");
        write_skill(
            tmp.path(),
            "alpha",
            "---\nname: alpha\n---\n\n# Alpha\n\nDo it.\n",
        );
        let bundle = tmp.path().join("alpha");
        for i in 0..12 {
            std::fs::write(bundle.join(format!("file{i:02}.txt")), "x").expect("f");
        }
        let tool = tool_with(vec![("alpha", "d", bundle)]);
        let out = run(&tool, "alpha").await.expect("loads");
        let text = text_of(&out);
        let count = text.matches("<file>").count();
        assert_eq!(count, super::SKILL_FILE_LIST_LIMIT, "capped at 10: {text}");
    }
}
