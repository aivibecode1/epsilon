//! Agent Skills spec support (ADR-015): SKILL.md directories with YAML-lite frontmatter,
//! progressive disclosure — catalog (name + description) in the prompt, body on demand.

use std::path::{Path, PathBuf};

/// Name rules per the spec (pi parity): <=64 chars, lowercase a-z/0-9/hyphen,
/// no leading/trailing/consecutive hyphens.
const MAX_NAME: usize = 64;
/// Description limit per the spec.
const MAX_DESC: usize = 1024;

// ---------------------------------------------------------------------------------------------
// Where skills are looked for
//
// The `SKILL.md` directory is an open convention, not an Epsilon one: Claude Code shipped it,
// and Codex, Gemini CLI, Cursor, Windsurf, opencode and DeepSeek Harness all read the same
// shape. A person who has been using any of those already has skills on disk, and a harness
// that reads only its own directory requires them to copy each skill into a second tree and
// then keep the two in sync by hand.
//
// So discovery scans every root below, and a skill's worth does not depend on which tool
// created its directory. Order matters — [`discover_with_notices`] lets the **later** root win
// a name collision — and the order encodes one rule: **workspace beats user, and a root
// written for this harness beats a compatibility root**. Within DSH's own tier `.dsh` beats
// `.agents`, and extras ([`all_roots_with_extra`]) win everything because the user named them.
//
// **What is deliberately NOT scanned, and why.** Each of these is a real feature of a tool
// named above, and each is out of scope here rather than overlooked:
//
// * **nested project directories** — Claude Code reads `<subdir>/.claude/skills` for a
//   monorepo, and Pi/opencode recurse below a root. Doing that means walking the workspace
//   tree, which is a different cost and a different semantic from reading a fixed list of
//   roots. Epsilon reads one level; that is stated as a divergence, not as parity.
// * **extra directories passed on the command line** — Claude Code's `--add-dir`, and
//   Codex's `/etc/codex/skills` admin tier. Epsilon has no flag that adds a root today, so
//   there is no surface to hang this on; when one exists, [`all_roots`] is where it goes.
//   Whether that surface is a CLI flag or a config key is a product decision not made
//   here, so nothing is built for it. (Deferred, not overlooked.)
// * **descending below the roots** — discovery reads one level: the entries of each
//   root, never their subdirectories. DSH deliberately reads the same depth, so a skill
//   tree resolves identically in both harnesses. (Deferred, not overlooked.)
// * **plugin skills** — `<plugin>/skills/<name>/SKILL.md`, namespaced by plugin. A plugin
//   root is installed software, not a directory the user typed into, and would need its own
//   trust story rather than borrowing the workspace's.
// * **rules files that are not skills** — `.clinerules/`, `.cursor/rules/`, `.roo/rules/`,
//   `.continue/rules/`, `CONVENTIONS.md`. These are always-on instructions in a different
//   format, closer to what Epsilon already reads from `AGENTS.md` / `MEMORY.md`; treating
//   them as skills would load a rules file into a tool that expects a SKILL.md.
// ---------------------------------------------------------------------------------------------

/// User-tier (global) skill roots, in ascending precedence.
///
/// These load whether or not the project is trusted: they are the user's own files, in the
/// user's own home directory, and a repository has no say in them.
///
/// The home directory is read through [`crate::config::home_dir`] so that `$HOME` stays
/// resolved in exactly one place (M223). When no home directory resolves, this list is just
/// Epsilon's own root — which is empty too, because that root lives under the home directory.
pub fn user_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    // Built-ins first, so every directory below outranks them on a name clash.
    // They are ordinary skills once enabled (`/skill <name> on` writes one), but
    // a user's own copy of the same name is the more specific instruction.
    if let Some(dir) = crate::config::global_dir() {
        roots.push(crate::builtin_skills::root_in(&dir));
    }
    if let Some(home) = crate::config::home_dir() {
        // Other harnesses' conventions next, so none of them can outrank a
        // directory the user filled *for Epsilon*. Later roots win collisions
        // (`discover_with_notices`), and the old order had this backwards: the
        // comment promised Epsilon was not outranked while `.agents` — last —
        // actually beat it on every clash.
        //
        // Claude Code: `~/.claude/skills/<name>/SKILL.md` (code.claude.com/docs/en/skills).
        roots.push(home.join(".claude").join("skills"));
        // OpenAI Codex: `~/.codex/skills` (learn.chatgpt.com/docs/build-skills).
        roots.push(home.join(".codex").join("skills"));
        // Gemini CLI: `~/.gemini/skills` (geminicli.com/docs/cli/skills).
        roots.push(home.join(".gemini").join("skills"));
        // opencode's XDG-style global dir (`~/.config/opencode/skills`).
        roots.push(home.join(".config").join("opencode").join("skills"));
        // DeepSeek Harness, honoring its documented home override. `DSH_HOME`
        // is the home directory itself; skills live directly under it.
        let dsh_home = std::env::var_os("DSH_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".dsh"));
        roots.push(dsh_home.join("skills"));
        // The portable alias, honoring DSH's documented override.
        let agents_home = std::env::var_os("DSH_AGENTS_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".agents"));
        roots.push(agents_home.join("skills"));
        // Pi's global skills live under its *agent* directory, which defaults to
        // `~/.pi/agent` and moves with `PI_CODING_AGENT_DIR`
        // (references/pi-1.3.0/packages/coding-agent/src/config.ts:563-568).
        let pi_agent = std::env::var_os("PI_CODING_AGENT_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".pi").join("agent"));
        roots.push(pi_agent.join("skills"));
        // Epsilon's own location, last so a skill written for this harness wins
        // a name clash against every compatibility root above it.
        roots.push(home.join(".config").join("epsilon").join("skills"));
    }
    roots
}

/// Project-tier (workspace) skill roots, in ascending precedence.
///
/// **Every one of these is repository-controlled**, which is why the caller — not this
/// function — decides whether to ask for them: [`crate::skills::all_roots`] takes the trust
/// bit. A repository that ships `.claude/skills/` is writing instructions for whoever opens
/// it, exactly as one that ships `.epsilon/skills/` is, and an untrusted checkout must not
/// have its instructions loaded before the user has agreed to read them.
pub fn project_roots(workspace: &Path) -> Vec<PathBuf> {
    vec![
        // Claude Code project skills (one level; nested `<subdir>/.claude/skills`
        // is deliberately not walked).
        workspace.join(".claude").join("skills"),
        // OpenAI Codex project skills.
        workspace.join(".codex").join("skills"),
        // Gemini CLI workspace skills.
        workspace.join(".gemini").join("skills"),
        // Cursor (community-documented convention).
        workspace.join(".cursor").join("skills"),
        // Windsurf.
        workspace.join(".windsurf").join("skills"),
        // opencode reads both spellings; the singular is documented beside the
        // plural and the loader supports `{skill,skills}/**/SKILL.md`.
        workspace.join(".opencode").join("skill"),
        workspace.join(".opencode").join("skills"),
        // Pi's native config dir (CONFIG_DIR_NAME). This harness began as a port of Pi,
        // so a skill written for it is a skill written for this one's shape.
        workspace.join(".pi").join("skills"),
        // The portable alias.
        workspace.join(".agents").join("skills"),
        // DeepSeek Harness — the same convention the reference tree in `references/` uses
        // (`packages/skill/skill-filesystem/src/index.ts:250`), and its documented
        // project-dsh rank outranks project-agents.
        workspace.join(".dsh").join("skills"),
        // Epsilon's native location last, so a project's own skill wins a clash.
        workspace.join(".epsilon").join("skills"),
    ]
}

/// A key under which two spellings of one directory collide.
///
/// **Why this exists.** Running Epsilon in your home directory puts the same directory in both
/// tiers: `~/.claude/skills` from [`user_roots`] and `<home>/.claude/skills` from
/// [`project_roots`]. Scanning it twice would find every skill in it a second time, and
/// [`discover_with_notices`] would announce each one as shadowing itself — a notice that names
/// two identical paths and says one won, on every launch, for every skill the user has.
///
/// The same collision happens through a symlinked or differently-spelled workspace
/// (`/c/Users/hp` versus `C:\Users\Hp`), which is why the key canonicalizes rather than
/// compares strings. Most of these roots do not exist, so `canonicalize` on the root itself
/// usually fails; the key then canonicalizes the nearest existing ancestor and re-attaches the
/// remainder, which is enough to make the two spellings agree.
fn root_key(root: &Path) -> PathBuf {
    if let Ok(canonical) = root.canonicalize() {
        return canonical;
    }
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = root.to_path_buf();
    while let Some(name) = cursor.file_name() {
        suffix.push(name.to_os_string());
        match cursor.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => cursor = parent.to_path_buf(),
            _ => break,
        }
    }
    suffix.reverse();
    let base = if cursor.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        cursor
    };
    match base.canonicalize() {
        Ok(canonical) => {
            let mut key = canonical;
            for part in suffix {
                key.push(part);
            }
            key
        }
        // Nothing in the path resolves: keep the root as written. Two roots that both fail to
        // canonicalize are compared as they were spelled, which is the best available answer
        // and never silently drops a root.
        Err(_) => root.to_path_buf(),
    }
}

/// Drop roots that resolve to a directory already in the list, keeping the first.
///
/// The first occurrence is the one that keeps its place in the precedence order, so a
/// duplicate never promotes a root past the `.agents` alias.
fn dedupe_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out: Vec<PathBuf> = Vec::new();
    for root in roots {
        let key = root_key(&root);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(root);
    }
    out
}

/// Every skill root this launch scans: the user's own, then the workspace's **when trusted**.
///
/// This is the one place the table and the trust gate meet, and it is here rather than in
/// `epsilon-runtime` because two front ends need it: the composition path builds the running
/// agent through it, and the terminal's `/reload` re-reads the same list mid-session.
/// `epsilon-tui` cannot depend on `epsilon-runtime` (that edge would be a cycle in review
/// terms — the runtime is a *dev*-dependency of the TUI, and the shipped binary must not
/// reach into the thing that composes it), so a table owned by runtime would have to be
/// copied into the TUI, which is the second-copy-of-composition defect `AGENTS.md` §2.2 names.
/// `epsilon-core` is the crate both already depend on.
pub fn all_roots(workspace: &Path, project_trusted: bool) -> Vec<PathBuf> {
    let mut roots = user_roots();
    if project_trusted {
        roots.extend(project_roots(workspace));
    }
    dedupe_roots(roots)
}

/// [`all_roots`] plus caller-supplied extra directories, appended last so they
/// win name collisions (DSH ranks `customSkillDirs` above both tiers for the
/// same reason: an explicitly configured directory outranks a convention).
///
/// The extras arrive as values, not as a config read, because only the caller
/// knows which config file is in force — and in tests, no file at all. Pass
/// [`crate::config::load_extra_skill_dirs`] (or the `_in` variant under test)
/// in production. Extras skip the project trust gate like the user tier:
/// they were written into the user's own config, not shipped by a repo.
pub fn all_roots_with_extra(
    workspace: &Path,
    project_trusted: bool,
    extra: &[PathBuf],
) -> Vec<PathBuf> {
    let mut roots = all_roots(workspace, project_trusted);
    roots.extend(extra.iter().cloned());
    dedupe_roots(roots)
}

/// One discovered skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Validated skill name (directory name unless frontmatter overrides; file stem
    /// for a flat `<name>.md` skill, where a frontmatter `name:` is ignored).
    pub name: String,
    /// One-line purpose shown in the catalog.
    pub description: String,
    /// Where the skill lives: the directory containing `SKILL.md` for a bundle, the
    /// parent skills **root** for a flat `<name>.md` file.
    ///
    /// Kept for the bundle file sample and base-directory guidance. The file to
    /// load is [`Self::file`], not a second guess from this field.
    pub dir: PathBuf,
    /// When true, only slash-command style invocation may load it (not model choice).
    pub disable_model_invocation: bool,
    /// The exact file the body is read from: `<dir>/SKILL.md` for a bundle,
    /// `<root>/<name>.md` for a flat skill.
    ///
    /// Carried rather than re-derived from `dir` and `name`, because that
    /// derivation was wrong in a real tree: a flat skill's `dir` is the shared
    /// root, so the loader tried `<root>/SKILL.md` first and a root-level
    /// `SKILL.md` made every flat skill in that root load the same wrong body.
    pub file: PathBuf,
}

impl Skill {
    /// The skills **root** this skill was discovered under.
    ///
    /// A bundle's `dir` is the skill directory (one level below its root); a
    /// flat skill's `dir` already is the root. Scope labels and RPC
    /// `sourceInfo` need the root/file distinction that `dir` alone lost.
    pub fn root(&self) -> &Path {
        if self.file.file_name().is_some_and(|name| name == "SKILL.md") {
            self.dir.parent().unwrap_or(&self.dir)
        } else {
            &self.dir
        }
    }
}

/// Parse `---` delimited frontmatter into key/value pairs (flat strings only).
/// Returns the pairs and the body AFTER the closing marker.
///
/// Unknown keys (`whenToUse`, `metadata`, licence blocks, ...) are ignored, not
/// preserved: opencode reads the same files and ignores what it does not know, and
/// discovery has no field worth adding to carry them — the catalog prints name and
/// description, the loader prints the body, and nothing in between is rendered
/// anywhere. (Deferred, not overlooked.)
fn parse_frontmatter(raw: &str) -> (Vec<(String, String)>, String) {
    // A UTF-8 BOM is legal in a file somebody saved on Windows and must not hide
    // the opening fence; CRLF is normalized by `lines()`.
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut lines = raw.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (Vec::new(), raw.to_string());
    }
    let mut pairs = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let mut in_frontmatter = true;
    let mut closed = false;
    while let Some(line) = lines.next() {
        if in_frontmatter && line.trim() == "---" {
            in_frontmatter = false;
            closed = true;
            continue;
        }
        if !in_frontmatter {
            body.push(line);
            continue;
        }
        // Comments and blank lines are legal YAML and carry no key.
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value.trim();
        // Block scalars are the shape a multi-line `description:` takes in the
        // references (`description: |`). Without this the description was the
        // literal `|`, which passed the non-empty check and reached the catalog.
        let value = if value == "|" || value == ">" {
            let folded = value == ">";
            let mut block: Vec<String> = Vec::new();
            while let Some(next) = lines.clone().next() {
                if next.starts_with(' ') || next.starts_with('\t') {
                    block.push(next.trim().to_string());
                    lines.next();
                } else {
                    break;
                }
            }
            if folded {
                block.join(" ")
            } else {
                block.join("\n")
            }
        } else {
            unquote_scalar(value)
        };
        pairs.push((key, value.trim().to_string()));
    }
    if !closed {
        // Unterminated frontmatter: treat the whole file as body, not as empty.
        return (Vec::new(), raw.to_string());
    }
    (pairs, body.join("\n"))
}

/// Strip one layer of matching YAML quotes from a scalar.
///
/// The parser is deliberately not a YAML implementation; it handles the two
/// shapes a hand-written SKILL.md actually uses — `name: "foo"` and
/// `name: 'foo'` — and leaves everything else verbatim. A full YAML dependency
/// for four keys would be a supply-chain decision this file does not need.
fn unquote_scalar(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
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

/// Discover skills across root directories (each scanned one level deep for
/// `SKILL.md` bundles and top-level `<name>.md` flat files).
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
/// * **an entry the directory could not hand over**, and **a `SKILL.md` or `<name>.md`
///   that is there but cannot be read** — the latter is not the same as a directory with
///   no `SKILL.md`, which is a skills root holding something else and is not a defect.
/// * **a skill the name or description rules reject** — the author wrote it and it will
///   never load. Silence here is what makes "my skill does nothing" unfixable.
/// * **a name collision** — both paths and the winner. See [`discover`] for why the later
///   root wins; that behaviour is unchanged, it is now only announced.
///
/// What deliberately does **not** produce one: a file in a skills root that is neither
/// a skill directory nor a `<name>.md` flat skill (a top-level `SKILL.md` is excluded
/// outright), and a directory that is not a skill. Those are ordinary trees, and a
/// notice per unrelated entry would bury the ones that matter.
pub fn discover_with_notices(roots: &[PathBuf]) -> Discovered {
    let mut out: Vec<Skill> = Vec::new();
    let mut notices: Vec<String> = Vec::new();
    // Real paths of skill files already taken. Two roots can reach one file — a linked
    // skill directory, a linked flat file, or the same root listed twice — and without
    // this the second copy either doubles the skill or announces that the skill shadows
    // itself. This mirrors Pi's `realPathSet`; the roots-level dedupe above stays as it
    // is, because two roots are not the same thing as two routes to one file.
    let mut seen_files: Vec<PathBuf> = Vec::new();
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
            let path = e.path();
            // DSH keeps bundled/system skills in a `.system` child that is not a
            // user-authored tree; scanning it produces invalid-name notices for
            // files the user never wrote.
            if e.file_name() == ".system" {
                continue;
            }
            // Two shapes reach the same validation below. A directory may hold a bundle
            // (`<dir>/SKILL.md`); a top-level `<name>.md` file IS the skill — DSH,
            // opencode and Pi all read loose files in a skills root, and a harness that
            // reads only bundles requires their authors to rewrap every file by hand.
            // Anything else is ordinary tree and stays silent.
            //
            // A top-level `SKILL.md` is excluded outright: it is not inside a skill
            // directory, and `SKILL` is not a valid stem, so without this it would fail
            // the name rules noisily on every launch for a file that names no skill.
            let (skill_file, dir, default_name, origin, is_flat) = if path.is_dir() {
                let dir_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let origin = path.clone();
                let skill_file = path.join("SKILL.md");
                (skill_file, path, dir_name, origin, false)
            } else if path.is_file()
                && path.extension().is_some_and(|ext| ext == "md")
                && !path.file_name().is_some_and(|name| name == "SKILL.md")
                && !path
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("README.md"))
            {
                let stem = path
                    .file_stem()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let origin = path.clone();
                // `dir` is the file's parent root: the bundle directory's only role is
                // locating its `SKILL.md`, and the root plays that role for a flat file
                // (see [`Skill::dir`]).
                (path, root.clone(), stem, origin, true)
            } else {
                continue;
            };
            // An already-seen file reached by a second route is skipped silently: there
            // is no second skill here, so there is nothing to announce — not a shadow,
            // not a problem. A path that does not resolve (a bundle directory with no
            // `SKILL.md` in it) keys on itself, which never collides with a real file.
            let file_key = skill_file
                .canonicalize()
                .unwrap_or_else(|_| skill_file.clone());
            if seen_files.contains(&file_key) {
                continue;
            }
            seen_files.push(file_key);
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

            let name = if is_flat {
                // The stem names a flat skill; a frontmatter `name:` is ignored rather
                // than honoured. The loader finds a flat skill at `<dir>/<name>.md`
                // from the catalogued name alone, so honouring an override would
                // discover a skill the `skill` tool cannot then open — and the filename
                // is the name on disk in every harness that reads loose files.
                default_name
            } else {
                get("name").unwrap_or(default_name)
            };
            if !valid_name(&name) {
                notices.push(format!(
                    "skills: ignoring {}: '{name}' is not a valid skill name (lowercase a-z, \
                     0-9 and hyphens, no leading, trailing or doubled hyphen, at most \
                     {MAX_NAME} characters)",
                    origin.display()
                ));
                continue;
            }
            let description = get("description").unwrap_or_default();
            if description.is_empty() {
                notices.push(format!(
                    "skills: ignoring {}: no description, and the description is the only line \
                     the catalog shows before a skill is invoked",
                    origin.display()
                ));
                continue;
            }
            // The spec's cap is characters, not bytes: a description written in
            // Japanese is three bytes per character and was rejected at a third of
            // the allowed length.
            let description_chars = description.chars().count();
            if description_chars > MAX_DESC {
                notices.push(format!(
                    "skills: ignoring {}: description is {description_chars} characters, over the \
                     {MAX_DESC}-character limit",
                    origin.display()
                ));
                continue;
            }
            let skill = {
                let raw_flag = get("disable-model-invocation");
                let Some(disable_model_invocation) = invocation_flag(raw_flag.clone()) else {
                    notices.push(format!(
                        "skills: ignoring {}: unrecognized disable-model-invocation '{}'",
                        origin.display(),
                        raw_flag.unwrap_or_default()
                    ));
                    continue;
                };
                Skill {
                    name,
                    description,
                    dir,
                    disable_model_invocation,
                    file: skill_file,
                }
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
/// Returns `None` when the key is present but holds no recognised spelling. The caller
/// then **skips the skill with a notice** instead of guessing: DSH drops a skill whose
/// boolean it cannot read and warns, and that strictness is the standard here — Pi's
/// fail-open (load it as invocable) is weaker, and the interim fail-closed answer
/// (load it with the restriction applied) still loaded a skill on a guess about what
/// the author meant. The notice channel carries the warning, so nothing is silent.
///
/// Absent means `Some(false)`: the key was not written, so there is no restriction.
fn invocation_flag(value: Option<String>) -> Option<bool> {
    let Some(raw) = value else { return Some(false) };
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        // Unrecognised: no guess. The caller drops the skill and says so.
        _ => None,
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
    catalog_with_disabled(skills, &[])
}

/// [`catalog`] minus the skills the user switched off.
///
/// **Why this is separate.** The catalog used to filter only
/// `disable-model-invocation`, so `/skill <name> off` removed the skill from the
/// tool while the prompt still advertised it. The model then read a name it could
/// not load — a documented capability that answers with an error. Both the tool
/// and the catalog now consult the same disabled list.
pub fn catalog_with_disabled(skills: &[Skill], disabled: &[String]) -> String {
    let loadable = skills
        .iter()
        .filter(|skill| {
            !skill.disable_model_invocation && !disabled.iter().any(|name| name == &skill.name)
        })
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
            file: std::path::PathBuf::from(name).join("SKILL.md"),
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
    fn a_disabled_skill_is_not_advertised_in_the_catalog() {
        // `/skill off` removed the skill from the tool but the prompt kept naming
        // it. One switch, two answers, and the model was told to load a skill the
        // tool would refuse.
        let skills = vec![skill("open", false), skill("switched-off", false)];
        let text = catalog_with_disabled(&skills, &["switched-off".to_string()]);
        assert!(text.contains("open"), "{text}");
        assert!(
            !text.contains("switched-off"),
            "a disabled skill must not be advertised: {text}"
        );
        assert!(
            catalog_with_disabled(&skills, &[]).contains("switched-off"),
            "the same list without the disable must still advertise it"
        );
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

    /// An unrecognised spelling **skips the skill with a notice**.
    ///
    /// DSH is the standard here, not Pi: DSH drops a skill whose boolean it cannot read
    /// and warns, while Pi loads it fail-open. Dropping is the stricter answer and the
    /// notice channel now exists to carry the warning, so a skill whose author wrote
    /// something unreadable in a capability field never loads in any capacity — neither
    /// as model-invocable (the old fail-open bug) nor as restricted (the interim
    /// fail-closed answer, which loaded a skill on a guess about what the author meant).
    #[test]
    fn an_unrecognised_spelling_skips_the_skill_with_a_notice() {
        for spelling in [
            "disable-model-invocation: perhaps",
            "disable-model-invocation:",
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_raw(dir.path(), "mystery", &format!("{spelling}\n"));
            let found = discover_with_notices(&[dir.path().to_path_buf()]);
            assert!(
                found.skills.iter().all(|s| s.name != "mystery"),
                "an unreadable capability field must not load the skill: {found:?}"
            );
            assert!(
                found.notices.iter().any(|n| n.contains("mystery")
                    && n.contains("unrecognized disable-model-invocation")),
                "dropping a skill the author wrote must be announced: {found:?}"
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

    // ---------------------------------------------------------------------------------------------
    // Flat-file skills: a top-level `<name>.md` directly inside a root
    // ---------------------------------------------------------------------------------------------

    /// Write a flat-file skill: `<root>/<name>.md` with the given frontmatter lines.
    fn write_flat_skill(root: &Path, name: &str, extra: &str) {
        std::fs::write(
            root.join(format!("{name}.md")),
            format!("---\ndescription: a thing\n{extra}---\nbody\n"),
        )
        .unwrap();
    }

    /// **A top-level `<name>.md` is a skill named for its stem.**
    ///
    /// DSH, opencode and Pi all read loose markdown files in a skills root; Epsilon used
    /// to walk past them because only `<name>/SKILL.md` directories counted. `dir` is the
    /// file's parent root — the bundle's directory plays no other role than locating its
    /// `SKILL.md`, and the root plays that role for a flat file — and the name is the
    /// file stem, validated by the same rules as a directory name.
    #[test]
    fn a_top_level_md_file_is_a_skill_named_for_its_stem() {
        let dir = tempfile::tempdir().unwrap();
        write_flat_skill(dir.path(), "commit-craft", "");
        write_skill(dir.path(), "bundled", "A bundle.", "body");

        let found = discover_with_notices(&[dir.path().to_path_buf()]);
        assert_eq!(found.skills.len(), 2, "{found:?}");
        let flat = found
            .skills
            .iter()
            .find(|s| s.name == "commit-craft")
            .expect("the flat file must be discovered");
        assert_eq!(
            flat.dir,
            dir.path(),
            "a flat skill's dir is its parent root: {flat:?}"
        );
        assert_eq!(flat.description, "a thing");
        assert!(
            found.notices.is_empty(),
            "a well-formed flat file is not a problem: {found:?}"
        );
    }

    /// A flat file without a description is announced, exactly like a bundle without one.
    #[test]
    fn a_flat_skill_without_a_description_is_announced_like_a_bundle() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("no-desc.md"), "---\n---\nbody\n").unwrap();

        let found = discover_with_notices(&[dir.path().to_path_buf()]);
        assert!(found.skills.is_empty(), "{found:?}");
        assert_eq!(found.notices.len(), 1, "{found:?}");
        assert!(
            found.notices[0].contains("no-desc.md") && found.notices[0].contains("description"),
            "the notice must name the file and say what is missing: {found:?}"
        );
    }

    /// A flat file whose stem breaks the name rules is announced, not loaded.
    #[test]
    fn a_flat_skill_with_an_invalid_stem_is_announced() {
        let dir = tempfile::tempdir().unwrap();
        write_flat_skill(dir.path(), "Bad_Name", "");

        let found = discover_with_notices(&[dir.path().to_path_buf()]);
        assert!(found.skills.is_empty(), "{found:?}");
        assert_eq!(found.notices.len(), 1, "{found:?}");
        assert!(
            found.notices[0].contains("Bad_Name") && found.notices[0].contains("skill name"),
            "the stem answers to the same rules as a directory name: {found:?}"
        );
    }

    /// **A top-level `SKILL.md` is not a skill, and saying so every launch would be noise.**
    ///
    /// It matches neither shape — it is not inside a skill directory, and `SKILL` is not a
    /// valid stem (uppercase) — so without an explicit exclusion it would fail the name
    /// rules noisily on every launch. Excluding it keeps the channel honest: a notice must
    /// mean something the author can fix, and there is no skill here to fix.
    #[test]
    fn a_top_level_skill_md_is_excluded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("SKILL.md"),
            "---\nname: stray\ndescription: a thing\n---\nbody\n",
        )
        .unwrap();

        let found = discover_with_notices(&[dir.path().to_path_buf()]);
        assert!(found.skills.is_empty(), "{found:?}");
        assert!(
            found.notices.is_empty(),
            "an excluded file is not a problem to report: {found:?}"
        );
    }

    #[test]
    fn a_readme_in_a_skills_root_is_documentation_not_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "# skills\n").unwrap();

        let found = discover_with_notices(&[dir.path().to_path_buf()]);
        assert!(found.skills.is_empty(), "{found:?}");
        assert!(
            found.notices.is_empty(),
            "a README is documentation, not an invalid skill to report on every launch: {found:?}"
        );
    }

    /// An unreadable flat file is announced, like an unreadable bundle file.
    #[test]
    fn an_unreadable_flat_file_is_announced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.md"), [0xff, 0xfe, 0x00]).unwrap();

        let found = discover_with_notices(&[dir.path().to_path_buf()]);
        assert!(found.skills.is_empty(), "{found:?}");
        assert_eq!(found.notices.len(), 1, "{found:?}");
        assert!(
            found.notices[0].contains("broken.md"),
            "the notice must name the file it could not read: {found:?}"
        );
    }

    /// **A flat skill and a bundle of the same name collide like any two skills.**
    ///
    /// The shape on disk does not matter to precedence: the later root still wins, and
    /// the notice still names both sides.
    #[test]
    fn a_flat_skill_collides_with_a_bundle_of_the_same_name() {
        let earlier = tempfile::tempdir().unwrap();
        let later = tempfile::tempdir().unwrap();
        write_flat_skill(earlier.path(), "shared", "");
        write_skill(later.path(), "shared", "The bundle copy.", "body");

        let roots = vec![earlier.path().to_path_buf(), later.path().to_path_buf()];
        let found = discover_with_notices(&roots);
        assert_eq!(found.skills.len(), 1, "one name, one winner: {found:?}");
        assert_eq!(
            found.skills[0].description, "The bundle copy.",
            "the later root wins across shapes too: {found:?}"
        );
        assert_eq!(found.notices.len(), 1, "one collision, one line: {found:?}");
    }

    // ---------------------------------------------------------------------------------------------
    // Symlink dedupe: one file reached twice loads once
    // ---------------------------------------------------------------------------------------------

    /// **One root listed twice is scanned once.**
    ///
    /// No link is involved — the same `SKILL.md` simply arrives twice — and without a
    /// file-level dedupe the second copy announces that the skill shadows itself, a
    /// notice naming two identical paths and declaring a winner.
    #[test]
    fn one_root_listed_twice_loads_each_skill_once_and_says_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "solo", "The only copy.", "body");

        let roots = vec![dir.path().to_path_buf(), dir.path().to_path_buf()];
        let found = discover_with_notices(&roots);
        assert_eq!(found.skills.len(), 1, "one file, one skill: {found:?}");
        assert!(
            found.notices.is_empty(),
            "and no notice saying the skill shadows itself: {found:?}"
        );
    }

    /// **One file reached through two roots loads once and says nothing.**
    ///
    /// This is Pi's `realPathSet`: the second root reaches the same `SKILL.md` through
    /// a link, and discovery that cannot tell answers with the skill twice — or, where
    /// the names agree, with a notice claiming the skill shadows itself. The link is a
    /// directory link (`test_support::dir_link`: a junction on Windows, a symlink
    /// elsewhere), following the dangling-root test's lead — a file link needs a
    /// privilege a stock Windows machine does not hold, and a hard link is a second
    /// directory entry rather than a second route to one file, so neither proves what
    /// this test names.
    #[test]
    fn one_file_reached_through_two_roots_loads_once_and_says_nothing() {
        let first = tempfile::tempdir().unwrap();
        write_skill(first.path(), "shared", "The one copy.", "body");
        let second = tempfile::tempdir().unwrap();
        assert!(
            crate::test_support::dir_link(
                &first.path().join("shared"),
                &second.path().join("shared"),
            ),
            "the platform refused to create a directory link, so this test asserted nothing"
        );

        let roots = vec![first.path().to_path_buf(), second.path().to_path_buf()];
        let found = discover_with_notices(&roots);
        assert_eq!(found.skills.len(), 1, "one file, one skill: {found:?}");
        assert_eq!(
            found.skills[0].dir,
            first.path().join("shared"),
            "the first route wins, because the second never loads: {found:?}"
        );
        assert!(
            found.notices.is_empty(),
            "and no notice saying the skill shadows itself: {found:?}"
        );
    }

    // ---------------------------------------------------------------------------------------------
    // The cross-harness root table
    // ---------------------------------------------------------------------------------------------

    /// Write a `SKILL.md` under `<root>/<name>/`, creating the root on the way.
    fn write_skill_at(root: &Path, name: &str, description: &str, body: &str) {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n{body}"),
        )
        .unwrap();
    }

    /// **A skill another harness wrote is a skill this one may use.**
    ///
    /// This is the whole point of the table, and the failure it replaces was total: the
    /// root list used to be two directories long (`.epsilon/skills` and `.agents/skills`),
    /// so a user who had been running Claude Code or Codex for a year had dozens of skills
    /// on disk and this harness read none of them. Nothing reported it either — an empty
    /// catalog and a `/skills` command that said "no skills discovered", which is the
    /// silent failure `AGENTS.md` §2.7 names.
    ///
    /// Drives the real discovery over the real `project_roots` list rather than a
    /// hand-picked root, so a root added to the table is covered by this test.
    #[test]
    fn a_skill_written_for_another_harness_is_discovered_and_loaded() {
        let ws = tempfile::tempdir().unwrap();
        for (dir, name) in [
            (".claude", "claude-wrote-me"),
            (".codex", "codex-wrote-me"),
            (".gemini", "gemini-wrote-me"),
            (".cursor", "cursor-wrote-me"),
            (".windsurf", "windsurf-wrote-me"),
            (".opencode", "opencode-wrote-me"),
            (".dsh", "dsh-wrote-me"),
            (".pi", "pi-wrote-me"),
            (".agents", "agents-wrote-me"),
            (".epsilon", "epsilon-wrote-me"),
        ] {
            write_skill_at(
                &ws.path().join(dir).join("skills"),
                name,
                "From that tool.",
                "body",
            );
        }

        let roots = all_roots(ws.path(), true);
        // The wiring, not just the table: every project root must appear in the list
        // `all_roots` hands to discovery, or the table is documentation.
        for root in project_roots(ws.path()) {
            assert!(
                roots.contains(&root),
                "{root:?} is a project root and all_roots must hand it to discovery: {roots:?}"
            );
        }
        // Discovery runs over the project roots alone. `all_roots` also carries the real
        // home directory's skill tree, and what this machine happens to have installed in
        // `~/.claude/skills` is not this test's subject.
        let found = discover_with_notices(&project_roots(ws.path()));
        for name in [
            "claude-wrote-me",
            "codex-wrote-me",
            "gemini-wrote-me",
            "cursor-wrote-me",
            "windsurf-wrote-me",
            "opencode-wrote-me",
            "dsh-wrote-me",
            "pi-wrote-me",
            "agents-wrote-me",
            "epsilon-wrote-me",
        ] {
            assert!(
                found.skills.iter().any(|s| s.name == name),
                "{name} is on disk under one of the project roots and discovery must reach \
                 it: {found:?}"
            );
        }
        assert!(
            found.notices.is_empty(),
            "ten well-formed skills is ten well-formed skills, not ten problems: {found:?}"
        );
    }

    /// **The portable alias outranks the tool-specific directory inside a tier.**
    ///
    /// This is Gemini CLI's documented rule (`geminicli.com/docs/cli/skills`: "the
    /// `.agents/skills/` alias takes precedence over the `.gemini/skills/` directory"),
    /// and following it is what makes a shared skill tree behave the same in every tool
    /// that reads it. The alternative — Epsilon inventing its own precedence — would mean
    /// a user's one skill tree resolves differently depending on which harness they
    /// happened to open, with nothing on screen to explain the difference.
    #[test]
    fn within_a_tier_the_portable_agents_alias_outranks_the_tool_specific_directory() {
        let ws = tempfile::tempdir().unwrap();
        write_skill_at(
            &ws.path().join(".claude").join("skills"),
            "shared",
            "The Claude copy.",
            "claude-body",
        );
        write_skill_at(
            &ws.path().join(".agents").join("skills"),
            "shared",
            "The portable copy.",
            "agents-body",
        );

        let found = discover_with_notices(&project_roots(ws.path()));
        assert_eq!(found.skills.len(), 1, "one name, one winner: {found:?}");
        assert_eq!(
            found.skills[0].description, "The portable copy.",
            "the portable alias wins the project tier: {found:?}"
        );
    }

    /// **A workspace skill outranks a user skill.**
    ///
    /// The other half of the documented precedence, and the one that matters for a
    /// repository: a team's standard version of `commit` must beat each member's personal
    /// one, or the team standard is advisory.
    #[test]
    fn a_workspace_skill_outranks_a_user_skill_of_the_same_name() {
        let ws = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        write_skill_at(user.path(), "commit", "Mine.", "user-body");
        write_skill_at(
            &ws.path().join(".epsilon").join("skills"),
            "commit",
            "The team's.",
            "project-body",
        );

        // Both roots, in the order `all_roots` puts them: the user tier first, so the
        // project tier lands later and wins.
        let mut roots = vec![user.path().to_path_buf()];
        roots.extend(project_roots(ws.path()));
        let found = discover_with_notices(&roots);
        assert_eq!(
            found.skills[0].description, "The team's.",
            "the later root wins, and the project root is the later one: {found:?}"
        );
    }

    /// **An untrusted workspace contributes no roots at all.**
    ///
    /// Every project-tier directory is repository-controlled, so a `.claude/skills` in a
    /// checkout the user has not agreed to trust is exactly as gated as an `.epsilon/skills`
    /// one. The user tier survives: those files are the user's own, in the user's own home
    /// directory, and a repository has no say in them.
    #[test]
    fn an_untrusted_workspace_contributes_no_roots_and_the_user_tier_survives() {
        let ws = tempfile::tempdir().unwrap();
        for root in project_roots(ws.path()) {
            std::fs::create_dir_all(&root).unwrap();
        }

        let untrusted = all_roots(ws.path(), false);
        assert!(
            untrusted.iter().all(|r| !r.starts_with(ws.path())),
            "nothing under an untrusted workspace may be scanned: {untrusted:?}"
        );
        let trusted = all_roots(ws.path(), true);
        assert!(
            trusted.iter().any(|r| r.starts_with(ws.path())),
            "and trusting it unlocks them: {trusted:?}"
        );
    }

    /// **Two spellings of one directory are scanned once.**
    ///
    /// Running Epsilon in your home directory puts the same directory in both tiers:
    /// `~/.claude/skills` from the user list, `<home>/.claude/skills` from the project
    /// list. Scanning it twice would find every skill a second time, and
    /// `discover_with_notices` would announce each one as shadowing itself — a notice
    /// naming two identical paths and declaring a winner, on every launch, for every
    /// skill the user owns. That notice is what makes this test's assertion observable:
    /// with the dedupe removed, `notices` is non-empty rather than merely the list being
    /// longer.
    #[test]
    fn one_directory_spelled_two_ways_is_scanned_once() {
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path().join(".claude").join("skills");
        write_skill_at(&root, "solo", "The only copy.", "body");

        // The same directory reached by two routes: as a workspace root and as a
        // differently-spelled path to it. `ws.path()` is already absolute, so the second
        // spelling goes through the `..` form, which is the same directory.
        let aliased = ws
            .path()
            .join(".claude")
            .join("..")
            .join(".claude")
            .join("skills");
        let roots = dedupe_roots(vec![root.clone(), aliased.clone()]);
        assert_eq!(
            roots.len(),
            1,
            "two spellings of one directory must collapse to one root: {roots:?}"
        );

        // The observable half: one skill, and no claim that it shadows something.
        let found = discover_with_notices(&roots);
        assert_eq!(found.skills.len(), 1, "one copy, one skill: {found:?}");
        assert!(
            found.notices.is_empty(),
            "and no notice saying a skill shadows itself: {found:?}"
        );
    }

    /// **The dedupe keeps the first spelling, so precedence is not silently rewritten.**
    ///
    /// Dropping the *later* duplicate instead would be equally correct as a set operation
    /// and wrong as a precedence rule: the surviving root's position in the list is what
    /// decides which skill wins a name collision.
    #[test]
    fn the_dedupe_keeps_the_first_spelling_rather_than_the_last() {
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        let aliased = ws.path().join(".").join("skills");

        let kept = dedupe_roots(vec![root.clone(), aliased]);
        assert_eq!(
            kept,
            vec![root],
            "the first spelling keeps its place in the precedence order: {kept:?}"
        );
    }

    /// **A root that resolves to nothing still gets a stable key.**
    ///
    /// Most of these directories do not exist, which is the ordinary case and must stay
    /// silent. `canonicalize` fails on all of them, so the key walks up to the nearest
    /// ancestor that does resolve; if even that fails, the root is compared as spelled.
    /// Either way a root is never dropped from the list for want of a key.
    #[test]
    fn a_root_that_does_not_exist_still_gets_a_usable_key() {
        let ws = tempfile::tempdir().unwrap();
        let missing = ws.path().join("nope").join("deeper").join("skills");
        let key = root_key(&missing);
        assert!(
            key.ends_with("skills"),
            "the key must still identify the directory: {key:?}"
        );
        // Two spellings of a directory that does not exist collide on the same ancestor.
        let spelled_twice = ws
            .path()
            .join(".")
            .join("nope")
            .join("deeper")
            .join("skills");
        assert_eq!(
            root_key(&spelled_twice),
            key,
            "the key is derived from the nearest existing ancestor, so both spellings agree"
        );
    }

    /// Frontmatter shapes a hand-written SKILL.md actually uses: a BOM from a
    /// Windows editor, quoted strings, a block description and CRLF.
    #[test]
    fn frontmatter_survives_bom_quotes_blocks_and_crlf() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("alpha");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "\u{feff}---\r\nname: \"alpha\"\r\ndescription: >\r\n  quoted and folded\r\n  onto one line\r\n---\r\n\r\nBODY\r\n",
        )
        .unwrap();
        let found = discover_with_notices(&[tmp.path().to_path_buf()]);
        assert!(found.notices.is_empty(), "{:?}", found.notices);
        let skill = found.skills.first().expect("the skill loads");
        assert_eq!(skill.name, "alpha", "quotes are not part of the name");
        assert_eq!(
            skill.description, "quoted and folded onto one line",
            "a folded block is one line, and CRLF does not leak into it"
        );

        // A literal block keeps its line breaks; comments do not become keys.
        let dir = tmp.path().join("beta");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\n# a comment\nname: beta\ndescription: |\n  line one\n  line two\n---\nBODY\n",
        )
        .unwrap();
        let found = discover_with_notices(&[tmp.path().to_path_buf()]);
        let beta = found
            .skills
            .iter()
            .find(|s| s.name == "beta")
            .expect("beta loads");
        assert_eq!(beta.description, "line one\nline two");
    }

    /// The DSH `.system` child is bundled material, not a user skill root entry.
    #[test]
    fn a_dot_system_child_is_skipped_without_a_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let system = tmp.path().join(".system").join("bundled");
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(
            system.join("SKILL.md"),
            "---\nname: Bundled_Name\ndescription: d\n---\nbody\n",
        )
        .unwrap();
        let found = discover_with_notices(&[tmp.path().to_path_buf()]);
        assert!(found.skills.is_empty(), "{:?}", found.skills);
        assert!(
            found.notices.is_empty(),
            "bundled system files are not the user's broken skill: {:?}",
            found.notices
        );
    }

    /// **The project root list is the one discovery uses, and it is a real list.**
    ///
    /// Not a documentation test for its own sake: `trust_requiring_resources` in
    /// `epsilon-runtime` reads this list to decide what a trust prompt has to name, so a
    /// root added here and not there is a gate with a hole in it. If a new harness
    /// convention is added to `project_roots`, this test is where the reader is told the
    /// other call site exists.
    #[test]
    fn the_project_roots_are_the_conventions_they_claim_to_be() {
        let ws = Path::new("/work/repo");
        let roots = project_roots(ws);
        for dir in [
            ".epsilon",
            ".claude",
            ".codex",
            ".gemini",
            ".cursor",
            ".windsurf",
            ".opencode",
            ".dsh",
            // Pi's native config dir (CONFIG_DIR_NAME): this harness began as a port of
            // it, and its project skills live beside the other harness dirs.
            ".pi",
            ".agents",
        ] {
            assert!(
                roots.contains(&ws.join(dir).join("skills")),
                "{dir}/skills is a convention harnesses read and is missing: {roots:?}"
            );
        }
        assert!(
            roots.contains(&ws.join(".opencode").join("skill")),
            "opencode documents the singular `skill/` spelling too: {roots:?}"
        );
        assert_eq!(
            roots.len(),
            11,
            "the list is exactly the documented conventions: {roots:?}"
        );
        // Epsilon's own root is last, so a project's skill wins a clash; DSH's
        // `.dsh` sits above the portable alias, matching its documented rank.
        assert_eq!(
            roots.last().unwrap(),
            &ws.join(".epsilon").join("skills"),
            "epsilon's own project root must win the tier: {roots:?}"
        );
        assert!(
            roots
                .iter()
                .position(|r| r == &ws.join(".dsh").join("skills"))
                > roots
                    .iter()
                    .position(|r| r == &ws.join(".agents").join("skills")),
            "project-dsh outranks project-agents: {roots:?}"
        );
    }

    /// Extra directories append last (so they win) and skip the trust gate.
    ///
    /// RED-first: extras are the user's own config, not a repo's — holding
    /// them back for trust would ask permission to read what the user just
    /// told this harness to read. And last place is the precedence DSH gives
    /// `customSkillDirs`: explicit beats conventional.
    #[test]
    fn extra_skill_dirs_append_last_and_need_no_trust() {
        let ws = Path::new("/work/repo");
        let extra = PathBuf::from("/opt/shared/skills");
        let trusted = all_roots_with_extra(ws, true, std::slice::from_ref(&extra));
        let plain = all_roots(ws, true);
        assert_eq!(
            trusted.last().unwrap(),
            &extra,
            "an explicit directory outranks conventions: {trusted:?}"
        );
        assert_eq!(
            trusted.len(),
            plain.len() + 1,
            "extras add, never replace: {trusted:?}"
        );
        // Untrusted: the project tier drops out, the extras stay — they came
        // from the user's own file, not from the workspace under review.
        let untrusted = all_roots_with_extra(ws, false, std::slice::from_ref(&extra));
        assert!(
            !untrusted.iter().any(|r| r.starts_with(ws)),
            "no workspace root without trust: {untrusted:?}"
        );
        assert_eq!(
            untrusted.last().unwrap(),
            &extra,
            "the user's own extra survives distrust: {untrusted:?}"
        );
        // A duplicate of a listed root dedupes instead of doubling.
        let dup = all_roots_with_extra(ws, true, &plain[..1]);
        assert_eq!(dup, plain, "an extra naming a known root changes nothing");
    }

    /// The user-tier root table, pinned the way the project tier is above.
    ///
    /// This table had no test: every other roots list has one, and an
    /// unlisted global convention is a skill directory the user filled for
    /// another harness that this one silently never reads. Expected values
    /// are derived from [`crate::config::home_dir`] at runtime rather than
    /// hardcoded, so the test holds under a hermetic `HOME` as well as on a
    /// developer machine — and with no home at all the list is empty.
    #[test]
    fn the_user_roots_are_the_conventions_they_claim_to_be() {
        let roots = user_roots();
        match crate::config::home_dir() {
            None => assert!(
                roots.is_empty(),
                "with no home directory there are no user roots: {roots:?}"
            ),
            Some(home) => {
                // Harness overrides (DSH_HOME, DSH_AGENTS_HOME,
                // PI_CODING_AGENT_DIR) move these roots, so the table is checked
                // by membership and precedence rather than by one exact vector.
                for path in [
                    home.join(".config").join("epsilon").join("builtin-skills"),
                    home.join(".claude").join("skills"),
                    home.join(".codex").join("skills"),
                    home.join(".gemini").join("skills"),
                    home.join(".config").join("opencode").join("skills"),
                    home.join(".pi").join("agent").join("skills"),
                    home.join(".config").join("epsilon").join("skills"),
                ] {
                    assert!(
                        roots.contains(&path),
                        "{} is a global convention and is missing: {roots:?}",
                        path.display()
                    );
                }
                // Epsilon's own root is last, so a skill deliberately written
                // for this harness is never outranked by a compatibility root.
                assert_eq!(
                    roots.last().unwrap(),
                    &home.join(".config").join("epsilon").join("skills"),
                    "epsilon's own root must win the user tier: {roots:?}"
                );
            }
        }
    }
}
