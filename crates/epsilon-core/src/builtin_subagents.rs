//! Subagent roles that ship in the binary and are installed on request.
//!
//! A **subagent role** here means an agent definition for the `task` tool: a role with
//! a system prompt and a tool subset. The definitions use the same
//! `agents::AgentDef` format a user writes by hand, so enabling one writes an
//! ordinary `<config>/builtin-agents/<name>.md` and discovery does the rest.
//!
//! Why not auto-load them: a subagent changes what a delegated child may do. A
//! user who has not asked for a reviewer, a verifier or an implementer should not
//! find those roles offered to the model. `/subagent <name> on` is the opt-in, and
//! `/subagent` lists what is available before anything is installed.

use std::path::{Path, PathBuf};

/// One built-in subagent role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinSubagent {
    /// The agent name shown to the `task` tool and used as the file stem.
    pub name: &'static str,
    /// One-line purpose.
    pub description: &'static str,
    /// Why the subagent is useful, shown by `/subagent` before it is enabled.
    pub why: &'static str,
    /// Tool subset. `None` means every tool the parent has.
    pub tools: Option<&'static [&'static str]>,
    /// The system prompt, without frontmatter.
    pub body: &'static str,
}

/// Built-in subagents, in the order `/subagent` presents them.
pub const BUILTIN_SUBAGENTS: &[BuiltinSubagent] = &[
    BuiltinSubagent {
        name: "explore",
        description: "Read-only reconnaissance: map a repository, trace a path, return evidence.",
        why: "Gives a parent a bounded-looking child that cannot edit the tree.",
        tools: Some(&["read", "grep", "find", "glob", "ls", "lsp", "todo"]),
        body: include_str!("builtin_subagents/explore.md"),
    },
    BuiltinSubagent {
        name: "review",
        description: "Review a diff for spec compliance, then quality; read-only, reports file:line.",
        why: "Makes the independent second reader a first-class role instead of a prompt habit.",
        // No `bash`: a role described as read-only must not carry a tool that
        // can write. The parent or the `verify` subagent runs the tests.
        tools: Some(&["read", "grep", "find", "glob", "ls", "lsp"]),
        body: include_str!("builtin_subagents/review.md"),
    },
    BuiltinSubagent {
        name: "verify",
        description: "Run the project's tests and checks, report the exact commands and results.",
        why: "Separates proving a change from making it, which is where green claims go wrong.",
        tools: Some(&["read", "grep", "find", "glob", "ls", "bash"]),
        body: include_str!("builtin_subagents/verify.md"),
    },
    BuiltinSubagent {
        name: "implement",
        description: "Implement one scoped change with tests, then report what was and was not verified.",
        why: "A worker role for delegated edits, kept inside the parent's permission policy.",
        tools: None,
        body: include_str!("builtin_subagents/implement.md"),
    },
];

/// All built-in subagents.
pub fn all() -> &'static [BuiltinSubagent] {
    BUILTIN_SUBAGENTS
}

/// Find a subagent by name, case-insensitively.
pub fn find(name: &str) -> Option<&'static BuiltinSubagent> {
    let wanted = name.trim();
    BUILTIN_SUBAGENTS
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(wanted))
}

/// The directory built-in subagents are installed under for a config directory.
pub fn root_in(config_dir: &Path) -> PathBuf {
    config_dir.join("builtin-agents")
}

/// Where one installed subagent lives.
pub fn file_in(config_dir: &Path, name: &str) -> PathBuf {
    root_in(config_dir).join(format!("{name}.md"))
}

/// Whether a subagent is installed in this config directory.
pub fn installed(name: &str, config_dir: &Path) -> bool {
    find(name).is_some_and(|h| file_in(config_dir, h.name).is_file())
}

/// Render the agent file: frontmatter plus the body.
pub fn rendered(subagent: &BuiltinSubagent) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("name: {}\n", subagent.name));
    out.push_str(&format!("description: \"{}\"\n", subagent.description));
    if let Some(tools) = subagent.tools {
        out.push_str("tools: ");
        out.push_str(&tools.join(", "));
        out.push('\n');
    }
    out.push_str("---\n\n");
    out.push_str(subagent.body.trim_end());
    out.push('\n');
    out
}

/// Install a subagent, returning the file that now exists.
///
/// Identical reinstall is a success. A differing file is refused rather than
/// overwritten: it may be the user's edit.
pub fn install(name: &str, config_dir: &Path) -> Result<PathBuf, String> {
    let subagent =
        find(name).ok_or_else(|| format!("no built-in subagent named '{name}' is bundled"))?;
    let path = file_in(config_dir, subagent.name);
    let body = rendered(subagent);
    if path.is_file() {
        let existing =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if existing == body {
            return Ok(path);
        }
        return Err(format!(
            "{} already exists and differs from the bundled copy; \
             edit or remove it yourself, then enable it again",
            path.display()
        ));
    }
    let dir = path
        .parent()
        .ok_or_else(|| "install path has no parent".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    // Stage then rename, so an interrupted write cannot leave a half agent
    // definition that discovery would report as unreadable on every launch.
    let staging = path.with_extension(format!("md.tmp.{}", std::process::id()));
    std::fs::write(&staging, body).map_err(|e| format!("write {}: {e}", staging.display()))?;
    if let Err(e) = std::fs::rename(&staging, &path) {
        let _ = std::fs::remove_file(&staging);
        return Err(format!("replace {}: {e}", path.display()));
    }
    Ok(path)
}

/// Remove a pristine installed subagent. Returns `true` when a file was removed.
pub fn remove(name: &str, config_dir: &Path) -> Result<bool, String> {
    let subagent =
        find(name).ok_or_else(|| format!("no built-in subagent named '{name}' is bundled"))?;
    let path = file_in(config_dir, subagent.name);
    if !path.exists() {
        return Ok(false);
    }
    let existing =
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if existing != rendered(subagent) {
        return Err(format!(
            "{} has been edited, so it was left in place; remove it yourself to disable it",
            path.display()
        ));
    }
    std::fs::remove_file(&path).map_err(|e| format!("remove {}: {e}", path.display()))?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents;

    fn temp_config() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp config dir")
    }

    #[test]
    fn every_subagent_renders_a_definition_discovery_accepts() {
        for subagent in all() {
            let root = tempfile::tempdir().expect("root");
            install(subagent.name, root.path()).expect("install");
            let found = agents::discover_agents(&[root_in(root.path())], &[], true);
            let def = found
                .iter()
                .find(|d| d.name == subagent.name)
                .unwrap_or_else(|| panic!("{} did not parse: {found:?}", subagent.name));
            assert_eq!(def.description, subagent.description);
            assert_eq!(
                def.tools.as_deref(),
                subagent
                    .tools
                    .map(|t| t.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                    .as_deref(),
                "{} tools round-trip",
                subagent.name
            );
        }
    }

    #[test]
    fn nothing_is_installed_until_the_user_asks() {
        let tmp = temp_config();
        assert!(!root_in(tmp.path()).exists());
        assert!(!installed("review", tmp.path()));
    }

    #[test]
    fn install_is_idempotent_and_remove_cleans_up() {
        let tmp = temp_config();
        let first = install("verify", tmp.path()).expect("install");
        let second = install("verify", tmp.path()).expect("install again");
        assert_eq!(first, second);
        assert!(installed("verify", tmp.path()));
        assert!(remove("verify", tmp.path()).expect("remove"));
        assert!(!installed("verify", tmp.path()));
        assert!(!root_in(tmp.path()).exists());
    }

    #[test]
    fn an_edited_subagent_is_never_overwritten_or_removed() {
        let tmp = temp_config();
        let path = install("explore", tmp.path()).expect("install");
        std::fs::write(&path, "my own agent").expect("edit");
        let refused = install("explore", tmp.path()).expect_err("must refuse overwrite");
        assert!(refused.contains("differs"), "{refused}");
        let kept = remove("explore", tmp.path()).expect_err("must refuse delete");
        assert!(kept.contains("edited"), "{kept}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "my own agent");
    }
}
