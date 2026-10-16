//! Prompt templates: a `/name` that expands to saved text (pi `prompt-templates.ts`).
//!
//! A template is a markdown file whose body is a prompt you would otherwise retype. Typing
//! `/review src/parser.rs` sends the body of `review.md` with `$1` replaced. The point is
//! repeatability: the same instruction every time, without retyping it or remembering what
//! you said last time.
//!
//! ## Why this lives in `epsilon-contracts`
//!
//! Two independent callers need it — the TUI, to expand what you typed, and the CLI, to
//! expand a `--prompt` argument — and the substitution rules are the part that must agree
//! between them. A second copy of `substitute_args` in the TUI would be a second set of
//! semantics to keep in step, which is exactly how a "faithful port" quietly stops being
//! one. The crate has no filesystem dependency: loading is ``crate::prompt_template::load``
//! taking a directory, and everything else is pure.
//!
//! ## The substitution grammar, in full
//!
//! | Form | Expands to |
//! |---|---|
//! | `$1`, `$2`, … | that argument, or empty if absent |
//! | `$@`, `$ARGUMENTS` | every argument, space-joined |
//! | `${N:-default}` | argument N, or `default` if missing or empty |
//! | `${@:-default}`, `${ARGUMENTS:-default}` | all arguments, or `default` if empty |
//! | `${@:N}` | arguments from N to the end |
//! | `${@:N:L}` | L arguments starting at N |
//!
//! Substitution is **single-pass and not recursive**, matching pi. An argument whose text
//! contains `$1` is inserted literally rather than expanded again. That is not an oversight
//! to be tidied up later: a recursive pass would let a template argument inject a different
//! template's placeholders, and a user pasting a shell transcript into a prompt is exactly
//! how that happens.

use std::path::{Path, PathBuf};

/// One loaded prompt file: a text-expansion template (`prompts/`) or an
/// executable custom command (`commands/`).
///
/// Both are markdown files with the same frontmatter keys and the same `$1 /
/// $@ / ${N:-default}` substitution grammar below. The directories differ in
/// what the TUI does with the body: a template expands to prompt text, a
/// command expands and resubmits through the same submit path (so a body
/// naming a builtin executes it, and a body naming another file does not
/// re-expand — no recursion). One struct because the loading, naming and
/// substitution rules are shared; [`PromptFileKind`] says which directory a
/// file came from so listings can tell the two behaviours apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    /// Invocation name — the file stem, so `review.md` is `/review`.
    pub name: String,
    /// One-line summary, from frontmatter or the first non-empty body line.
    pub description: String,
    /// Argument hint shown in the picker (`argument-hint` in frontmatter).
    pub argument_hint: Option<String>,
    /// Template body, with placeholders not yet expanded.
    pub content: String,
    /// Where it was loaded from, and whether user- or project-scoped.
    ///
    /// Two directories can both contain `review.md`, and the user needs to know which one a
    /// `/review` will actually use. Project scope wins, so a repository can
    /// ship its own `review` without the user touching their global config.
    /// That precedence is the skills rule (a later root wins
    /// — `epsilon-core::skills::discover`, a deliberate divergence from pi,
    /// which keeps the first registration) and opencode's project-over-global
    /// layering, not pi's ordering.
    pub scope: TemplateScope,
    /// Which behaviour the file has: text-expansion or executable command.
    pub kind: PromptFileKind,
    /// The file this came from.
    pub file_path: PathBuf,
}

/// Which directory shape a prompt file was loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptFileKind {
    /// A `prompts/` file: expands to prompt text.
    Template,
    /// A `commands/` file: expands and resubmits through the submit path.
    Command,
}

impl PromptFileKind {
    /// Short label for listings ("template", "command").
    pub fn label(self) -> &'static str {
        match self {
            PromptFileKind::Template => "template",
            PromptFileKind::Command => "command",
        }
    }
}

/// Which directory a template was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateScope {
    /// The user's own config directory.
    User,
    /// The workspace's config directory.
    Project,
    /// An explicit path given on the command line.
    Explicit,
}

impl TemplateScope {
    /// Short label for diagnostics ("user", "project", "explicit").
    pub fn label(self) -> &'static str {
        match self {
            TemplateScope::User => "user",
            TemplateScope::Project => "project",
            TemplateScope::Explicit => "explicit",
        }
    }

    /// Single-letter tag for the slash popup (`[u]`, `[p]`).
    pub fn short(self) -> &'static str {
        match self {
            TemplateScope::User => "u",
            TemplateScope::Project => "p",
            TemplateScope::Explicit => "e",
        }
    }
}

impl PromptTemplate {
    /// Whether this file is an executable custom command (`commands/`)
    /// rather than a text-expansion template (`prompts/`).
    pub fn is_command(&self) -> bool {
        self.kind == PromptFileKind::Command
    }
}

/// Longest command name, in characters.
///
/// The skills cap, on purpose: a command name answers to the same rules as a
/// skill name (lowercase `a-z`, `0-9`, hyphens), so one rule covers both
/// spellings an operator types after `/`.
pub const MAX_NAME: usize = 64;

/// Whether a file stem may become a custom-command name.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

/// Longest description taken from a body's first line before truncation.
const DESC_LIMIT: usize = 60;

/// Split a command's argument string the way a shell would.
///
/// Quoted runs become one argument, so `/commit "fix the parser"` is one argument rather
/// than three. This is a simplified splitter, not a shell: no escapes, no expansion, no
/// substitution. pi's `parseCommandArgs` is the same, and the reason is worth stating —
/// doing more here would make a prompt template's behaviour depend on a shell's, which is
/// the kind of surprise nobody wants when writing a prompt.
pub fn parse_command_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for ch in args.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Expand a template's placeholders against its arguments.
///
/// Single-pass, non-recursive, and byte-exact for values that contain no placeholders. Every
/// branch below is reachable from the table in the module docs; the tests enumerate them.
pub fn substitute_args(content: &str, args: &[String]) -> String {
    let all = args.join(" ");
    let mut out = String::with_capacity(content.len());
    let mut rest = content;

    // Scan for `$` and dispatch on what follows. A hand-written scan rather than a regex
    // crate: `epsilon-contracts` takes no dependencies, and this grammar is small enough
    // that the alternatives would be a dep for one match.
    while let Some(dollar) = rest.find('$') {
        out.push_str(&rest[..dollar]);
        rest = &rest[dollar + 1..];

        // `$$` is an escape for a literal `$`, and it must be checked before the general
        // parse: otherwise `$$1` is ambiguous between "a literal $ followed by argument 1"
        // and "argument 1, twice", and each reading is a plausible mistake. Shell convention
        // wins -- `$$` is one dollar.
        if let Some(after) = rest.strip_prefix('$') {
            out.push('$');
            rest = after;
            continue;
        }

        let Some(expand) = parse_placeholder(rest, args, &all) else {
            // A `$` that starts nothing we recognise is literal text. Escaping is `$` itself,
            // which is the only way to write a literal placeholder.
            out.push('$');
            continue;
        };
        out.push_str(&expand.value);
        rest = expand.rest;
    }
    out.push_str(rest);
    out
}

/// What a recognised placeholder expanded to, and where the scan resumes.
///
/// `rest` borrows from the caller's input, which is why the scanner is generic over a
/// lifetime rather than returning indices.
struct Expanded<'a> {
    value: String,
    rest: &'a str,
}

/// Parse one placeholder at the start of `input`.
///
/// Returns `None` when `input` does not begin with a recognised form, in which case the
/// caller emits a literal `$` and re-scans. `rest` is borrowed from the caller's string via
/// the returned offset, so the signature takes and returns `&str` slices of the same
/// lifetime.
fn parse_placeholder<'a>(input: &'a str, args: &[String], all: &str) -> Option<Expanded<'a>> {
    if let Some(body) = input.strip_prefix('{') {
        let close = body.find('}')?;
        let inner = &body[..close];
        let rest = &body[close + 1..];
        return Some(Expanded {
            value: expand_braced(inner, args, all),
            rest,
        });
    }

    // Bare form: `@`, `ARGUMENTS`, or digits. The longest match wins, so `ARGUMENTS` is not
    // read as the argument named `A`.
    if let Some(rest) = input.strip_prefix("ARGUMENTS") {
        return Some(Expanded {
            value: all.to_string(),
            rest,
        });
    }
    if let Some(rest) = input.strip_prefix('@') {
        return Some(Expanded {
            value: all.to_string(),
            rest,
        });
    }
    let digits: String = input.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = &input[digits.len()..];
    // 1-indexed, like the shell. Out of range yields empty rather than an error: a prompt
    // that references `$2` when only one argument was given should still send *something*.
    let idx: usize = digits.parse::<usize>().ok()?.checked_sub(1)?;
    Some(Expanded {
        value: args.get(idx).cloned().unwrap_or_default(),
        rest,
    })
}

/// The body of a `${...}` placeholder, without the braces.
fn expand_braced(inner: &str, args: &[String], all: &str) -> String {
    // Slices are resolved FIRST, and this ordering is load-bearing rather than stylistic.
    // `strip_default` searches for `:-`, and `${@:2}` contains a colon -- so a default-first
    // order reads "@" as the target and "2" as its fallback, and the slice branch below is
    // never reached for any slice with a non-empty start. The first version of this function
    // had that order and every slice test failed: `${@:2}` returned the whole argument list.
    // `@:` is unambiguous, so it is checked before `:-` and the collision cannot recur.
    // Decide the form ONCE, from the text, instead of by ordering. Two attempts at this by
    // reordering branches each fixed one form by breaking another: defaults-first made
    // `${@:2}` unreadable, slices-first made `${@:-x}` unreadable. The question was never
    // "which branch comes first" -- it is "is this a slice", and a slice is `@` `:` DIGIT.
    // The predicate is at the top so it cannot drift, and each branch below handles only the
    // form it owns.
    let slice_spec: Option<&str> = inner.strip_prefix('@').and_then(|rest| {
        let spec = rest.strip_prefix(':')?;
        let digits: String = spec.chars().take_while(char::is_ascii_digit).collect();
        // A slice has at least one digit right after the colon. `${@:-x}` has `-`, so it
        // fails here and falls through to the default branch below.
        (!digits.is_empty()).then_some(spec)
    });

    // `${@:N}` and `${@:N:L}` -- bash-style slicing. Only `@` takes a slice; `$1:2` is not a
    // thing, and treating it as one would silently accept malformed templates.
    if let Some(spec) = slice_spec {
        let mut parts = spec.splitn(2, ':');
        // `take_while(digit)` above already proved the first part is numeric, so this parse
        // cannot fail; `unwrap_or(1)` is the documented default, not a live fallback.
        let start_one: usize = parts.next().unwrap_or("1").parse().unwrap_or(1);
        // 0 clamps to the first argument rather than wrapping, following bash.
        let start = start_one.saturating_sub(1);
        let joined = |v: Vec<String>| v.join(" ");
        return match parts.next().and_then(|l| l.parse::<usize>().ok()) {
            Some(len) => joined(args.iter().skip(start).take(len).cloned().collect()),
            None => joined(args.iter().skip(start).cloned().collect()),
        };
    }

    // `${@:-default}` and `${ARGUMENTS:-default}` — the default applies when the whole
    // argument list is empty.
    if let Some(default) = strip_default(inner) {
        let (target, fallback) = default;
        let value = if target == "@" || target == "ARGUMENTS" {
            all
        } else {
            match target.parse::<usize>().ok().and_then(|n| n.checked_sub(1)) {
                Some(i) => args.get(i).map(String::as_str).unwrap_or(""),
                None => return fallback.to_string(),
            }
        };
        // An empty string counts as missing, matching the shell's `:-` (as opposed to
        // `-` alone, which would not).
        return if value.is_empty() {
            fallback.to_string()
        } else {
            value.to_string()
        };
    }

    // `${@}` — the braced form of `$@`. Also not in pi's grammar (its regex requires a
    // colon inside the braces), and added here for the same reason as `${1}` below: someone
    // writing `${@}` meant all the arguments, and silently sending `${@}` to the model would
    // be a worse outcome than accepting a form pi does not.
    if inner == "@" || inner == "ARGUMENTS" {
        return all.to_string();
    }

    // `${1}` — braced but plain. Not in pi's grammar, but it is what people write, and
    // rejecting it would leave `${1}` in a prompt verbatim.
    if let Ok(n) = inner.parse::<usize>() {
        if let Some(v) = n.checked_sub(1).and_then(|i| args.get(i)) {
            return v.clone();
        }
        return String::new();
    }

    // Not a form we know. Emitting the braces keeps the damage visible instead of
    // silently deleting text the author wrote.
    format!("${{{inner}}}")
}

/// Split `TARGET:-DEFAULT`, returning the target and default.
fn strip_default(inner: &str) -> Option<(&str, &str)> {
    let idx = inner.find(":-")?;
    Some((&inner[..idx], &inner[idx + 2..]))
}

/// Expand `text` if it names a template, else return it unchanged.
///
/// This is the whole user-facing rule: a leading `/name` expands, anything else — including
/// a `/name` with no matching template — is passed through untouched. Returning the input
/// rather than an error is what lets an unknown slash command keep its existing "unknown
/// command, did you mean…?" path instead of a new failure mode.
pub fn expand_prompt_template(text: &str, templates: &[PromptTemplate]) -> Option<String> {
    let rest = text.strip_prefix('/')?;
    let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..name_end];
    let args_str = rest[name_end..].trim_start();

    let t = templates.iter().find(|t| t.name == name)?;
    let args = parse_command_args(args_str);
    Some(substitute_args(&t.content, &args))
}

/// Load every `.md` file in `dir` as a template, non-recursively.
///
/// A missing directory is not an error: most workspaces have no `prompts/` at all, and
/// making that a failure would mean every caller has to ask whether it exists first.
/// Symlinks never load (see `load_dir_scan`); rejections are silent here — this
/// signature predates the notices channel — and announced through
/// [`discover_templates`], which is the same scan with its warnings attached.
pub fn load_from_dir(dir: &Path, scope: TemplateScope) -> Vec<PromptTemplate> {
    let mut dropped = Vec::new();
    load_dir_scan(dir, scope, &mut dropped)
}

/// Scan one template dir, symlink-safe, reporting every skip into `notices`.
///
/// `load_from_dir` passes a throwaway sink (its signature is shared with
/// startup code this change does not touch); [`discover_templates`] is the
/// noticed presentation of the same scan. One scan, two presentations — never
/// two scanners that can disagree about what a directory holds.
fn load_dir_scan(
    dir: &Path,
    scope: TemplateScope,
    notices: &mut Vec<String>,
) -> Vec<PromptTemplate> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // A symlink is never loaded: a link answers to a path outside the dir
        // the trust gate approved, so following it would be an escape, not a
        // load. `symlink_metadata` tests the LINK itself — `metadata` would
        // follow it into whatever it points at, which is exactly the escape —
        // and every skip is announced: a file the operator wrote that never
        // loads must say so, or "my template does nothing" is unfixable.
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                notices.push(format!(
                    "prompts: ignoring {}: it is a symlink, which never loads",
                    path.display()
                ));
                continue;
            }
            Err(e) => {
                notices.push(format!("prompts: cannot read {}: {e}", path.display()));
                continue;
            }
            Ok(_) => {}
        }
        // `metadata` follows the link `symlink_metadata` just refused, so a
        // failure here means the target vanished mid-scan (or is unreadable) —
        // also announced, never silent.
        let Ok(meta) = std::fs::metadata(&path) else {
            notices.push(format!(
                "prompts: cannot read {}: unreadable or a link to nothing",
                path.display()
            ));
            continue;
        };
        if !meta.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Some(t) = load_from_file(&path, scope) {
            out.push(t);
        }
    }
    // Sorted so `/help` and the picker list templates in a stable order. Directory order is
    // filesystem-dependent, and a list that reorders between runs reads as a bug.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Load one template file, or `None` if it cannot be read or has no usable name.
pub fn load_from_file(path: &Path, scope: TemplateScope) -> Option<PromptTemplate> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (front, body) = parse_frontmatter(&raw);
    let name = path.file_stem()?.to_str()?.to_string();
    if name.is_empty() {
        return None;
    }

    let get = |key: &str| front.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());

    // A template with no description takes the first non-empty body line, truncated, so a
    // bare file is still usable in the picker. A template that is *only* frontmatter has no
    // description and is still listed — an empty one is better than an invisible template.
    let description = get("description")
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| {
            body.lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(|l| {
                    if l.chars().count() > DESC_LIMIT {
                        let head: String = l.chars().take(DESC_LIMIT).collect();
                        format!("{head}...")
                    } else {
                        l.to_string()
                    }
                })
                .unwrap_or_default()
        });

    Some(PromptTemplate {
        name,
        description,
        argument_hint: get("argument-hint").filter(|h| !h.is_empty()),
        content: body,
        scope,
        kind: PromptFileKind::Template,
        file_path: path.to_path_buf(),
    })
}

/// Load prompt templates from scope-tagged `prompts/` directories — the noticed
/// presentation of the [`load_from_dir`] scan.
///
/// Every `.md` file becomes a template EXCEPT a file whose stem is a builtin
/// head ([`crate::command::COMMAND_HEADS`]): the builtin always wins, in both
/// tiers, so a repository file can never hijack `/trust` (a `prompts/trust.md`
/// answering the trust prompt is the attack, not a feature — this diverges
/// from the old shadowing rule on purpose, exactly as `commands/` already
/// does). Dropped with a warning on the same [`DiscoveredFiles`] channel
/// [`discover_commands`] uses. Symlink rejections arrive with their notices
/// from the shared scan.
pub fn discover_templates(dirs: &[(PathBuf, TemplateScope)]) -> DiscoveredFiles {
    let mut files: Vec<PromptTemplate> = Vec::new();
    let mut notices: Vec<String> = Vec::new();
    for (dir, scope) in dirs {
        match std::fs::read_dir(dir) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                notices.push(format!("prompts: cannot read {}: {e}", dir.display()));
                continue;
            }
        }
        let mut loaded = load_dir_scan(dir, *scope, &mut notices);
        // The builtin guard, announced: a template answering to a builtin name
        // would run instead of it at submit time (expansion runs before the
        // command parse), so silence here is a hijack, not an override.
        loaded.retain(|t| {
            if crate::command::COMMAND_HEADS.contains(&t.name.as_str()) {
                notices.push(format!(
                    "prompts: ignoring {}: '/{}' is a builtin command, which always wins \
                     over a file of the same name",
                    t.file_path.display(),
                    t.name
                ));
                return false;
            }
            true
        });
        files.extend(loaded);
    }
    // Later directories win, exactly as [`merge`]: user first, project after.
    let mut out: Vec<PromptTemplate> = Vec::new();
    for f in files {
        match out.iter_mut().find(|existing| existing.name == f.name) {
            Some(slot) => *slot = f,
            None => out.push(f),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    DiscoveredFiles {
        files: out,
        notices,
    }
}

/// Parse `---` delimited frontmatter into flat key/value pairs, returning them with the body.
///
/// Deliberately the same flat parser `skills.rs` uses, and for the same reason: a full YAML
/// dependency for `description: …` is not worth it, and a template that needs nesting is a
/// template that should be a tool. Unterminated frontmatter yields no pairs and the whole
/// file as body, so a file that merely starts with a horizontal rule is not truncated.
fn parse_frontmatter(raw: &str) -> (Vec<(String, String)>, String) {
    if raw.lines().next().map(str::trim) != Some("---") {
        return (Vec::new(), raw.to_string());
    }
    let mut pairs = Vec::new();
    let mut consumed = 1usize;
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
    (Vec::new(), raw.to_string())
}

/// Merge template directories into one list, project scope winning on a name clash.
///
/// The order is the precedence order: user templates first, then project
/// templates, then explicit paths. A later entry with the same name replaces an
/// earlier one, so a repository can override a personal template without the user editing
/// their own config — and can also be overridden by an explicit `--prompt` on the command
/// line, which is the point of "explicit" being last.
///
/// The precedence is the skills rule (later root wins) and opencode's
/// project-over-global layering — not pi's: pi keeps the *first* registration
/// and reports the collision, so citing pi for project-wins is false.
pub fn merge(
    user: Vec<PromptTemplate>,
    project: Vec<PromptTemplate>,
    explicit: Vec<PromptTemplate>,
) -> Vec<PromptTemplate> {
    let mut out: Vec<PromptTemplate> = Vec::new();
    for t in user.into_iter().chain(project).chain(explicit) {
        match out.iter_mut().find(|existing| existing.name == t.name) {
            Some(slot) => *slot = t,
            None => out.push(t),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Custom commands: `commands/` directories with executable behaviour
//
// The `commands/` table lives here rather than beside the skill roots in
// `epsilon-core::skills`, and that placement is load-bearing rather than
// convenience. `skills.rs` owns the cross-harness Skills shape (SKILL.md
// bundles, `disable-model-invocation`, one-level scans of every harness's
// directory); commands are flat `.md` files whose frontmatter keys and
// substitution grammar are *this* module's. Splitting the flat-`.md` loading
// across two modules would invite the second grammar copy this file's header
// refuses to add. The table takes its bases as arguments because this crate
// has no internal dependencies and cannot read the home directory itself —
// the caller passes what only it knows (`epsilon-core::config::global_dir`
// in production), and the table owns the shape and the trust gate.
// ---------------------------------------------------------------------------------------------

/// Prompt-template directories in precedence order: the user's own first,
/// then the workspace pair **when trusted**.
///
/// Later entries win name clashes ([`merge`]), so project overrides user.
/// An absent directory is an ordinary entry: most workspaces have none, and
/// callers skip what does not exist without a word.
pub fn template_dirs(
    global: Option<&Path>,
    workspace: &Path,
    project_trusted: bool,
) -> Vec<(PathBuf, TemplateScope)> {
    let mut dirs = Vec::new();
    if let Some(base) = global {
        dirs.push((base.join("prompts"), TemplateScope::User));
    }
    if project_trusted {
        dirs.push((
            workspace.join(".agents").join("prompts"),
            TemplateScope::Project,
        ));
        dirs.push((
            workspace.join(".epsilon").join("prompts"),
            TemplateScope::Project,
        ));
    }
    dirs
}

/// Custom-command directories, mirroring [`template_dirs`] with the
/// `commands` leaf: the user's own first, then the workspace pair **when
/// trusted**. Two behaviours sharing one grammar — `prompts/` stays pure
/// text-expansion while `commands/` expands and resubmits — not two copies
/// of the loader.
///
/// Other harnesses' command trees (Claude Code's `.claude/commands`,
/// opencode's `.opencode/commands`) are deliberately NOT scanned: discovery
/// reads Epsilon's own dirs plus the portable `.agents` alias, exactly as
/// for templates. Wider interop is deferred, not overlooked.
pub fn command_dirs(
    global: Option<&Path>,
    workspace: &Path,
    project_trusted: bool,
) -> Vec<(PathBuf, TemplateScope)> {
    let mut dirs = Vec::new();
    if let Some(base) = global {
        dirs.push((base.join("commands"), TemplateScope::User));
    }
    if project_trusted {
        dirs.push((
            workspace.join(".agents").join("commands"),
            TemplateScope::Project,
        ));
        dirs.push((
            workspace.join(".epsilon").join("commands"),
            TemplateScope::Project,
        ));
    }
    dirs
}

/// What file discovery found, and what it would not load.
///
/// Shared by [`discover_commands`] and [`discover_templates`]: one channel for
/// both tiers and both directories, so a dropped file is announced the same
/// way wherever it was found.
///
/// The notices exist for the same reason the skills channel has them: a file
/// the operator wrote that never loads must say so, or "my command does
/// nothing" is unfixable. An absent directory is NOT a notice — most
/// workspaces have no `commands/` at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFiles {
    /// The commands that passed every gate, sorted by name.
    pub files: Vec<PromptTemplate>,
    /// One line per skipped file, in discovery order.
    pub notices: Vec<String>,
}

/// Load custom commands from scope-tagged `commands/` directories.
///
/// Every `.md` file whose stem passes [`valid_name`] becomes a command with
/// [`PromptFileKind::Command`]; the frontmatter keeps template keys
/// (`description`, `argument-hint`) and no new ones. Skipped WITH a notice:
/// an invalid stem, an unreadable file, and — always — a name colliding with
/// a builtin head ([`crate::command::COMMAND_HEADS`]): the builtin is kept,
/// so a repository file can never hijack `/trust`, `/login`, `/preset` or
/// `/plan`. This diverges from opencode on purpose (opencode lets a
/// same-named custom command override the builtin); a repo file answering
/// the trust prompt is the attack, not a feature.
pub fn discover_commands(dirs: &[(PathBuf, TemplateScope)]) -> DiscoveredFiles {
    let mut files: Vec<PromptTemplate> = Vec::new();
    let mut notices: Vec<String> = Vec::new();
    for (dir, scope) in dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                notices.push(format!("commands: cannot read {}: {e}", dir.display()));
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Symlinks never load here either (see `load_dir_scan` for why):
            // the link itself is tested via `symlink_metadata`, and a file
            // that cannot even be statted is announced rather than swallowed —
            // a broken link used to vanish silently, which is exactly what
            // makes "my command does nothing" unfixable.
            match std::fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    notices.push(format!(
                        "commands: ignoring {}: it is a symlink, which never loads",
                        path.display()
                    ));
                    continue;
                }
                Err(e) => {
                    notices.push(format!("commands: cannot read {}: {e}", path.display()));
                    continue;
                }
                Ok(_) => {}
            }
            // `metadata` follows the link `symlink_metadata` just refused, so
            // a failure here means the target vanished mid-scan (or is
            // unreadable) — also announced, never silent.
            let Ok(meta) = std::fs::metadata(&path) else {
                notices.push(format!(
                    "commands: cannot read {}: unreadable or a link to nothing",
                    path.display()
                ));
                continue;
            };
            if !meta.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let raw = match std::fs::read_to_string(&path) {
                Ok(raw) => raw,
                Err(e) => {
                    notices.push(format!("commands: cannot read {}: {e}", path.display()));
                    continue;
                }
            };
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if !valid_name(&stem) {
                notices.push(format!(
                    "commands: ignoring {}: '{stem}' is not a valid command name (lowercase a-z, \
                     0-9 and hyphens, no leading, trailing or doubled hyphen, at most \
                     {MAX_NAME} characters)",
                    path.display()
                ));
                continue;
            }
            if crate::command::COMMAND_HEADS.contains(&stem.as_str()) {
                notices.push(format!(
                    "commands: ignoring {}: '/{stem}' is a builtin command, which always wins \
                     over a file of the same name",
                    path.display()
                ));
                continue;
            }
            let (front, body) = parse_frontmatter(&raw);
            let get = |key: &str| front.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
            let description = get("description")
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| {
                    body.lines()
                        .map(str::trim)
                        .find(|l| !l.is_empty())
                        .map(|l| {
                            if l.chars().count() > DESC_LIMIT {
                                let head: String = l.chars().take(DESC_LIMIT).collect();
                                format!("{head}...")
                            } else {
                                l.to_string()
                            }
                        })
                        .unwrap_or_default()
                });
            files.push(PromptTemplate {
                name: stem,
                description,
                argument_hint: get("argument-hint").filter(|h| !h.is_empty()),
                content: body,
                scope: *scope,
                kind: PromptFileKind::Command,
                file_path: path,
            });
        }
    }
    // Later directories win, exactly as [`merge`]: user first, project after.
    let mut out: Vec<PromptTemplate> = Vec::new();
    for f in files {
        match out.iter_mut().find(|existing| existing.name == f.name) {
            Some(slot) => *slot = f,
            None => out.push(f),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    DiscoveredFiles {
        files: out,
        notices,
    }
}

/// Merge loaded templates with loaded commands into the one list the TUI caches.
///
/// A command displaces a same-named template — announced, because two files
/// share one `/name` and a listing that hid one of them would lie about what
/// the name does. The notice names the winner so the template's author can
/// find the file that took its name.
///
/// A builtin-named TEMPLATE never reaches the cache either, whatever tier it
/// came from: the TUI pipeline scans templates with the silent
/// [`load_from_dir`], so the announced drop on that path fires here, where
/// the notices already reach the transcript (startup and `/reload` forward
/// them; RPC ignores them exactly like the command ones).
/// [`discover_templates`] drops the same files with the same sentence for
/// direct API users — one sentence, two gates, and each path fires exactly
/// once because each path scans once.
pub fn merge_kinds(
    templates: Vec<PromptTemplate>,
    commands: Vec<PromptTemplate>,
) -> (Vec<PromptTemplate>, Vec<String>) {
    let mut notices = Vec::new();
    let mut out = Vec::new();
    for t in templates {
        if crate::command::COMMAND_HEADS.contains(&t.name.as_str()) {
            notices.push(format!(
                "prompts: ignoring {}: '/{}' is a builtin command, which always wins \
                 over a file of the same name",
                t.file_path.display(),
                t.name
            ));
            continue;
        }
        out.push(t);
    }
    for c in commands {
        if let Some(slot) = out.iter_mut().find(|t| t.name == c.name) {
            notices.push(format!(
                "commands: '/{}' from {} shadows a prompt template of the same name; \
                 the command wins",
                c.name,
                c.file_path.display()
            ));
            *slot = c;
        } else {
            out.push(c);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (out, notices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // --- argument splitting -------------------------------------------------

    #[test]
    fn plain_words_split_on_whitespace() {
        assert_eq!(
            parse_command_args("one two  three"),
            a(&["one", "two", "three"])
        );
    }

    /// Quoted runs are one argument. Without this, `/commit "fix the parser"` sends three
    /// arguments and the template's `$1` is `fix` — a silent truncation of what the user
    /// asked for.
    #[test]
    fn quoted_runs_are_one_argument() {
        assert_eq!(
            parse_command_args("\"fix the parser\""),
            a(&["fix the parser"])
        );
        assert_eq!(parse_command_args("'single quoted'"), a(&["single quoted"]));
    }

    /// A quote can open mid-argument.
    #[test]
    fn a_quote_opens_mid_argument() {
        assert_eq!(
            parse_command_args(r#"say "hello world" now"#),
            a(&["say", "hello world", "now"])
        );
    }

    #[test]
    fn empty_and_whitespace_only_input_yields_no_arguments() {
        assert!(parse_command_args("").is_empty());
        assert!(parse_command_args("    ").is_empty());
    }

    /// An unterminated quote takes the rest of the line rather than dropping it.
    #[test]
    fn an_unterminated_quote_takes_the_remainder() {
        assert_eq!(
            parse_command_args(r#"open "never closed"#),
            a(&["open", "never closed"])
        );
    }

    // --- bare placeholders --------------------------------------------------

    #[test]
    fn positional_arguments_expand() {
        assert_eq!(
            substitute_args("review $1 and $2", &a(&["a", "b"])),
            "review a and b"
        );
    }

    #[test]
    fn an_out_of_range_positional_expands_to_nothing() {
        assert_eq!(substitute_args("[$1][$9]", &a(&["only"])), "[only][]");
    }

    #[test]
    fn all_arguments_expand_via_at_and_arguments() {
        assert_eq!(substitute_args("$@", &a(&["a", "b"])), "a b");
        assert_eq!(substitute_args("$ARGUMENTS", &a(&["a", "b"])), "a b");
    }

    /// `ARGUMENTS` must not be read as the argument named `A`.
    #[test]
    fn arguments_is_read_as_whole_not_as_a_letter() {
        assert_eq!(substitute_args("$ARGUMENTS", &a(&["x"])), "x");
    }

    // --- defaults -----------------------------------------------------------

    #[test]
    fn a_default_applies_when_the_argument_is_missing() {
        assert_eq!(substitute_args("${1:-none}", &a(&[])), "none");
    }

    /// `:-` means "if unset **or empty**", as in the shell. A template author writing
    /// `${1:-all}` expects the default when they pass an empty string, which is easy to do
    /// with a trailing space in a TUI input box.
    #[test]
    fn a_default_applies_when_the_argument_is_empty() {
        assert_eq!(substitute_args("${1:-all}", &a(&[""])), "all");
    }

    #[test]
    fn a_default_is_not_used_when_the_argument_is_present() {
        assert_eq!(substitute_args("${1:-none}", &a(&["given"])), "given");
    }

    #[test]
    fn the_all_arguments_default_applies_only_when_no_arguments() {
        assert_eq!(substitute_args("${@:-nothing}", &a(&[])), "nothing");
        assert_eq!(
            substitute_args("${@:-nothing}", &a(&["something"])),
            "something"
        );
        assert_eq!(substitute_args("${ARGUMENTS:-nothing}", &a(&[])), "nothing");
    }

    // --- slicing ------------------------------------------------------------

    #[test]
    fn a_slice_takes_from_n_to_the_end() {
        assert_eq!(substitute_args("${@:2}", &a(&["a", "b", "c"])), "b c");
    }

    #[test]
    fn a_slice_can_be_bounded() {
        assert_eq!(
            substitute_args("${@:2:2}", &a(&["a", "b", "c", "d"])),
            "b c"
        );
    }

    /// bash treats start 0 as 1. Following bash rather than Rust indexing is the point:
    /// a template that works in someone's shell must work here.
    #[test]
    fn a_zero_start_clamps_to_the_first_argument() {
        assert_eq!(substitute_args("${@:0}", &a(&["a", "b"])), "a b");
        assert_eq!(substitute_args("${@:0:1}", &a(&["a", "b"])), "a");
    }

    #[test]
    fn a_slice_past_the_end_is_empty_rather_than_an_error() {
        assert_eq!(substitute_args("${@:9}", &a(&["a"])), "");
        assert_eq!(substitute_args("${@:9:2}", &a(&["a"])), "");
    }

    // --- escaping and unknowns ---------------------------------------------

    /// A `$` that starts nothing recognisable is literal -- but a bare digit IS
    /// recognisable, and expands to nothing when that argument was not supplied.
    ///
    /// pi's grammar is `\$(ARGUMENTS|@|\d+)`, so pi drops `$5` here too. The fidelity case
    /// is therefore only the trailing `$`, which is what a price or a unit would contain.
    #[test]
    fn a_lone_dollar_is_literal() {
        assert_eq!(
            substitute_args("costs $5 and $ per GB", &a(&[])),
            "costs  and $ per GB"
        );
        assert_eq!(substitute_args("about $ per GB", &a(&[])), "about $ per GB");
    }

    /// A `$$` is how a template author writes a literal dollar.
    #[test]
    fn a_doubled_dollar_yields_one_dollar() {
        assert_eq!(substitute_args("$$HOME", &a(&[])), "$HOME");
    }

    /// An unrecognised braced form keeps its text. Deleting it would be worse: the author
    /// would see their prompt come back mangled with no way to tell why.
    #[test]
    fn an_unknown_braced_form_is_preserved() {
        assert_eq!(substitute_args("${weird}", &a(&[])), "${weird}");
    }

    /// Substitution is single-pass. An argument containing `$1` is inserted literally.
    ///
    /// This is pi's rule and it is a safety property, not an oversight: a recursive pass
    /// would let a pasted shell transcript inject another template's placeholders.
    #[test]
    fn substitution_is_not_recursive() {
        assert_eq!(
            substitute_args("say $1", &a(&["$2 and $1"])),
            "say $2 and $1"
        );
    }

    /// `${1}` is not in pi's grammar, but it is what people type, and leaving it literal
    /// would send the user a prompt full of braces.
    #[test]
    fn a_braced_positional_expands() {
        assert_eq!(
            substitute_args("${1} then ${2}", &a(&["a", "b"])),
            "a then b"
        );
    }

    // --- the `@:` collision -------------------------------------------------
    //
    // `${@:2}` (a slice) and `${@:-default}` (a default) both begin `@:`, and the
    // implementation got this wrong three times by reordering branches: defaults-first made
    // the slice unreadable, slices-first made the default unreadable, and a "non-digit bail"
    // made the default return its own text. The form is decided once, from the character
    // after the colon. These tests exist together, not separately, because testing either
    // one alone is what let the bug survive twice.

    #[test]
    fn both_at_colon_forms_work_in_the_same_run() {
        let args = a(&["a", "b", "c"]);
        assert_eq!(substitute_args("${@:2}", &args), "b c", "the slice form");
        assert_eq!(
            substitute_args("${@:-none}", &args),
            "a b c",
            "the default form, args present"
        );
        assert_eq!(
            substitute_args("${@:-none}", &a(&[])),
            "none",
            "the default form, no args"
        );
    }

    /// A slice whose start is 1 is the whole list, and a bounded slice of 1 is the first.
    #[test]
    fn slice_edges_are_correct() {
        let args = a(&["a", "b", "c"]);
        assert_eq!(substitute_args("${@:1}", &args), "a b c");
        assert_eq!(substitute_args("${@:1:1}", &args), "a");
        assert_eq!(substitute_args("${@:3}", &args), "c");
    }

    /// A bare `${@}` is all arguments -- no colon, so it is not a malformed slice.
    #[test]
    fn a_bare_braced_at_is_all_arguments() {
        assert_eq!(substitute_args("${@}", &a(&["a", "b"])), "a b");
    }

    /// A slice length past the end truncates rather than erroring.
    #[test]
    fn an_oversized_slice_length_truncates() {
        assert_eq!(substitute_args("${@:1:99}", &a(&["a", "b"])), "a b");
    }

    /// `$1:2` is not a slice. Only `@` slices, and treating a positional as one would accept
    /// a malformed template silently.
    #[test]
    fn a_positional_cannot_slice() {
        assert_eq!(substitute_args("${1:2}", &a(&["a", "b", "c"])), "${1:2}");
    }

    // --- expansion ----------------------------------------------------------

    fn tpl(name: &str, content: &str) -> PromptTemplate {
        PromptTemplate {
            name: name.to_string(),
            description: String::new(),
            argument_hint: None,
            content: content.to_string(),
            scope: TemplateScope::User,
            kind: PromptFileKind::Template,
            file_path: PathBuf::from(format!("/tmp/{name}.md")),
        }
    }

    #[test]
    fn a_slash_name_with_arguments_expands_the_template() {
        let t = vec![tpl("review", "Review $1 carefully.")];
        assert_eq!(
            expand_prompt_template("/review src/parser.rs", &t).as_deref(),
            Some("Review src/parser.rs carefully.")
        );
    }

    #[test]
    fn a_bare_slash_name_expands_with_no_arguments() {
        let t = vec![tpl("review", "Review the diff.")];
        assert_eq!(
            expand_prompt_template("/review", &t).as_deref(),
            Some("Review the diff.")
        );
    }

    /// An unknown name is left alone so the caller keeps its existing "unknown command" path.
    /// Returning an error here would replace a helpful suggestion with a new failure mode.
    #[test]
    fn an_unknown_name_expands_to_nothing() {
        let t = vec![tpl("review", "x")];
        assert_eq!(expand_prompt_template("/bogus arg", &t), None);
    }

    #[test]
    fn text_without_a_leading_slash_is_not_a_template() {
        let t = vec![tpl("review", "x")];
        assert_eq!(expand_prompt_template("please /review this", &t), None);
    }

    /// Leading whitespace before the `/` means it is prose, not a command.
    #[test]
    fn leading_whitespace_disables_expansion() {
        let t = vec![tpl("review", "x")];
        assert_eq!(expand_prompt_template("  /review", &t), None);
    }

    // --- loading ------------------------------------------------------------

    #[test]
    fn frontmatter_becomes_metadata_and_the_body_stays_content() {
        let dir = std::env::temp_dir().join(format!("eps-pt-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("review.md");
        std::fs::write(
            &path,
            "---\ndescription: Review carefully\nargument-hint: <path>\n---\nBody line one.\nBody two.\n",
        )
        .unwrap();
        let t = load_from_file(&path, TemplateScope::Project).expect("should load");
        assert_eq!(t.name, "review");
        assert_eq!(t.description, "Review carefully");
        assert_eq!(t.argument_hint.as_deref(), Some("<path>"));
        // Lines are rejoined with `\n`, so a trailing newline in the file does not survive.
        // Normalised rather than byte-preserved on purpose: what gets sent to the model is a
        // prompt, and a trailing blank line is not worth a byte-exact parse.
        assert_eq!(t.content, "Body line one.\nBody two.");
        assert_eq!(t.scope, TemplateScope::Project);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file with no frontmatter still works, taking its description from the first line.
    #[test]
    fn a_bare_markdown_file_is_still_a_template() {
        let dir = std::env::temp_dir().join(format!("eps-pt-bare-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("quick.md");
        std::fs::write(&path, "Just do the thing.\nMore detail.\n").unwrap();
        let t = load_from_file(&path, TemplateScope::User).expect("should load");
        assert_eq!(t.name, "quick");
        assert_eq!(t.description, "Just do the thing.");
        assert_eq!(t.content, "Just do the thing.\nMore detail.\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unterminated frontmatter must not truncate the file. A markdown file that opens with
    /// a horizontal rule is common, and swallowing the rest of it would send a blank prompt.
    #[test]
    fn unterminated_frontmatter_keeps_the_whole_file() {
        let dir = std::env::temp_dir().join(format!("eps-pt-un-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hr.md");
        std::fs::write(&path, "---\n\nSome content after a rule.\n").unwrap();
        let t = load_from_file(&path, TemplateScope::User).expect("should load");
        assert!(t.content.contains("Some content after a rule."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A missing directory is not an error: most workspaces have no `prompts/`.
    #[test]
    fn a_missing_directory_yields_no_templates() {
        let missing = std::env::temp_dir().join("epsilon-prompts-does-not-exist-xyz");
        assert!(load_from_dir(&missing, TemplateScope::User).is_empty());
    }

    /// Loading is sorted, because directory order is filesystem-dependent and a list that
    /// reorders between runs reads as a bug to the user.
    #[test]
    fn loading_is_sorted_by_name() {
        let dir = std::env::temp_dir().join(format!("eps-pt-sort-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for n in ["zeta", "alpha", "mid"] {
            std::fs::write(dir.join(format!("{n}.md")), "body").unwrap();
        }
        // A non-markdown file must be ignored, not loaded as a template named "notes".
        std::fs::write(dir.join("notes.txt"), "ignore me").unwrap();
        let names: Vec<String> = load_from_dir(&dir, TemplateScope::User)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["alpha", "mid", "zeta"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- precedence ---------------------------------------------------------

    /// Project beats user on a name clash, so a repository can ship its own `review`.
    #[test]
    fn project_templates_override_user_templates() {
        let merged = merge(
            vec![tpl("review", "user version")],
            vec![tpl("review", "project version")],
            vec![],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].content, "project version");
    }

    /// Explicit beats project, which is the point of an explicit path on the command line.
    #[test]
    fn explicit_templates_override_everything() {
        let merged = merge(
            vec![tpl("review", "user")],
            vec![tpl("review", "project")],
            vec![tpl("review", "explicit")],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].content, "explicit");
    }

    /// Overriding replaces rather than duplicates, or a single `/review` would be ambiguous.
    #[test]
    fn distinct_names_accumulate() {
        let merged = merge(
            vec![tpl("a", "1")],
            vec![tpl("b", "2")],
            vec![tpl("c", "3")],
        );
        assert_eq!(merged.len(), 3);
    }

    // --- custom commands (commands/): executable files sharing the grammar ------
    //
    // RED-first: every test below names API that does not exist yet
    // (`PromptFileKind`, `valid_name`, `TemplateScope::short`, `template_dirs`,
    // `command_dirs`, `discover_commands`, `merge_kinds`). The first run must
    // fail to compile on exactly those names; the implementation that follows
    // makes each one pass for the reason it names.

    /// A file stem becomes a command name only when it obeys the same rules as
    /// a skill name: lowercase `a-z`, `0-9` and hyphens, no leading, trailing
    /// or doubled hyphen, at most 64 characters.
    #[test]
    fn command_names_reject_uppercase_symbols_and_bad_hyphens() {
        for bad in [
            "Bad_Name",
            "UPPER",
            "has space",
            "dot.name",
            "-leading",
            "trailing-",
            "double--hyphen",
            "",
        ] {
            assert!(!valid_name(bad), "'{bad}' must not be a valid command name");
        }
    }

    /// The length cap is the skills cap (64): a name longer than that is
    /// rejected rather than truncated, so a typo cannot silently rename.
    #[test]
    fn command_names_reject_overlong_stems() {
        let long = "a".repeat(65);
        assert!(!valid_name(&long), "65 characters must be rejected");
        assert!(valid_name(&"a".repeat(64)), "64 characters is the cap");
    }

    /// The control: ordinary stems are accepted.
    #[test]
    fn command_names_accept_valid_stems() {
        for good in ["deploy", "review-2", "a", "0-day", "x-y-z-9"] {
            assert!(valid_name(good), "'{good}' must be a valid command name");
        }
    }

    /// Scope tags for the popup are single letters: user `[u]`, project `[p]`.
    #[test]
    fn scope_short_tags_are_single_letters() {
        assert_eq!(TemplateScope::User.short(), "u");
        assert_eq!(TemplateScope::Project.short(), "p");
    }

    /// A `trust.md` command file must never hijack `/trust`: the builtin is
    /// kept and the file is dropped with a warning naming it.
    #[test]
    fn a_command_colliding_with_a_builtin_is_dropped_with_a_warning() {
        let base = std::env::temp_dir().join(format!("eps-cmd-collide-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("trust.md"), "You are now trusted.\n").unwrap();
        std::fs::write(user.join("deploy.md"), "Ship it.\n").unwrap();

        let found = discover_commands(&[(user.clone(), TemplateScope::User)]);
        let names: Vec<&str> = found.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["deploy"],
            "the colliding file must not load: {names:?}"
        );
        assert!(
            found
                .notices
                .iter()
                .any(|n| n.contains("trust") && n.contains("built")),
            "dropping a file the operator wrote must be announced: {:?}",
            found.notices
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Project beats user for commands, exactly as for templates: a
    /// repository ships its own `deploy` without the operator editing theirs.
    /// The precedence stands on the skills divergence (later root wins) and
    /// opencode's project-over-global layering — not on pi, which keeps the
    /// first registration.
    #[test]
    fn project_commands_override_user_commands() {
        let base = std::env::temp_dir().join(format!("eps-cmd-shadow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        let project = base.join("ws").join(".epsilon").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(user.join("deploy.md"), "user deploy\n").unwrap();
        std::fs::write(project.join("deploy.md"), "project deploy\n").unwrap();

        let found = discover_commands(&[
            (user, TemplateScope::User),
            (project, TemplateScope::Project),
        ]);
        assert_eq!(found.files.len(), 1, "{:?}", found.files);
        assert_eq!(found.files[0].content.trim(), "project deploy");
        assert_eq!(found.files[0].scope, TemplateScope::Project);
        assert!(
            found.notices.is_empty(),
            "a designed override is not a problem to report: {:?}",
            found.notices
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Loaded command files carry the command kind, so listings can tell an
    /// executable command from a text-expansion template sharing one grammar.
    #[test]
    fn loaded_commands_carry_the_command_kind() {
        let base = std::env::temp_dir().join(format!("eps-cmd-kind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("ship.md"), "Ship $1.\n").unwrap();

        let found = discover_commands(&[(user, TemplateScope::User)]);
        assert_eq!(found.files.len(), 1);
        assert_eq!(found.files[0].kind, PromptFileKind::Command);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// An invalid stem is skipped WITH a notice: silence is what makes "my
    /// command does nothing" unfixable.
    #[test]
    fn an_invalid_command_stem_is_skipped_with_a_notice() {
        let base = std::env::temp_dir().join(format!("eps-cmd-badname-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("Bad_Name.md"), "body\n").unwrap();

        let found = discover_commands(&[(user, TemplateScope::User)]);
        assert!(
            found.files.is_empty(),
            "nothing loads from an invalid stem: {:?}",
            found.files
        );
        assert_eq!(found.notices.len(), 1, "{:?}", found.notices);
        assert!(
            found.notices[0].contains("Bad_Name"),
            "the notice must name the file: {:?}",
            found.notices
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// An unreadable command file is skipped with a notice. Invalid UTF-8 is
    /// the portable way to make `read_to_string` refuse a file that exists.
    #[test]
    fn an_unreadable_command_file_is_skipped_with_a_notice() {
        let base = std::env::temp_dir().join(format!("eps-cmd-unread-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("broken.md"), [0xff, 0xfe, 0x00]).unwrap();

        let found = discover_commands(&[(user, TemplateScope::User)]);
        assert!(found.files.is_empty(), "{:?}", found.files);
        assert_eq!(found.notices.len(), 1, "{:?}", found.notices);
        assert!(
            found.notices[0].contains("broken"),
            "the notice must name the file: {:?}",
            found.notices
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A missing commands directory is the common case, not an error: most
    /// workspaces have none, and a notice per absent dir would train readers
    /// to ignore the channel.
    #[test]
    fn missing_command_dirs_yield_nothing_and_say_nothing() {
        let missing = std::env::temp_dir().join("epsilon-commands-does-not-exist-xyz");
        let found = discover_commands(&[(missing, TemplateScope::Project)]);
        assert!(found.files.is_empty());
        assert!(
            found.notices.is_empty(),
            "an absent dir is not a problem: {:?}",
            found.notices
        );
    }

    /// Unterminated frontmatter keeps the whole file, exactly as for
    /// templates: a file opening with a horizontal rule is common, and
    /// swallowing it would send a blank command.
    #[test]
    fn unterminated_frontmatter_keeps_the_whole_command_file() {
        let base = std::env::temp_dir().join(format!("eps-cmd-hr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("hr.md"), "---\n\nSome content after a rule.\n").unwrap();

        let found = discover_commands(&[(user, TemplateScope::User)]);
        assert_eq!(found.files.len(), 1, "{:?}", found.notices);
        assert!(
            found.files[0]
                .content
                .contains("Some content after a rule.")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Only `description` and `argument-hint` are read: no new frontmatter
    /// keys, so a command file cannot smuggle scoping or execution options.
    #[test]
    fn command_frontmatter_reads_no_new_keys() {
        let base = std::env::temp_dir().join(format!("eps-cmd-fm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(
            user.join("scoped.md"),
            "---\ndescription: Do it\nscope: project\nshell: rm -rf /\n---\nBody.\n",
        )
        .unwrap();

        let found = discover_commands(&[(user, TemplateScope::User)]);
        assert_eq!(found.files.len(), 1, "{:?}", found.notices);
        assert_eq!(found.files[0].description, "Do it");
        assert!(
            !found.files[0].content.contains("scope:"),
            "unknown keys stay in frontmatter, never in behaviour: {:?}",
            found.files[0].content
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The args grammar is ONE grammar: a command body expands exactly as a
    /// template body does, through the same `substitute_args`.
    #[test]
    fn command_args_expand_exactly_like_template_args() {
        let base = std::env::temp_dir().join(format!("eps-cmd-parity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("ship.md"), "Ship $1 ${2:-soon}.\n").unwrap();
        let found = discover_commands(&[(user, TemplateScope::User)]);
        let command_expanded = expand_prompt_template("/ship auth", &found.files)
            .expect("a loaded command expands by name");
        // The same body as a template expands the same way: one grammar, two
        // directories, not two copies of the substitution rules.
        let template = tpl("ship-t", "Ship $1 ${2:-soon}.");
        let template_expanded = expand_prompt_template("/ship-t auth", &[template])
            .expect("the template control expands");
        assert_eq!(command_expanded.trim(), template_expanded.trim());
        assert_eq!(command_expanded.trim(), "Ship auth soon.");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Headless stays literal: with nothing discovered (headless performs no
    /// discovery) a `/name` is returned untouched — there is no execution
    /// outside the TUI.
    #[test]
    fn slash_text_without_discovery_stays_literal() {
        let empty: Vec<PromptTemplate> = Vec::new();
        assert_eq!(expand_prompt_template("/deploy latest", &empty), None);
        assert_eq!(expand_prompt_template("/help", &empty), None);
    }

    /// The dir tables gate the project tier on trust: user dirs always list,
    /// project dirs list only when trusted. Callers pass the SAME
    /// project_trusted bit composition decided — the table owns the shape.
    #[test]
    fn command_and_template_dirs_gate_the_project_tier_on_trust() {
        let ws = PathBuf::from("/ws");
        let global = PathBuf::from("/home/user/.config/epsilon");

        let untrusted_cmds = command_dirs(Some(global.as_path()), &ws, false);
        assert!(
            untrusted_cmds.iter().all(|(p, _)| !p.starts_with(&ws)),
            "nothing under an untrusted workspace may be scanned: {untrusted_cmds:?}"
        );
        assert!(
            untrusted_cmds
                .iter()
                .any(|(p, s)| p.ends_with("commands") && *s == TemplateScope::User),
            "the user tier survives distrust: {untrusted_cmds:?}"
        );
        let trusted_cmds = command_dirs(Some(global.as_path()), &ws, true);
        assert!(
            trusted_cmds.iter().any(|(p, _)| p.starts_with(&ws)),
            "trusting the workspace unlocks its commands: {trusted_cmds:?}"
        );

        let untrusted_tpl = template_dirs(Some(global.as_path()), &ws, false);
        assert!(
            untrusted_tpl.iter().all(|(p, _)| !p.starts_with(&ws)),
            "templates gate identically: {untrusted_tpl:?}"
        );
        let trusted_tpl = template_dirs(Some(global.as_path()), &ws, true);
        assert!(
            trusted_tpl
                .iter()
                .any(|(p, _)| p == &ws.join(".agents").join("prompts")),
            "the portable alias is in the table: {trusted_tpl:?}"
        );
        assert!(
            trusted_cmds
                .iter()
                .any(|(p, _)| p == &ws.join(".agents").join("commands")),
            "commands mirror prompts: {trusted_cmds:?}"
        );
    }

    /// A command wins over a same-named template, and the displacement is
    /// announced: two files, one `/name`, and a listing that hid one of them
    /// would lie about what `/name` does.
    #[test]
    fn a_command_displaces_a_same_named_template_with_a_notice() {
        let t = PromptTemplate {
            name: "deploy".to_string(),
            description: "old text".to_string(),
            argument_hint: None,
            content: "old".to_string(),
            scope: TemplateScope::User,
            file_path: PathBuf::from("/home/u/prompts/deploy.md"),
            kind: PromptFileKind::Template,
        };
        let c = PromptTemplate {
            name: "deploy".to_string(),
            description: "new cmd".to_string(),
            argument_hint: None,
            content: "new".to_string(),
            scope: TemplateScope::Project,
            file_path: PathBuf::from("/ws/.epsilon/commands/deploy.md"),
            kind: PromptFileKind::Command,
        };
        let (merged, notices) = merge_kinds(vec![t], vec![c]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].kind, PromptFileKind::Command);
        assert_eq!(merged[0].content, "new");
        assert!(
            notices.iter().any(|n| n.contains("deploy")),
            "the displacement must be announced: {notices:?}"
        );
    }

    /// Drift-style: every `.md` file under the scanned dirs either loads or
    /// is announced. Nothing is silently dropped.
    #[test]
    fn every_command_file_resolves_or_is_announced() {
        let base = std::env::temp_dir().join(format!("eps-cmd-drift-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        for name in ["alpha", "mid", "zeta"] {
            std::fs::write(user.join(format!("{name}.md")), "body\n").unwrap();
        }
        std::fs::write(user.join("Bad.md"), "body\n").unwrap();
        std::fs::write(user.join("notes.txt"), "ignore me\n").unwrap();

        let found = discover_commands(&[(user.clone(), TemplateScope::User)]);
        // Three valid files load; the invalid stem is announced; the .txt is
        // ordinary tree and stays silent, exactly as for templates.
        assert_eq!(
            found.files.len(),
            3,
            "{:?} / {:?}",
            found.files,
            found.notices
        );
        for f in &found.files {
            assert!(
                expand_prompt_template(&format!("/{}", f.name), &found.files).is_some(),
                "every listed command resolves by name: {}",
                f.name
            );
        }
        assert!(
            found.notices.iter().any(|n| n.contains("Bad")),
            "the invalid stem is announced: {:?}",
            found.notices
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // --- prompts/ builtin-shadow (Fix 1) + symlink rejection (Fix 3) -----------
    //
    // RED-first: `discover_templates` does not exist yet — the first run must
    // fail to compile on exactly that name (the established RED pattern for a
    // new API in this module). `load_from_dir` cannot carry the warning: it
    // returns files only, and the channel this must match is `DiscoveredFiles`
    // (files + notices) — the same one `discover_commands` uses.

    /// A `trust.md` template must never hijack `/trust` — user tier or
    /// project tier, a builtin always runs the builtin. Dropped at discovery
    /// with a warning, exactly as `discover_commands` drops a colliding command.
    #[test]
    fn a_template_colliding_with_a_builtin_is_dropped_with_a_warning() {
        let base = std::env::temp_dir().join(format!("eps-pt-collide-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("prompts");
        let project = base.join("ws").join(".epsilon").join("prompts");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(user.join("trust.md"), "MALICIOUS-USER-BODY.\n").unwrap();
        std::fs::write(project.join("trust.md"), "MALICIOUS-PROJECT-BODY.\n").unwrap();
        std::fs::write(project.join("review.md"), "Review $1.\n").unwrap();

        let found = discover_templates(&[
            (user.clone(), TemplateScope::User),
            (project.clone(), TemplateScope::Project),
        ]);
        let names: Vec<&str> = found.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["review"],
            "neither colliding file may load: {names:?}"
        );
        assert_eq!(
            found.notices.len(),
            2,
            "one warning per dropped file: {:?}",
            found.notices
        );
        for n in &found.notices {
            assert!(
                n.contains("trust") && n.contains("builtin"),
                "the warning names the file and the reason: {n}"
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The merged cache never carries a builtin-named template either: the TUI
    /// pipeline scans with the silent `load_from_dir`, so the announced drop
    /// on that path fires here, where the notices already reach the UI — and
    /// the RPC listing, which ignores them exactly like the command ones.
    #[test]
    fn merge_kinds_drops_a_builtin_named_template_with_a_notice() {
        let mk = |scope| PromptTemplate {
            name: "trust".to_string(),
            description: "shadow".to_string(),
            argument_hint: None,
            content: "MALICIOUS-BODY.".to_string(),
            scope,
            kind: PromptFileKind::Template,
            file_path: PathBuf::from("trust.md"),
        };
        for scope in [TemplateScope::User, TemplateScope::Project] {
            let (merged, notices) = merge_kinds(vec![mk(scope)], vec![]);
            assert!(
                merged.is_empty(),
                "a builtin-named template never survives the merge ({scope:?})"
            );
            assert!(
                notices
                    .iter()
                    .any(|n| n.contains("trust") && n.contains("builtin")),
                "the drop is announced ({scope:?}): {notices:?}"
            );
        }
    }

    /// Best-effort symlink creation: Windows demands a privilege this runner
    /// may not hold — then the test says it skips instead of failing on the OS.
    #[cfg(unix)]
    fn try_symlink(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    /// Best-effort symlink creation (see the unix twin): without the
    /// privilege the test skips loudly rather than asserting nothing.
    #[cfg(windows)]
    fn try_symlink(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }

    /// A symlink inside a `commands/` dir never loads — and says so, naming
    /// the link. A link answers to a path outside the dir the trust gate
    /// approved, so following it would be an escape, not a load.
    #[test]
    fn a_symlinked_command_is_rejected_with_a_notice_naming_the_link() {
        let base = std::env::temp_dir().join(format!("eps-cmd-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("real.md"), "Real body.\n").unwrap();
        if !try_symlink(&user.join("real.md"), &user.join("evil.md")) {
            eprintln!("SKIP: this runner may not create symlinks");
            let _ = std::fs::remove_dir_all(&base);
            return;
        }

        let found = discover_commands(&[(user.clone(), TemplateScope::User)]);
        assert!(
            found.files.iter().all(|f| f.name != "evil"),
            "a link never loads: {:?}",
            found.files
        );
        assert!(
            found
                .notices
                .iter()
                .any(|n| n.contains("evil") && n.contains("symlink")),
            "the rejection names the link: {:?}",
            found.notices
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A broken symlink is rejected with a notice too, not silence: today the
    /// commands/ scan swallows it, and "my command does nothing" is unfixable.
    #[test]
    fn a_broken_command_symlink_is_rejected_with_a_notice_not_silence() {
        let base = std::env::temp_dir().join(format!("eps-cmd-blink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("commands");
        std::fs::create_dir_all(&user).unwrap();
        if !try_symlink(&user.join("missing-target.md"), &user.join("ghost.md")) {
            eprintln!("SKIP: this runner may not create symlinks");
            let _ = std::fs::remove_dir_all(&base);
            return;
        }

        let found = discover_commands(&[(user.clone(), TemplateScope::User)]);
        assert!(
            found.files.iter().all(|f| f.name != "ghost"),
            "a broken link never loads: {:?}",
            found.files
        );
        assert!(
            found.notices.iter().any(|n| n.contains("ghost")),
            "the rejection names the link instead of staying silent: {:?}",
            found.notices
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The same gate on the prompts/ side, through the noticed API.
    #[test]
    fn a_symlinked_template_is_rejected_with_a_notice_naming_the_link() {
        let base = std::env::temp_dir().join(format!("eps-pt-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("prompts");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("real.md"), "Real body.\n").unwrap();
        if !try_symlink(&user.join("real.md"), &user.join("evil.md")) {
            eprintln!("SKIP: this runner may not create symlinks");
            let _ = std::fs::remove_dir_all(&base);
            return;
        }

        let found = discover_templates(&[(user.clone(), TemplateScope::User)]);
        assert!(
            found.files.iter().all(|f| f.name != "evil"),
            "a link never loads: {:?}",
            found.files
        );
        assert!(
            found
                .notices
                .iter()
                .any(|n| n.contains("evil") && n.contains("symlink")),
            "the rejection names the link: {:?}",
            found.notices
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A broken template link is announced too: silence is what makes "my
    /// template does nothing" unfixable, on this tier like the other.
    #[test]
    fn a_broken_template_symlink_is_rejected_with_a_notice_not_silence() {
        let base = std::env::temp_dir().join(format!("eps-pt-blink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user").join("prompts");
        std::fs::create_dir_all(&user).unwrap();
        if !try_symlink(&user.join("missing-target.md"), &user.join("ghost.md")) {
            eprintln!("SKIP: this runner may not create symlinks");
            let _ = std::fs::remove_dir_all(&base);
            return;
        }

        let found = discover_templates(&[(user.clone(), TemplateScope::User)]);
        assert!(
            found.files.iter().all(|f| f.name != "ghost"),
            "a broken link never loads: {:?}",
            found.files
        );
        assert!(
            found.notices.iter().any(|n| n.contains("ghost")),
            "the rejection names the link instead of staying silent: {:?}",
            found.notices
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
