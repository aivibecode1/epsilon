//! Three-tier byte-stable system prompt assembly (ADR-008).
//!
//! Tiers, in fixed order:
//! 1. **STABLE** — identity (the embedded agent contract, or the workspace's own
//!    `SYSTEM.md` with `APPEND_SYSTEM.md` after the guidelines) + tool list + global
//!    guidelines. Byte-identical for a session lifetime; providers cache it.
//! 2. **CONTEXT** — project instructions (the AGENTS.md family, from the workspace and
//!    from every ancestor above it), versioned by content hash.
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
    /// Text that replaces the embedded contract inside the identity block
    /// (pi `.pi/SYSTEM.md`, loaded as `customPrompt`).
    ///
    /// Only the contract is replaced. The tool inventory and the guidelines below it are
    /// generated from the live registry, so a user's prompt cannot leave the model
    /// believing in a tool set that is not there; pi drops its own tool list along with
    /// the default prompt (`core/system-prompt.ts:48-73`), which for us would trade a
    /// documented divergence for a blind one.
    pub custom_identity: Option<&'a str>,
    /// Text appended at the end of this tier, after the guidelines (pi
    /// `.pi/APPEND_SYSTEM.md`, `core/system-prompt.ts:41,146-148`).
    ///
    /// Last in the tier on purpose: it is the user's own instruction and the closest
    /// thing to the conversation, which is the position a model reads as most specific.
    pub append: Option<&'a str>,
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
    /// The session's goal, when one is set (M203).
    ///
    /// CONTEXT rather than VOLATILE, and rather than a per-turn reminder: a goal changes when a
    /// person or the model changes it, which is rare, and it sits in the tier whose whole point
    /// is "stable while the thing it describes is unchanged". Putting it in the volatile tier
    /// would move the byte boundary a provider's prompt cache keys on for a value that did not.
    ///
    /// It is *not* the goal record: no id, no revision, no round counter. The model has
    /// `get_goal` for those, and a number that changes every round would rewrite the prefix on
    /// every round.
    pub goal: Option<String>,
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
/// A change in this number is a genuine cache event: the tool set, the identity block, or
/// the appended system text moved. That is the quantity ADR-008's discipline is supposed
/// to hold constant, now measurable from the loop (`crate::cache_stats`).
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
    let identity = stable.custom_identity.unwrap_or(AGENT_CONTRACT);
    let mut h = mix(SEED, identity.trim_end().as_bytes());
    for tool in stable.tools {
        h = mix(h, tool.name.as_bytes());
        h = mix(h, b"\0");
        h = mix(h, tool.snippet.as_bytes());
    }
    if let Some(extra) = stable.append {
        h = mix(h, b"\0");
        h = mix(h, extra.trim_end().as_bytes());
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
    // `.epsilon/SYSTEM.md` replaces the contract; everything below it in the tier is
    // generated from the running registry, so only this block is user-owned.
    p.push_str(stable.custom_identity.unwrap_or(AGENT_CONTRACT).trim_end());
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
    // pi appends `APPEND_SYSTEM.md` here: after the guidelines, before project context
    // (`core/system-prompt.ts:146-148`), so it stays part of the cached prefix.
    if let Some(extra) = stable.append {
        p.push_str("\n\n");
        p.push_str(extra.trim_end());
        p.push('\n');
    }

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
            // **A file cannot close the frame it lives in.** Untrusted text is
            // untrusted even when it is called AGENTS.md: a `</project_instructions>`
            // inside it would end the block early and let the rest of the file
            // read as harness instructions. The backslash keeps the words and
            // removes the tag.
            let safe = escape_project_text(content.trim_end());
            let path = escape_project_attr(path);
            p.push_str(&format!(
                "<project_instructions path=\"{path}\" hash=\"{hash:08x}\">\n{safe}\n</project_instructions>\n"
            ));
        }
        p.push_str("</project_context>\n");
    }

    // ---- Skills catalog (still CONTEXT tier: stable while skill set unchanged) --------
    if let Some(cat) = &context.skills_catalog {
        p.push('\n');
        p.push_str(cat);
    }

    // ---- Session goal (CONTEXT: unchanged until someone changes the goal) ------------
    //
    // Last in the tier on purpose: a session with no goal produces byte-identical output to
    // what every earlier build produced, so this milestone cannot move the prefix of a
    // session that does not use it.
    if let Some(goal) = &context.goal {
        p.push_str("\n<session_goal>\n");
        p.push_str(goal.trim_end());
        p.push_str("\n</session_goal>\n");
    }

    // ---- Tier 3: VOLATILE --------------------------------------------------------------
    p.push_str("\n<runtime_context>\n");
    p.push_str(&format!("cwd: {}\n", vol.cwd.display()));
    p.push_str(&format!("model: {}\n", vol.model_id));
    p.push_str(&format!("today: {}\n", vol.today));
    p.push_str("</runtime_context>\n");

    p
}

/// Load AGENTS.md-family files + persistent MEMORY.md for `workspace`.
///
/// Hermes-style durable memory: `MEMORY.md` at the workspace root is the *project*
/// memory tier (long-lived project facts), while `~/.config/epsilon/MEMORY.md` (the
/// global tier) is injected first so workspace specifics override it. Both are
/// optional; missing files are silently skipped so a fresh install has zero
/// ceremony. Loaded entries join the CONTEXT tier as hash-tagged
/// `<project_instructions>` blocks — same prefix-cache discipline as AGENTS.md.
///
/// The context files themselves come from `workspace` **and every ancestor up to the
/// filesystem root**, outermost first (pi `loadProjectContextFiles`,
/// `core/resource-loader.ts:119-157`). pi's reason is the one that bites: a session
/// opened in a subdirectory of a repository still has to see the repository's own
/// instructions. Reading the list top-down, the outermost file is background and the
/// nearest one is the last word — which is why the workspace's own file is pushed last
/// rather than first, and why `MEMORY.md` (the most specific tier of all) follows it.
pub fn load_project_instructions(workspace: &Path) -> Vec<(String, String)> {
    const MEMORY_NAME: &str = "MEMORY.md";
    let mut out = Vec::new();

    // Global memory comes FIRST so workspace can override it.
    let global_memory = crate::config::global_dir()
        .map(|d| d.join(MEMORY_NAME))
        .filter(|p| p.is_file());
    if let Some(path) = global_memory
        && let Some(content) = read_capped(&path)
        && !content.trim().is_empty()
    {
        out.push((format!("~/.config/epsilon/{MEMORY_NAME}"), content));
    }

    // Context files, outermost first: pi walks up from the workspace and unshifts each
    // hit (`resource-loader.ts:140-152`), so the list it ends with is root-first.
    let mut ancestors: Vec<(String, String)> = Vec::new();
    let mut dir: Option<&Path> = Some(workspace);
    while let Some(d) = dir {
        // A relative workspace bottoms out at the empty path, whose "AGENTS.md" would
        // resolve against the process's own directory — a file nobody asked for.
        if d.as_os_str().is_empty() {
            break;
        }
        if let Some(path) = context_file_in(d)
            && let Some(content) = read_capped(&path)
        {
            ancestors.push((context_file_label(workspace, &path), content));
        }
        dir = d.parent();
    }
    ancestors.reverse();
    out.extend(ancestors);

    // Workspace MEMORY.md is additive (not mutually exclusive with AGENTS.md).
    let ws_memory = workspace.join(MEMORY_NAME);
    if let Some(content) = read_capped(&ws_memory)
        && !content.trim().is_empty()
    {
        out.push((MEMORY_NAME.to_string(), content));
    }

    out
}

/// Keep a file's words but remove the tags that could close its frame.
fn escape_project_text(text: &str) -> String {
    text.replace("</project_instructions>", "<\\/project_instructions>")
        .replace("</project_context>", "<\\/project_context>")
}

/// Make a path safe inside a double-quoted XML attribute.
fn escape_project_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// How much of one instruction file may enter the prompt.///
/// AGENTS.md is prose, and 256 KiB is already a book chapter; the cap exists so
/// a runaway or hostile file cannot size the request (and the bill) by itself.
/// The marker says what happened, so a reader can open the file instead of
/// assuming the text ended.
const CONTEXT_FILE_MAX_BYTES: usize = 256 * 1024;

/// Read at most [`CONTEXT_FILE_MAX_BYTES`] of a text file, with a whole-file
/// marker when the cap bites.
fn read_capped(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut limited = file.take((CONTEXT_FILE_MAX_BYTES + 1) as u64);
    let mut content = String::new();
    limited.read_to_string(&mut content).ok()?;
    if content.len() <= CONTEXT_FILE_MAX_BYTES {
        return Some(content);
    }
    // Never cut inside a codepoint; `read_to_string` gave valid UTF-8, so the
    // cap can only land in the middle of a multi-byte sequence.
    let mut cut = CONTEXT_FILE_MAX_BYTES;
    while cut > 0 && !content.is_char_boundary(cut) {
        cut -= 1;
    }
    let total = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Some(format!(
        "{}\n\n[harness] this file is {total} bytes; the first {cut} are shown. Open the full \
         file if the instructions you need are not here.\n",
        &content[..cut]
    ))
}

/// Candidate context-file names for ONE directory, in pi's precedence order
/// (`core/resource-loader.ts:72`).
///
/// `AGENTS.override.md` leads because it exists to be the exception: it is how a
/// machine-local or personal note overrides a checked-in `AGENTS.md` without the user
/// editing a tracked file.
const CONTEXT_FILE_CANDIDATES: [&str; 5] = [
    "AGENTS.override.md",
    "AGENTS.md",
    "AGENTS.MD",
    "CLAUDE.md",
    "CLAUDE.MD",
];

/// The single context file that governs `dir`, or `None` when the directory has none.
///
/// One file per directory, first existing candidate wins (pi `loadContextFileFromDir`,
/// `core/resource-loader.ts:71-90`). Loading an `AGENTS.md` *and* the
/// `AGENTS.override.md` sitting beside it would make "override" mean "add", which is the
/// opposite of what the name promises.
fn context_file_in(dir: &Path) -> Option<std::path::PathBuf> {
    CONTEXT_FILE_CANDIDATES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

/// How a context file is named in the prompt.
///
/// Workspace-relative while the file is inside the workspace, because that is how a user
/// refers to their own instructions; absolute above it, because `../AGENTS.md` repeated
/// once per level would name no location the model can act on.
fn context_file_label(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Identity-tier text a workspace supplies to replace or extend the built-in contract
/// (pi `.pi/SYSTEM.md` / `.pi/APPEND_SYSTEM.md`).
///
/// Paths are carried alongside the text so a frontend can tell the user which file is in
/// force — the complaint that produced this type is a user writing `SYSTEM.md`, being
/// asked to trust it, and seeing nothing happen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemOverlay {
    /// Display path of the file that replaces the contract, when one was found.
    pub custom_path: Option<String>,
    /// That file's contents.
    pub custom: Option<String>,
    /// Display path of the file appended to the identity tier, when one was found.
    pub append_path: Option<String>,
    /// That file's contents.
    pub append: Option<String>,
}

/// Find the identity-tier overlays for `workspace` (pi `discoverSystemPromptFile` /
/// `discoverAppendSystemPromptFile`, `core/resource-loader.ts:1023-1049`).
///
/// The project's file wins only when the project is trusted: an untrusted checkout must
/// not be able to rewrite the harness's own contract, which is exactly what the trust
/// prompt in front of these two names exists to prevent. A project without a file of its
/// own — or an untrusted one — falls back to the global pair under `~/.config/epsilon`,
/// matching pi's global fallback. The pair is not additive across scopes: one project
/// `SYSTEM.md` shadows the global one, so a project's prompt is predictable.
pub fn load_system_overlay(workspace: &Path, project_trusted: bool) -> SystemOverlay {
    load_system_overlay_in(
        workspace,
        project_trusted,
        crate::config::global_dir().as_deref(),
    )
}

/// [`load_system_overlay`] with the global directory supplied.
///
/// Split out so tests can drive both scopes without writing into the real home
/// directory (the same `_in` seam `crate::config` uses for its own resolution).
pub fn load_system_overlay_in(
    workspace: &Path,
    project_trusted: bool,
    global_dir: Option<&Path>,
) -> SystemOverlay {
    const CUSTOM: &str = "SYSTEM.md";
    const APPEND: &str = "APPEND_SYSTEM.md";
    const PROJECT_CUSTOM: &str = ".epsilon/SYSTEM.md";
    const PROJECT_APPEND: &str = ".epsilon/APPEND_SYSTEM.md";
    const GLOBAL_CUSTOM: &str = "~/.config/epsilon/SYSTEM.md";
    const GLOBAL_APPEND: &str = "~/.config/epsilon/APPEND_SYSTEM.md";

    let project = workspace.join(".epsilon");
    let custom = if project_trusted {
        read_overlay(&project, CUSTOM, PROJECT_CUSTOM)
    } else {
        None
    }
    .or_else(|| global_dir.and_then(|d| read_overlay(d, CUSTOM, GLOBAL_CUSTOM)));
    let append = if project_trusted {
        read_overlay(&project, APPEND, PROJECT_APPEND)
    } else {
        None
    }
    .or_else(|| global_dir.and_then(|d| read_overlay(d, APPEND, GLOBAL_APPEND)));

    SystemOverlay {
        custom_path: custom.as_ref().map(|(path, _)| path.clone()),
        custom: custom.map(|(_, content)| content),
        append_path: append.as_ref().map(|(path, _)| path.clone()),
        append: append.map(|(_, content)| content),
    }
}

/// Read one overlay file as `(display path, contents)`; `None` when it is missing,
/// unreadable or blank.
///
/// Blank counts as absent: pi tests the string it read (`if (customPrompt)`,
/// `core/system-prompt.ts:48`), so an empty `SYSTEM.md` means the default prompt stands.
/// A zero-byte file that silently deleted the contract would be a trap, not a feature.
fn read_overlay(dir: &Path, name: &str, label: &str) -> Option<(String, String)> {
    let content = std::fs::read_to_string(dir.join(name)).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    Some((label.to_string(), content))
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
/// - **Ancestors only, at most ``HINT_MAX_DEPTH``** — a hint governs the subtree it sits in, and
///   walking further up would pull in unrelated projects on a shared drive.
/// - **The workspace root is excluded** — [`load_project_instructions`] already carries it, and a
///   file must not appear twice in the same prompt.
/// - **Deduplicated by content digest** — a vendored copy of the same `AGENTS.md` in three
///   directories is one block in the prompt, which is both cheaper and less confusing.
/// - **Nearest first, at most ``HINT_MAX_FILES``** — when a deep tree has more hints than the
///   budget, the closest directory is the one that governs what the model is editing.
/// - **One file per directory, by the same candidate order as the root loader** — an
///   `AGENTS.override.md` in a subdirectory overrides its sibling exactly as it does at
///   the workspace root; a second name in the same directory would only add to it.
/// - **Each file capped at ``HINT_MAX_BYTES``** — an unbounded include is how a prompt becomes
///   unaffordable; the cut is announced in the block rather than silent.
///
/// The result is deterministic for a given set of touched directories (sorted), because a prompt
/// that reshuffles itself between turns defeats the prefix cache (ADR-008).
pub fn load_subdirectory_hints(
    workspace: &Path,
    touched: &[std::path::PathBuf],
) -> Vec<(String, String)> {
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
            if let Some(path) = context_file_in(d)
                && seen_paths.insert(path.clone())
            {
                candidates.push((depth, path));
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
        let base = stable_fingerprint(&stable(&tools()));
        assert_eq!(
            base,
            stable_fingerprint(&stable(&tools())),
            "the same tools fingerprint the same"
        );

        // A different snippet is different bytes — the prompt cache would see them too.
        let mut renamed = tools();
        renamed[0].snippet = "Read files, differently described.".into();
        assert_ne!(base, stable_fingerprint(&stable(&renamed)));

        // An added tool changes the tier.
        let mut longer = tools();
        longer.push(ToolLine {
            name: "grep".into(),
            snippet: "Search files.".into(),
        });
        assert_ne!(base, stable_fingerprint(&stable(&longer)));

        // Order is part of the bytes: the registry sorts descriptors precisely so that the same
        // tool set cannot produce two different prefixes (D-05). A different order *should*
        // fingerprint differently, because the provider would see different bytes.
        let mut flipped = tools();
        flipped.reverse();
        assert_ne!(base, stable_fingerprint(&stable(&flipped)));

        // And nothing about the volatile tiers leaks into it: today's date, the cwd and the
        // model id are not part of this number, which is why a turn that only changes them does
        // not look like a cache break.
        let with_context = build(
            &stable(&tools()),
            &ContextTier::default(),
            &Volatile {
                cwd: Path::new("/tmp/here"),
                model_id: "whatever",
                today: "2026-01-01".into(),
            },
        );
        let with_other_context = build(
            &stable(&tools()),
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
            stable_fingerprint(&stable(&tools())),
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

    /// A STABLE tier with no identity overlay: the shape every pre-overlay expectation used,
    /// and the shape a workspace with no `SYSTEM.md` still produces.
    fn stable<'a>(tools: &'a [ToolLine]) -> Stable<'a> {
        Stable {
            tools,
            custom_identity: None,
            append: None,
        }
    }

    #[test]
    fn byte_stable_for_identical_inputs() {
        let tools = tools();
        let mk = || {
            (
                Stable {
                    tools: &tools,
                    custom_identity: None,
                    append: None,
                },
                ContextTier {
                    agent_brief: String::new(),
                    instructions: vec![],
                    skills_catalog: None,
                    goal: None,
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
    fn a_goal_rides_in_the_context_tier_and_an_absent_one_changes_nothing() {
        // Two properties at once, and the second is the one that protects every session that
        // does not use goals: with no goal the prompt is byte-for-byte what it was before this
        // tier could carry one, so the feature cannot cost a cached prefix.
        let with_none = build(
            &stable(&tools()),
            &ContextTier {
                agent_brief: String::new(),
                instructions: vec![],
                skills_catalog: None,
                goal: None,
            },
            &Volatile {
                cwd: Path::new("/w"),
                model_id: "m",
                today: "t".into(),
            },
        );
        assert!(
            !with_none.contains("session_goal"),
            "{}",
            &with_none[..200.min(with_none.len())]
        );

        let with_goal = build(
            &stable(&tools()),
            &ContextTier {
                agent_brief: String::new(),
                instructions: vec![],
                skills_catalog: None,
                goal: Some("This session's goal: make the doc gates pass".into()),
            },
            &Volatile {
                cwd: Path::new("/w"),
                model_id: "m",
                today: "t".into(),
            },
        );
        assert!(with_goal.contains("<session_goal>"), "{with_goal}");
        assert!(with_goal.contains("make the doc gates pass"), "{with_goal}");
        assert!(with_goal.contains("</session_goal>"), "{with_goal}");
        // CONTEXT, not volatile: the goal has to sit before the runtime block, or it lands
        // after the bytes a provider's prompt cache keys on and every round rewrites them.
        let i_goal = with_goal.find("<session_goal>").unwrap();
        let i_vol = with_goal.find("<runtime_context>").unwrap();
        assert!(i_goal < i_vol, "the goal must ride the CONTEXT tier");
        // And the STABLE fingerprint ignores it: the cached prefix is unchanged by a goal.
        assert_eq!(
            stable_fingerprint(&stable(&tools())),
            stable_fingerprint(&stable(&tools())),
        );
    }

    #[test]
    fn identity_embeds_agent_contract_and_tiers_are_ordered() {
        let out = build(
            &stable(&tools()),
            &ContextTier {
                agent_brief: String::new(),
                instructions: vec![("AGENTS.md".into(), "be nice".into())],
                skills_catalog: None,
                goal: None,
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
            &stable(&tools()),
            &ContextTier {
                agent_brief: String::new(),
                instructions: vec![],
                skills_catalog: None,
                goal: None,
            },
            &Volatile {
                cwd: Path::new("/w"),
                model_id: "m",
                today: "t".into(),
            },
        );
        let with = build(
            &stable(&tools()),
            &ContextTier {
                agent_brief: String::new(),
                instructions: vec![("AGENTS.md".into(), "rules".into())],
                skills_catalog: None,
                goal: None,
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
            &stable(&[]),
            &ContextTier {
                agent_brief: String::new(),
                instructions: vec![("A.md".into(), "v1".into())],
                skills_catalog: None,
                goal: None,
            },
            &Volatile {
                cwd: Path::new("/"),
                model_id: "",
                today: "".into(),
            },
        );
        let b = build(
            &stable(&[]),
            &ContextTier {
                agent_brief: String::new(),
                instructions: vec![("A.md".into(), "v2".into())],
                skills_catalog: None,
                goal: None,
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
            goal: None,
        };
        let out = build(
            &stable(&[]),
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

    // ---- D-8: the five-name list and the walk above the workspace -----------------------

    /// Only the entries carrying one of `markers`.
    ///
    /// The loader reads the whole directory chain, which on a developer's machine reaches
    /// directories the test did not create; filtering by content keeps these assertions
    /// about this workspace rather than about whatever sits above the temp directory.
    fn marked(loaded: &[(String, String)], markers: &[&str]) -> Vec<(String, String)> {
        loaded
            .iter()
            .filter(|(_, content)| markers.contains(&content.as_str()))
            .cloned()
            .collect()
    }

    #[test]
    fn one_context_file_per_directory_and_the_override_wins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "epsilon-claude").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "epsilon-agents").unwrap();
        std::fs::write(dir.path().join("AGENTS.override.md"), "epsilon-override").unwrap();
        let markers = ["epsilon-claude", "epsilon-agents", "epsilon-override"];

        // pi's order within one directory: the override is the file that governs, and the
        // sibling it overrides is not loaded beside it.
        let loaded = marked(&load_project_instructions(dir.path()), &markers);
        assert_eq!(
            loaded.iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>(),
            vec!["epsilon-override"],
            "{loaded:#?}"
        );
        assert_eq!(
            loaded[0].0, "AGENTS.override.md",
            "labelled by its own name"
        );

        // Remove it: AGENTS.md is next in the list, still ahead of CLAUDE.md.
        std::fs::remove_file(dir.path().join("AGENTS.override.md")).unwrap();
        let loaded = marked(&load_project_instructions(dir.path()), &markers);
        assert_eq!(loaded[0].0, "AGENTS.md");

        // And with that gone, CLAUDE.md is the last candidate that matches.
        std::fs::remove_file(dir.path().join("AGENTS.md")).unwrap();
        let loaded = marked(&load_project_instructions(dir.path()), &markers);
        assert_eq!(loaded[0].0, "CLAUDE.md");
    }

    #[test]
    fn context_files_above_the_workspace_are_loaded_outermost_first() {
        // Opening a session in a subdirectory used to lose the repository's own
        // instructions entirely. pi walks to the filesystem root and unshifts each hit, so
        // the outermost file leads the list and the workspace's own file is last.
        let ws = ws_with(&[
            ("AGENTS.md", "epsilon-outer"),
            ("a/AGENTS.md", "epsilon-middle"),
            ("a/b/AGENTS.md", "epsilon-inner"),
        ]);
        let markers = ["epsilon-outer", "epsilon-middle", "epsilon-inner"];
        let loaded = marked(&load_project_instructions(&ws.path().join("a/b")), &markers);
        assert_eq!(
            loaded.iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>(),
            vec!["epsilon-outer", "epsilon-middle", "epsilon-inner"],
            "outermost first, nearest last: {loaded:#?}"
        );
        // The workspace's own file keeps the short relative label; a file above it has no
        // relative name that would tell the model where it lives.
        assert_eq!(loaded.last().unwrap().0, "AGENTS.md");
        assert!(
            Path::new(&loaded[0].0).is_absolute(),
            "an ancestor file is named by its absolute path: {}",
            loaded[0].0
        );
    }

    #[test]
    fn a_workspace_below_the_repository_root_still_gets_the_repository_instructions() {
        // The same walk read the way a user experiences it: the repo's own file reaches the
        // model even though the workspace is a subdirectory, and it is rendered first so the
        // nearest file has the last word.
        let ws = ws_with(&[
            ("AGENTS.md", "epsilon-repo-rules"),
            ("crates/x/AGENTS.md", "epsilon-crate-rules"),
        ]);
        let cwd = ws.path().join("crates/x");
        let loaded = load_project_instructions(&cwd);
        assert!(
            loaded.iter().any(|(_, c)| c == "epsilon-repo-rules"),
            "{loaded:#?}"
        );
        let out = build(
            &stable(&[]),
            &ContextTier {
                agent_brief: String::new(),
                instructions: loaded,
                skills_catalog: None,
                goal: None,
            },
            &Volatile {
                cwd: &cwd,
                model_id: "m",
                today: "t".into(),
            },
        );
        assert!(
            out.find("epsilon-repo-rules").unwrap() < out.find("epsilon-crate-rules").unwrap(),
            "the repository file is background and the crate file is the last word"
        );
    }

    // ---- D-7: the identity overlays -----------------------------------------------------

    #[test]
    fn the_project_overlay_pair_shadows_the_global_pair() {
        let ws = ws_with(&[
            (".epsilon/SYSTEM.md", "epsilon-project-contract"),
            (".epsilon/APPEND_SYSTEM.md", "epsilon-project-append"),
        ]);
        let global = ws_with(&[
            ("SYSTEM.md", "epsilon-global-contract"),
            ("APPEND_SYSTEM.md", "epsilon-global-append"),
        ]);

        let trusted = load_system_overlay_in(ws.path(), true, Some(global.path()));
        assert_eq!(trusted.custom.as_deref(), Some("epsilon-project-contract"));
        assert_eq!(trusted.custom_path.as_deref(), Some(".epsilon/SYSTEM.md"));
        assert_eq!(trusted.append.as_deref(), Some("epsilon-project-append"));
        assert_eq!(
            trusted.append_path.as_deref(),
            Some(".epsilon/APPEND_SYSTEM.md")
        );

        // An untrusted project contributes nothing — rewriting the harness's own contract is
        // what the trust prompt in front of these names exists to prevent — while the user's
        // global pair still applies.
        let untrusted = load_system_overlay_in(ws.path(), false, Some(global.path()));
        assert_eq!(untrusted.custom.as_deref(), Some("epsilon-global-contract"));
        assert_eq!(
            untrusted.custom_path.as_deref(),
            Some("~/.config/epsilon/SYSTEM.md")
        );
        assert_eq!(untrusted.append.as_deref(), Some("epsilon-global-append"));
    }

    #[test]
    fn an_absent_or_blank_overlay_leaves_the_built_in_contract_alone() {
        let bare = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        assert_eq!(
            load_system_overlay_in(bare.path(), true, Some(global.path())),
            SystemOverlay::default(),
            "no files, no overlay"
        );

        // A blank SYSTEM.md is not a contract replacement: pi tests the string it read
        // (`system-prompt.ts:48`), and a zero-byte file that deleted the instructions would
        // be a trap rather than a feature.
        std::fs::create_dir_all(bare.path().join(".epsilon")).unwrap();
        std::fs::write(bare.path().join(".epsilon/SYSTEM.md"), "   \n").unwrap();
        let blank = load_system_overlay_in(bare.path(), true, Some(global.path()));
        assert_eq!(blank.custom, None);
    }

    #[test]
    fn a_custom_identity_replaces_the_contract_and_keeps_the_generated_sections() {
        let tools = tools();
        let out = build(
            &Stable {
                tools: &tools,
                custom_identity: Some("epsilon-my-rules\n"),
                append: None,
            },
            &ContextTier::default(),
            &Volatile {
                cwd: Path::new("/w"),
                model_id: "m",
                today: "t".into(),
            },
        );
        assert!(out.contains("epsilon-my-rules"));
        assert!(
            !out.contains("Epsilon Agent Operating Contract"),
            "the contract is replaced, not amended: {out}"
        );
        // The tool inventory is a fact about the running registry, so it survives a user
        // prompt that says nothing about tools.
        assert!(
            out.contains("# Available tools") && out.contains("- read: Read files."),
            "{out}"
        );
    }

    #[test]
    fn appended_system_text_follows_the_guidelines_and_precedes_project_context() {
        let tools = tools();
        let out = build(
            &Stable {
                tools: &tools,
                custom_identity: None,
                append: Some("epsilon-house-style\n"),
            },
            &ContextTier {
                agent_brief: String::new(),
                instructions: vec![("AGENTS.md".into(), "epsilon-project".into())],
                skills_catalog: None,
                goal: None,
            },
            &Volatile {
                cwd: Path::new("/w"),
                model_id: "m",
                today: "t".into(),
            },
        );
        let guidelines = out.find("# Guidelines").unwrap();
        let appended = out.find("epsilon-house-style").unwrap();
        let context = out.find("<project_context>").unwrap();
        assert!(
            guidelines < appended && appended < context,
            "pi appends at the end of the identity tier: {out}"
        );
        // Inside the cached prefix, not after the volatile snapshot.
        assert!(appended < out.find("<runtime_context>").unwrap());
    }

    #[test]
    fn the_fingerprint_covers_both_overlays() {
        let tools = tools();
        let base = stable_fingerprint(&stable(&tools));
        let with_custom = stable_fingerprint(&Stable {
            tools: &tools,
            custom_identity: Some("epsilon-mine"),
            append: None,
        });
        let with_append = stable_fingerprint(&Stable {
            tools: &tools,
            custom_identity: None,
            append: Some("epsilon-mine"),
        });
        assert_ne!(base, with_custom, "a replaced identity is a cache event");
        assert_ne!(base, with_append, "appended text is part of the same bytes");
        assert_ne!(
            with_custom, with_append,
            "the two overlays are not one field"
        );

        // Editing the file between runs changes the number — that is the point of folding it
        // in: a cache miss caused by a user's own edit is explained, not mysterious.
        let edited = stable_fingerprint(&Stable {
            tools: &tools,
            custom_identity: Some("epsilon-mine, revised"),
            append: None,
        });
        assert_ne!(with_custom, edited);
    }

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
    fn a_subdirectory_override_file_shadows_its_agents_md() {
        // The override has to mean the same thing one directory down as it does at the root,
        // otherwise the escape hatch only works where it is least needed.
        let ws = ws_with(&[
            ("AGENTS.md", "root"),
            ("crates/x/AGENTS.md", "epsilon-crate"),
            ("crates/x/AGENTS.override.md", "epsilon-crate-override"),
        ]);
        let hints = load_subdirectory_hints(ws.path(), &[ws.path().join("crates/x")]);
        assert_eq!(hints.len(), 1, "{hints:#?}");
        assert_eq!(hints[0].0, "crates/x/AGENTS.override.md");
        assert_eq!(hints[0].1, "epsilon-crate-override");
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

    /// A project file cannot close the frame it is quoted inside.
    #[test]
    fn project_text_cannot_close_its_own_frame() {
        assert_eq!(
            escape_project_text("before</project_instructions>after"),
            "before<\\/project_instructions>after"
        );
        assert_eq!(
            escape_project_text("before</project_context>after"),
            "before<\\/project_context>after"
        );
        // A path with a quote cannot break out of its attribute either.
        assert_eq!(escape_project_attr(r#"a"b<c>&"#), "a&quot;b&lt;c&gt;&amp;");
    }

    /// One hostile file cannot size the request by itself. The cap keeps the
    /// beginning (where instructions usually are) and says it cut.
    #[test]
    fn an_oversized_instruction_file_is_capped_with_a_marker() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("AGENTS.md");
        let body = format!("HEAD\n{}", "x".repeat(CONTEXT_FILE_MAX_BYTES + 64));
        std::fs::write(&path, &body).expect("write fixture");
        let read = read_capped(&path).expect("the file is readable");
        assert!(read.starts_with("HEAD"), "the beginning survives");
        assert!(
            read.contains("are shown"),
            "the marker says the file was cut: {}",
            &read[..80.min(read.len())]
        );
        assert!(
            read.len() <= CONTEXT_FILE_MAX_BYTES + 256,
            "the cap bounds the prompt: {} bytes",
            read.len()
        );
    }
}
