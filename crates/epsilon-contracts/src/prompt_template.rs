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

/// One loaded prompt template.
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
    /// `/review` will actually use. Project scope wins (pi's ordering), so a repository can
    /// ship its own `review` without the user touching their global config.
    pub scope: TemplateScope,
    /// The file this came from.
    pub file_path: PathBuf,
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
pub fn load_from_dir(dir: &Path, scope: TemplateScope) -> Vec<PromptTemplate> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // A symlink to a file is a file. `read_dir`'s file type is the link itself, so a
        // symlinked template directory entry is reported as neither file nor dir.
        let Ok(meta) = std::fs::metadata(&path) else {
            continue; // broken symlink
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
        file_path: path.to_path_buf(),
    })
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
/// The order is the precedence order, and it is also the order pi uses: user templates first,
/// then project templates, then explicit paths. A later entry with the same name replaces an
/// earlier one, so a repository can override a personal template without the user editing
/// their own config — and can also be overridden by an explicit `--prompt` on the command
/// line, which is the point of "explicit" being last.
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
}
