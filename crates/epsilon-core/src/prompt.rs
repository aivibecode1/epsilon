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
    /// Delegated-child role brief (M50: agent-def body). Empty for top-level
    /// sessions; rendered FIRST in the tier when set so role survives cache
    /// discipline (prefix-stable: brief changes only when the task type does).
    pub agent_brief: String,
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

/// Fingerprint of the STABLE tier: the bytes a provider's prompt cache is expected to keep.
///
/// Deliberately *not* the whole system prompt. The CONTEXT and VOLATILE tiers change for reasons
/// that have nothing to do with cache health — today's date, the working directory, a reloaded
/// AGENTS.md — and folding them in would report a cache break on every turn that has one. What a
/// provider caches is the prefix up to the first differing byte, and for a session in good health
/// that boundary is the end of this tier.
///
/// A change in this number is a genuine cache event: the tool set or the identity block moved.
/// That is the quantity ADR-008's discipline is supposed to hold constant, now measurable from
/// the loop (`crate::cache_stats`).
pub fn stable_fingerprint(stable: &Stable<'_>) -> u64 {
    fn mix(h: u64, bytes: &[u8]) -> u64 {
        let mut h = h;
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
    const SEED: u64 = 0xcbf2_9ce4_8422_2325;
    let mut h = mix(SEED, AGENT_CONTRACT.trim_end().as_bytes());
    for tool in stable.tools {
        h = mix(h, tool.name.as_bytes());
        h = mix(h, b"\0");
        h = mix(h, tool.snippet.as_bytes());
    }
    h
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
    // Agent brief first (M50): the delegated child's role is the most stable
    // context after identity — cache-prefix discipline wants it before the
    // hash-tagged project instructions.
    if !context.agent_brief.trim().is_empty() {
        p.push_str("\n<agent_role>\n");
        p.push_str(context.agent_brief.trim_end());
        p.push_str("\n</agent_role>\n");
    }
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

/// Instruction files found in the directories the agent has actually touched (M73).
///
/// Only the workspace root's `AGENTS.md`/`CLAUDE.md` used to reach the model, so in a monorepo
/// (`crates/x/AGENTS.md` with the crate's own conventions) the model guessed at conventions that
/// were written down one directory away. Hermes loads the same family from the directories a run
/// works in (`agent/subdirectory_hints.py`); this is that rule with the bounds made explicit.
///
/// The rules, and why each one exists:
///
/// - **Ancestors only, at most [`HINT_MAX_DEPTH`]** — a hint governs the subtree it sits in, and
///   walking further up would pull in unrelated projects on a shared drive.
/// - **The workspace root is excluded** — [`load_project_instructions`] already carries it, and a
///   file must not appear twice in the same prompt.
/// - **Deduplicated by content digest** — a vendored copy of the same `AGENTS.md` in three
///   directories is one block in the prompt, which is both cheaper and less confusing.
/// - **Nearest first, at most [`HINT_MAX_FILES`]** — when a deep tree has more hints than the
///   budget, the closest directory is the one that governs what the model is editing.
/// - **Each file capped at [`HINT_MAX_BYTES`]** — an unbounded include is how a prompt becomes
///   unaffordable; the cut is announced in the block rather than silent.
///
/// The result is deterministic for a given set of touched directories (sorted), because a prompt
/// that reshuffles itself between turns defeats the prefix cache (ADR-008).
pub fn load_subdirectory_hints(
    workspace: &Path,
    touched: &[std::path::PathBuf],
) -> Vec<(String, String)> {
    const CANDIDATES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];
    let mut seen_paths: std::collections::HashSet<std::path::PathBuf> = Default::default();
    let mut seen_digests: std::collections::HashSet<u64> = Default::default();
    let mut candidates: Vec<(usize, std::path::PathBuf)> = Vec::new();

    for dir in touched {
        let mut current = Some(dir.as_path());
        for depth in 0..HINT_MAX_DEPTH {
            let Some(d) = current else { break };
            if !d.starts_with(workspace) || d == workspace {
                break;
            }
            for name in CANDIDATES {
                let path = d.join(name);
                if path.is_file() && seen_paths.insert(path.clone()) {
                    candidates.push((depth, path));
                }
            }
            current = d.parent();
        }
    }

    // Nearest directories first; ties broken by path so the order never depends on walk order.
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut out = Vec::new();
    for (_, path) in candidates {
        if out.len() >= HINT_MAX_FILES {
            break;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if content.trim().is_empty() {
            continue;
        }
        if !seen_digests.insert(fnv1a(content.as_bytes())) {
            continue; // the same file, verbatim, was already included
        }
        let display = path
            .strip_prefix(workspace)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        out.push((display, truncate_hint(content)));
    }
    out
}

/// How far up from a touched directory to look for instruction files.
const HINT_MAX_DEPTH: usize = 5;
/// How many hint files may ride in one prompt.
const HINT_MAX_FILES: usize = 5;
/// Per-file cap for a hint, so a runaway include cannot price the prompt out of use.
const HINT_MAX_BYTES: usize = 4 * 1024;

fn truncate_hint(content: String) -> String {
    if content.len() <= HINT_MAX_BYTES {
        return content;
    }
    let mut cut = HINT_MAX_BYTES;
    while cut > 0 && !content.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n\n[harness] this file is {} bytes; the first {HINT_MAX_BYTES} are shown. Read the \nfull file if the conventions you need are not here.\n",
        &content[..cut],
        content.len()
    )
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

    #[test]
    fn the_stable_fingerprint_moves_only_when_the_stable_tier_does() {
        // The fingerprint exists so a cache miss can be attributed (M69). It has to be stable
        // across everything that is *not* the stable tier, and sensitive to everything that is.
        let base = stable_fingerprint(&Stable { tools: &tools() });
        assert_eq!(
            base,
            stable_fingerprint(&Stable { tools: &tools() }),
            "the same tools fingerprint the same"
        );

        // A different snippet is different bytes — the prompt cache would see them too.
        let mut renamed = tools();
        renamed[0].snippet = "Read files, differently described.".into();
        assert_ne!(base, stable_fingerprint(&Stable { tools: &renamed }));

        // An added tool changes the tier.
        let mut longer = tools();
        longer.push(ToolLine {
            name: "grep".into(),
            snippet: "Search files.".into(),
        });
        assert_ne!(base, stable_fingerprint(&Stable { tools: &longer }));

        // Order is part of the bytes: the registry sorts descriptors precisely so that the same
        // tool set cannot produce two different prefixes (D-05). A different order *should*
        // fingerprint differently, because the provider would see different bytes.
        let mut flipped = tools();
        flipped.reverse();
        assert_ne!(base, stable_fingerprint(&Stable { tools: &flipped }));

        // And nothing about the volatile tiers leaks into it: today's date, the cwd and the
        // model id are not part of this number, which is why a turn that only changes them does
        // not look like a cache break.
        let with_context = build(
            &Stable { tools: &tools() },
            &ContextTier::default(),
            &Volatile {
                cwd: Path::new("/tmp/here"),
                model_id: "whatever",
                today: "2026-01-01".into(),
            },
        );
        let with_other_context = build(
            &Stable { tools: &tools() },
            &ContextTier::default(),
            &Volatile {
                cwd: Path::new("/somewhere/else"),
                model_id: "another-model",
                today: "2030-12-31".into(),
            },
        );
        assert_ne!(
            with_context, with_other_context,
            "the prompt itself does differ (the volatile tier is real)"
        );
        assert_eq!(
            base,
            stable_fingerprint(&Stable { tools: &tools() }),
            "while the stable fingerprint does not"
        );
    }

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
                    agent_brief: String::new(),
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
                agent_brief: String::new(),
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
                agent_brief: String::new(),
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
                agent_brief: String::new(),
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
                agent_brief: String::new(),
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
                agent_brief: String::new(),
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
            agent_brief: String::new(),
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

    // ---- M73: subdirectory hints ---------------------------------------------------------

    fn ws_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, content) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }
        dir
    }

    #[test]
    fn a_touched_subdirectory_contributes_its_instructions() {
        let ws = ws_with(&[
            ("AGENTS.md", "root rules"),
            ("crates/x/AGENTS.md", "crate x rules"),
            ("crates/y/AGENTS.md", "crate y rules"),
        ]);
        let touched = vec![ws.path().join("crates/x/src")];
        let hints = load_subdirectory_hints(ws.path(), &touched);
        assert_eq!(hints.len(), 1, "{hints:#?}");
        assert_eq!(hints[0].0, "crates/x/AGENTS.md");
        assert_eq!(hints[0].1, "crate x rules");
        // An untouched crate's conventions stay out of the prompt.
        assert!(!hints.iter().any(|(p, _)| p.contains("y")));
        // ...and the root file is not repeated: `load_project_instructions` already has it.
        assert!(!hints.iter().any(|(p, _)| p == "AGENTS.md"));
    }

    #[test]
    fn hints_are_read_from_the_touched_directory_and_its_ancestors_only() {
        let ws = ws_with(&[
            ("AGENTS.md", "root"),
            ("crates/AGENTS.md", "all crates"),
            ("crates/x/AGENTS.md", "crate x"),
            ("crates/x/src/deep/AGENTS.md", "deep"),
        ]);
        // Touching the leaf pulls its own file and the ancestors between it and the root.
        let touched = vec![ws.path().join("crates/x/src/deep/module")];
        let names: Vec<String> = load_subdirectory_hints(ws.path(), &touched)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert!(
            names.contains(&"crates/x/src/deep/AGENTS.md".to_string()),
            "{names:?}"
        );
        assert!(
            names.contains(&"crates/x/AGENTS.md".to_string()),
            "{names:?}"
        );
        assert!(names.contains(&"crates/AGENTS.md".to_string()), "{names:?}");
        assert!(!names.contains(&"AGENTS.md".to_string()), "{names:?}");
    }

    #[test]
    fn the_walk_stops_after_a_few_levels() {
        // A hint governs the subtree it sits in. Walking to the filesystem root would pull in
        // whatever happens to be above the workspace on a shared drive.
        let ws = ws_with(&[
            ("AGENTS.md", "root"),
            ("a/AGENTS.md", "too far up"),
            ("a/b/c/d/AGENTS.md", "close enough"),
        ]);
        // Five levels down from the workspace: `a/b/c/d/AGENTS.md` is 4 ancestors away,
        // `a/AGENTS.md` is 7.
        let touched = vec![ws.path().join("a/b/c/d/e/f/g")];
        let names: Vec<String> = load_subdirectory_hints(ws.path(), &touched)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(names, vec!["a/b/c/d/AGENTS.md".to_string()], "{names:?}");
    }

    #[test]
    fn the_same_file_in_two_places_is_one_block() {
        // A vendored copy of the same instructions: one block, not two.
        let ws = ws_with(&[
            ("AGENTS.md", "root"),
            ("a/AGENTS.md", "identical text"),
            ("b/AGENTS.md", "identical text"),
        ]);
        let touched = vec![ws.path().join("a"), ws.path().join("b")];
        let hints = load_subdirectory_hints(ws.path(), &touched);
        assert_eq!(hints.len(), 1, "{hints:#?}");
    }

    #[test]
    fn the_nearest_directories_win_when_there_are_too_many() {
        let mut files: Vec<(String, String)> = vec![("AGENTS.md".into(), "root".into())];
        for i in 0..8 {
            files.push((format!("d{i}/AGENTS.md"), format!("rules {i}")));
        }
        let owned: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let ws = ws_with(&owned);
        let touched: Vec<std::path::PathBuf> =
            (0..8).map(|i| ws.path().join(format!("d{i}"))).collect();
        let hints = load_subdirectory_hints(ws.path(), &touched);
        assert_eq!(
            hints.len(),
            HINT_MAX_FILES,
            "the budget is honoured: {hints:#?}"
        );
        // Every directory here is one level deep, so the tie-break is the path: deterministic.
        let names: Vec<&String> = hints.iter().map(|(p, _)| p).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            names, sorted,
            "order must not depend on the walk: {names:?}"
        );
    }

    #[test]
    fn a_huge_hint_is_cut_and_says_so() {
        let big = "x".repeat(HINT_MAX_BYTES * 2);
        let ws = ws_with(&[("AGENTS.md", "root"), ("big/AGENTS.md", &big)]);
        let hints = load_subdirectory_hints(ws.path(), &[ws.path().join("big")]);
        assert_eq!(hints.len(), 1);
        let (path, content) = &hints[0];
        assert_eq!(path, "big/AGENTS.md");
        assert!(content.len() < big.len(), "it was cut");
        assert!(
            content.contains("the first 4096 are shown"),
            "and the cut is announced, not silent"
        );
        assert!(content.starts_with('x'));
    }

    #[test]
    fn nothing_is_loaded_before_anything_is_touched() {
        let ws = ws_with(&[("AGENTS.md", "root"), ("crates/x/AGENTS.md", "crate x")]);
        assert!(load_subdirectory_hints(ws.path(), &[]).is_empty());
        // A directory outside the workspace contributes nothing, however it was reached.
        let outside =
            load_subdirectory_hints(ws.path(), &[std::path::PathBuf::from("/somewhere/else")]);
        assert!(outside.is_empty());
    }

    #[test]
    fn the_order_does_not_depend_on_the_order_directories_were_touched() {
        let ws = ws_with(&[
            ("AGENTS.md", "root"),
            ("a/AGENTS.md", "rules a"),
            ("b/AGENTS.md", "rules b"),
        ]);
        let forward =
            load_subdirectory_hints(ws.path(), &[ws.path().join("a"), ws.path().join("b")]);
        let backward =
            load_subdirectory_hints(ws.path(), &[ws.path().join("b"), ws.path().join("a")]);
        assert_eq!(
            forward, backward,
            "a prompt that reshuffles defeats the prefix cache"
        );
    }
}
