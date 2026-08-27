//! Three-tier byte-stable system prompt assembly (ADR-008).
//!
//! Tiers, in fixed order:
//! 1. **STABLE** — identity (the embedded agent contract) + tool list + global guidelines.
//!    Byte-identical for a session lifetime; providers cache it.
//! 2. **CONTEXT** — project instructions (AGENTS.md family), versioned by content hash.
//! 3. **VOLATILE** — runtime snapshot (cwd/date/model); changes per turn, placed LAST so
//!    stable prefixes survive.
//!
//! Determinism is enforced by tests: identical inputs produce byte-identical prompts.

use std::path::Path;

/// The product's behavior contract, embedded at compile time (final home since the
/// AGENTS.md split; see docs/qa/05).
pub const AGENT_CONTRACT: &str = include_str!("../assets/agent-contract.md");

/// One tool's prompt-facing summary line.
#[derive(Debug, Clone)]
pub struct ToolLine {
    /// Tool name.
    pub name: String,
    /// One-line description for the available-tools list.
    pub snippet: String,
}

/// Inputs that determine each tier. Anything volatile must live in [`Volatile`].
#[derive(Debug, Clone)]
pub struct Stable<'a> {
    /// Prompt-facing summaries of ACTIVE tools only.
    pub tools: &'a [ToolLine],
}

/// Project-sourced instructions (AGENTS.md family), hash-tagged for observability.
#[derive(Debug, Clone, Default)]
pub struct ContextTier {
    /// Project instruction files as `(source_path, content)` pairs; order = file order given.
    pub instructions: Vec<(String, String)>,
    /// Pre-rendered skill catalog lines (progressive disclosure; see skills.rs).
    pub skills_catalog: Option<String>,
}

/// Per-turn runtime snapshot. Must stay LAST in the assembled prompt.
#[derive(Debug, Clone)]
pub struct Volatile<'a> {
    /// Workspace root display path.
    pub cwd: &'a Path,
    /// Current model id.
    pub model_id: &'a str,
    /// ISO-ish date line (YYYY-MM-DD) so time questions stop hallucinating.
    pub today: String,
}

/// Assemble the full system prompt from the three tiers.
///
/// Sections have fixed names and order; extensions (M8) will register into named slots
/// rather than replacing whole strings (ADR-008's anti-fight rule).
pub fn build(stable: &Stable<'_>, context: &ContextTier, vol: &Volatile<'_>) -> String {
    let mut p = String::with_capacity(8192);

    // ---- Tier 1: STABLE ------------------------------------------------------------
    p.push_str("# Identity\n\n");
    p.push_str(AGENT_CONTRACT.trim_end());
    p.push_str("\n\n# Available tools\n\n");
    if stable.tools.is_empty() {
        p.push_str("(no tools active)\n");
    } else {
        for t in stable.tools {
            p.push_str(&format!("- {}: {}\n", t.name, t.snippet));
        }
    }
    p.push_str(
        "\n# Guidelines\n\
         - Answer questions before acting; act before narrating when asked to implement.\n\
         - Prefer the smallest sufficient tool; batch independent reads.\n\
         - Verify with the project's own checks after code changes; report failures honestly.\n",
    );

    // ---- Tier 2: CONTEXT -------------------------------------------------------------
    if !context.instructions.is_empty() {
        p.push_str("\n<project_context>\nProject-specific instructions:\n\n");
        for (path, content) in &context.instructions {
            // Content hash makes cache invalidation observable and debuggable.
            let hash = fnv1a(content.as_bytes());
            p.push_str(&format!(
                "<project_instructions path=\"{path}\" hash=\"{hash:08x}\">\n{}\n</project_instructions>\n",
                content.trim_end()
            ));
        }
        p.push_str("</project_context>\n");
    }

    // ---- Skills catalog (still CONTEXT tier: stable while skill set unchanged) --------
    if let Some(cat) = &context.skills_catalog {
        p.push('\n');
        p.push_str(cat);
    }

    // ---- Tier 3: VOLATILE --------------------------------------------------------------
    p.push_str("\n<runtime_context>\n");
    p.push_str(&format!("cwd: {}\n", vol.cwd.display()));
    p.push_str(&format!("model: {}\n", vol.model_id));
    p.push_str(&format!("today: {}\n", vol.today));
    p.push_str("</runtime_context>\n");

    p
}

/// Load AGENTS.md-family files + persistent MEMORY.md that exist under `workspace`.
///
/// Hermes-style durable memory: `MEMORY.md` at the workspace root is the *project*
/// memory tier (long-lived project facts), while `~/.config/epsilon/MEMORY.md` (the
/// global tier) is injected first so workspace specifics override it. Both are
/// optional; missing files are silently skipped so a fresh install has zero
/// ceremony. Loaded entries join the CONTEXT tier as hash-tagged
/// `<project_instructions>` blocks — same prefix-cache discipline as AGENTS.md.
pub fn load_project_instructions(workspace: &Path) -> Vec<(String, String)> {
    const AGENTS_CANDIDATES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];
    const MEMORY_NAME: &str = "MEMORY.md";
    let mut out = Vec::new();

    // Global memory comes FIRST so workspace can override it.
    let global_memory = crate::config::global_dir()
        .map(|d| d.join(MEMORY_NAME))
        .filter(|p| p.is_file());
    if let Some(path) = global_memory
        && let Ok(content) = std::fs::read_to_string(&path)
        && !content.trim().is_empty()
    {
        out.push((format!("~/.config/epsilon/{MEMORY_NAME}"), content));
    }

    // Workspace AGENTS.md / CLAUDE.md — first match wins per file-type convention.
    for name in AGENTS_CANDIDATES {
        let path = workspace.join(name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            out.push((name.to_string(), content));
            break;
        }
    }

    // Workspace MEMORY.md is additive (not mutually exclusive with AGENTS.md).
    let ws_memory = workspace.join(MEMORY_NAME);
    if let Ok(content) = std::fs::read_to_string(&ws_memory)
        && !content.trim().is_empty()
    {
        out.push((MEMORY_NAME.to_string(), content));
    }

    out
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> Vec<ToolLine> {
        vec![
            ToolLine {
                name: "read".into(),
                snippet: "Read files.".into(),
            },
            ToolLine {
                name: "bash".into(),
                snippet: "Run commands.".into(),
            },
        ]
    }

    #[test]
    fn byte_stable_for_identical_inputs() {
        let tools = tools();
        let mk = || {
            (
                Stable { tools: &tools },
                ContextTier {
                    instructions: vec![],
                    skills_catalog: None,
                },
                Volatile {
                    cwd: Path::new("/w"),
                    model_id: "faux-1",
                    today: "2026-08-23".into(),
                },
            )
        };
        let (s, c, v) = mk();
        let a = build(&s, &c, &v);
        let (s, c, v) = mk();
        let b = build(&s, &c, &v);
        assert_eq!(a, b);
    }

    #[test]
    fn identity_embeds_agent_contract_and_tiers_are_ordered() {
        let out = build(
            &Stable { tools: &tools() },
            &ContextTier {
                instructions: vec![("AGENTS.md".into(), "be nice".into())],
                skills_catalog: None,
            },
            &Volatile {
                cwd: Path::new("/w"),
                model_id: "m",
                today: "t".into(),
            },
        );
        assert!(
            out.contains("Epsilon Agent Operating Contract"),
            "contract embedded"
        );
        let i_contract = out.find("# Identity").unwrap();
        let i_tools = out.find("# Available tools").unwrap();
        let i_ctx = out.find("<project_context>").unwrap();
        let i_vol = out.find("<runtime_context>").unwrap();
        assert!(i_contract < i_tools && i_tools < i_ctx && i_ctx < i_vol);
    }

    #[test]
    fn context_changes_do_not_disturb_stable_prefix_length() {
        let stable_tier = |out: &str| out.find("<project_context>").unwrap_or(out.len());
        let base = build(
            &Stable { tools: &tools() },
            &ContextTier {
                instructions: vec![],
                skills_catalog: None,
            },
            &Volatile {
                cwd: Path::new("/w"),
                model_id: "m",
                today: "t".into(),
            },
        );
        let with = build(
            &Stable { tools: &tools() },
            &ContextTier {
                instructions: vec![("AGENTS.md".into(), "rules".into())],
                skills_catalog: None,
            },
            &Volatile {
                cwd: Path::new("/w"),
                model_id: "m",
                today: "t".into(),
            },
        );
        // Everything up to the CONTEXT tier marker is byte-identical in both prompts.
        let cut = stable_tier(&with).min(stable_tier(&base));
        assert!(cut > 0);
        assert_eq!(&base[..cut], &with[..cut]);
    }

    #[test]
    fn instruction_hash_detects_content_change() {
        let a = build(
            &Stable { tools: &[] },
            &ContextTier {
                instructions: vec![("A.md".into(), "v1".into())],
                skills_catalog: None,
            },
            &Volatile {
                cwd: Path::new("/"),
                model_id: "",
                today: "".into(),
            },
        );
        let b = build(
            &Stable { tools: &[] },
            &ContextTier {
                instructions: vec![("A.md".into(), "v2".into())],
                skills_catalog: None,
            },
            &Volatile {
                cwd: Path::new("/"),
                model_id: "",
                today: "".into(),
            },
        );
        assert_ne!(a, b, "content change must alter the prompt (and its hash)");
        assert!(b.contains("hash=\""));
    }

    #[test]
    fn project_loader_first_match_wins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "claude rules").unwrap();

        // No AGENTS.md: CLAUDE.md is the fallback match.
        let loaded = load_project_instructions(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, "CLAUDE.md");

        // AGENTS.md appears and wins over the fallback.
        std::fs::write(dir.path().join("AGENTS.md"), "epsilon rules").unwrap();
        let loaded = load_project_instructions(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, "AGENTS.md");
    }

    #[test]
    fn workspace_memory_is_additive_with_agents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "be crisp").unwrap();
        std::fs::write(dir.path().join("MEMORY.md"), "loves short answers").unwrap();
        let loaded = load_project_instructions(dir.path());
        // AGENTS.md first, then project MEMORY.md — both present.
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].0, "AGENTS.md");
        assert_eq!(loaded[1].0, "MEMORY.md");
        let all_names: Vec<_> = loaded.iter().map(|(n, _)| n.as_str()).collect();
        assert!(all_names.contains(&"MEMORY.md"));
        // Byte-stable: MEMORY.md participates in the CONTEXT hash.
        let tier = ContextTier {
            instructions: loaded,
            skills_catalog: None,
        };
        let out = build(
            &Stable { tools: &[] },
            &tier,
            &Volatile {
                cwd: dir.path(),
                model_id: "m",
                today: "t".into(),
            },
        );
        assert!(
            out.contains("loves short answers"),
            "MEMORY.md must reach the prompt"
        );
        assert!(out.contains("hash=\""), "each entry hash-tagged");
    }
}
