//! Search and listing tools: grep, glob, ls, find.

use std::path::Path;

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use regex::RegexBuilder;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::util::{self, escaped_workspace};

// ---- grep -----------------------------------------------------------------------

/// Secret-class file names (plan 004): grep never prints their lines. Mirrors the
/// read-gate defaults (`*.env` asks, `*.env.example` allowed) so the two layers agree.
fn is_secret_class_file(rel: &Path) -> bool {
    let name = rel
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name == ".env.example" || name.ends_with(".env.example") {
        return false;
    }
    // `.env`, `.env.local`, `prod.env`, and common credential file names.
    let env_shaped = name.split('.').any(|seg| seg == "env")
        && (name.starts_with(".env") || name.ends_with(".env") || name.contains(".env."));
    let cred_named = matches!(
        name.as_str(),
        "credentials" | "credentials.json" | ".npmrc" | ".netrc" | "secrets.yaml" | "secrets.yml"
    ) || name.contains("secret");
    env_shaped || cred_named
}

/// Arguments for grep.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrepArgs {
    /// Regex pattern — or a literal string when `literal` is set (pi parity).
    pub pattern: String,
    /// Directory or file to search (default: workspace root).
    #[serde(default)]
    pub path: Option<String>,
    /// Case-insensitive match.
    #[serde(default)]
    pub ignore_case: bool,
    /// Treat the pattern as a literal string instead of a regex
    /// (pi `literal` → ripgrep `--fixed-strings`).
    #[serde(default)]
    pub literal: bool,
    /// Restrict searched files to a glob such as `*.rs` or `**/*.spec.ts`
    /// (pi `glob` → ripgrep `--glob`). Bare names match at any depth.
    #[serde(default)]
    pub glob: Option<String>,
    /// Lines of context before/after each match, `path-line- text` for
    /// context rows (pi `context`: `path:line:` for matches,
    /// `path-line- ` for context).
    #[serde(default)]
    pub context: Option<usize>,
    /// Maximum matches returned (default 100, pi `DEFAULT_LIMIT`).
    #[serde(default)]
    pub limit: Option<usize>,
}

pub const GREP_DESC: &str = include_str!("../descriptions/grep.txt");

/// Default match cap (pi `DEFAULT_LIMIT`).
pub const GREP_DEFAULT_LIMIT: usize = 100;
/// Max chars per rendered match line (pi `GREP_MAX_LINE_LENGTH`).
pub const GREP_MAX_LINE_LENGTH: usize = 500;
/// Default entry cap for `ls` (pi `ls.ts:23` `DEFAULT_LIMIT`).
pub const LS_DEFAULT_LIMIT: usize = 500;

/// Regex line search over the workspace walker (git/target/node_modules skipped).
pub struct GrepTool {
    descriptor: ToolDescriptor,
}

impl GrepTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "grep".into(),
                label: "Grep".into(),
                description: GREP_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(GrepArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for GrepTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]

impl Tool for GrepTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn resource_key(
        &self,
        arguments: &serde_json::Value,
        base: &std::path::Path,
    ) -> Option<std::path::PathBuf> {
        search_key(arguments, base)
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: GrepArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'grep': {e}")))?;
        let root = match &args.path {
            Some(p) => {
                let (r, inside) = util::resolve_in_workspace(&ctx.cwd, p);
                if !inside {
                    return Err(escaped_workspace(p));
                }
                r
            }
            None => ctx.cwd.clone(),
        };

        let re = if args.literal {
            // pi `--fixed-strings`: the pattern is data, never syntax.
            RegexBuilder::new(&regex::escape(&args.pattern))
                .case_insensitive(args.ignore_case)
                .build()
        } else {
            RegexBuilder::new(&args.pattern)
                .case_insensitive(args.ignore_case)
                .build()
        }
        .map_err(|e| ToolFailure::msg(format!("bad regex: {e}")))?;

        // pi `glob` filter: reuse the glob tool's dialect (bare names match at
        // any depth), compiled once per call.
        let glob_re = match &args.glob {
            Some(g) => {
                let effective = if g.contains('/') || g.contains('\\') {
                    g.clone()
                } else {
                    format!("**/{g}")
                };
                Some(
                    RegexBuilder::new(&glob_to_regex(&effective))
                        .build()
                        .map_err(|e| ToolFailure::msg(format!("bad glob: {e}")))?,
                )
            }
            None => None,
        };

        let max_matches = args.limit.unwrap_or(GREP_DEFAULT_LIMIT).max(1);
        let context = args.context.unwrap_or(0);

        // One entry per match or context row, in file order.
        let mut rows: Vec<(String, usize, bool, String)> = Vec::new();
        let mut files_scanned = 0usize;
        let mut limit_reached = false;
        let mut lines_truncated = false;
        let mut skipped_secrets: Vec<String> = Vec::new();
        // Files longer than the byte cap were searched only in their first MAX_READ_BYTES
        // bytes. That is a real miss and it used to be silent (M95): the cap stays — an
        // unbounded read per walked file trades a silent miss for an unbounded scan — but the
        // result now says which files were only searched in part.
        let mut capped_files: Vec<String> = Vec::new();
        'outer: for rel in util::walk_files(&root) {
            let abs = root.join(&rel);
            if abs.is_dir() {
                continue;
            }
            let rel_norm = rel.to_string_lossy().replace('\\', "/");
            if let Some(g) = &glob_re
                && !g.is_match(&rel_norm)
            {
                continue;
            }
            // Plan 004 (SECURITY-03): never print lines from secret-class files.
            // The read-gate protects `read .env`, but grep would otherwise print the
            // same secrets line-by-line into the transcript ungated. `.env.example`
            // is public by design and stays searchable.
            if is_secret_class_file(&rel) {
                skipped_secrets.push(rel_norm);
                continue;
            }
            let Ok((text, capped)) = util::read_text_capped_ex(&abs) else {
                continue;
            };
            if capped {
                capped_files.push(rel_norm.clone());
            }
            files_scanned += 1;
            // CRLF-normalized once per file so `context` windows and line text
            // match pi's re-read path (`\r\n`/`\r` → `\n`).
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if re.is_match(line) {
                    let line_no = i + 1;
                    let match_text = truncate_grep_line(line);
                    if match_text.1 {
                        lines_truncated = true;
                    }
                    rows.push((rel_norm.clone(), line_no, true, match_text.0));
                    if rows.iter().filter(|r| r.2).count() >= max_matches {
                        limit_reached = true;
                        break 'outer;
                    }
                    // Context rows join the same stream (excluded from the cap).
                    if context > 0 {
                        let lo = line_no.saturating_sub(context).max(1);
                        let hi = (line_no + context).min(lines.len());
                        // Before-rows (ascending), skipping the match itself.
                        for c in lo..line_no {
                            let (t, trunc) = truncate_grep_line(lines[c - 1]);
                            lines_truncated |= trunc;
                            rows.push((rel_norm.clone(), c, false, t));
                        }
                        for c in line_no + 1..=hi {
                            let (t, trunc) = truncate_grep_line(lines[c - 1]);
                            lines_truncated |= trunc;
                            rows.push((rel_norm.clone(), c, false, t));
                        }
                    }
                }
            }
        }

        // Honesty notes that have no file row of their own: skipped secret-class files, and
        // files only partly searched. Both are reported even when nothing matched — an
        // incomplete search is exactly the state in which a bare "no matches" would be a lie.
        let mut notes: Vec<String> = Vec::new();
        if !skipped_secrets.is_empty() {
            notes.push(format!(
                "[grep] skipped {} secret-class file(s): {}",
                skipped_secrets.len(),
                skipped_secrets.join(", ")
            ));
        }
        if !capped_files.is_empty() {
            notes.push(format!(
                "[grep] searched only the first {} bytes of {} file(s) — matches past that \
                 byte are not reported: {}",
                util::MAX_READ_BYTES,
                capped_files.len(),
                capped_files.join(", ")
            ));
        }

        if rows.iter().all(|r| r.0.is_empty()) {
            let mut text = String::from("no matches");
            for note in &notes {
                text.push('\n');
                text.push_str(note);
            }
            return Ok(ToolOutput::text(text));
        }
        let mut out_lines: Vec<String> = Vec::with_capacity(rows.len() + notes.len() + 3);
        for (rel, no, is_match, text) in &rows {
            if rel.is_empty() {
                out_lines.push(text.clone());
            } else if *is_match {
                out_lines.push(format!("{rel}:{no}: {text}"));
            } else {
                out_lines.push(format!("{rel}-{no}- {text}"));
            }
        }
        out_lines.extend(notes);
        let mut notices: Vec<String> = Vec::new();
        if limit_reached {
            notices.push(format!(
                "{max_matches} matches limit reached. Use limit={} for more, or refine pattern",
                max_matches * 2
            ));
        }
        if lines_truncated {
            notices.push(format!(
                "Some lines truncated to {GREP_MAX_LINE_LENGTH} chars. Use read tool to see full lines"
            ));
        }
        if !notices.is_empty() {
            out_lines.push(String::new());
            out_lines.push(format!("[{}]", notices.join(". ")));
        }
        out_lines.push(format!("[{files_scanned} files scanned]"));
        let mut details = serde_json::json!({
            "truncated": limit_reached,
            "lines_truncated": lines_truncated,
        });
        if !capped_files.is_empty() {
            details["files_capped"] = serde_json::json!(capped_files);
        }
        if limit_reached {
            details["match_limit_reached"] = serde_json::json!(max_matches);
        }
        Ok(ToolOutput {
            content: vec![epsilon_contracts::content::ContentBlock::text(
                out_lines.join("\n"),
            )],
            details,
            usage: None,
            terminate: false,
        })
    }
}

/// Truncate one rendered grep line (pi `truncateLine`: 500 chars + suffix).
pub fn truncate_grep_line(line: &str) -> (String, bool) {
    // Grapheme-safe cut: char (not byte) boundary, so multi-byte text never splits.
    let len = line.chars().count();
    if len <= GREP_MAX_LINE_LENGTH {
        return (line.to_string(), false);
    }
    let head: String = line.chars().take(GREP_MAX_LINE_LENGTH).collect();
    (format!("{head}... [truncated]"), true)
}

// ---- glob -----------------------------------------------------------------------

/// Arguments for glob.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GlobArgs {
    /// Glob such as `src/*.rs` or `README*`. `*` stays within one path segment;
    /// `**` crosses directories. A pattern without `/` matches at any depth.
    pub pattern: String,
    /// Directory to search from (default: workspace root).
    #[serde(default)]
    pub path: Option<String>,
}

pub const GLOB_DESC: &str = include_str!("../descriptions/glob.txt");

/// Filename pattern search over the same walker as grep.
pub struct GlobTool {
    descriptor: ToolDescriptor,
}

impl GlobTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "glob".into(),
                label: "Glob".into(),
                description: GLOB_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(GlobArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for GlobTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Translate our glob dialect to a regex string.
/// `**/` allows zero or more directories; `**` spans separators; `*`/`?` do not.
pub(crate) fn glob_to_regex(glob: &str) -> String {
    let mut re = String::from("^");
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                i += 2;
                // collapse extra stars
                while i < chars.len() && chars[i] == '*' {
                    i += 1;
                }
                if i < chars.len() && (chars[i] == '/' || chars[i] == '\\') {
                    re.push_str("(?:.*/)?"); // `**/` = zero or more dirs
                    i += 1;
                } else {
                    re.push_str(".*");
                }
            }
            '*' => {
                re.push_str("[^/\\\\]*");
                i += 1;
            }
            '?' => {
                re.push_str("[^/\\\\]");
                i += 1;
            }
            c => {
                re.push_str(&regex::escape(&c.to_string()));
                i += 1;
            }
        }
    }
    re.push('$');
    re
}

/// Planner key for the search tools (M72): the directory being searched, defaulting to the
/// workspace root. A directory key conflicts with anything *inside* it — the planner compares
/// keys by prefix, so a `grep` over `src/` will not run beside a `write` to `src/lib.rs`.
pub(crate) fn search_key(
    arguments: &serde_json::Value,
    base: &std::path::Path,
) -> Option<std::path::PathBuf> {
    match arguments.get("path").and_then(|v| v.as_str()) {
        Some(raw) => Some(util::resolve_in_workspace(base, raw).0),
        None => Some(base.to_path_buf()),
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn resource_key(
        &self,
        arguments: &serde_json::Value,
        base: &std::path::Path,
    ) -> Option<std::path::PathBuf> {
        search_key(arguments, base)
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: GlobArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'glob': {e}")))?;
        let root = match &args.path {
            Some(p) => {
                let (r, inside) = util::resolve_in_workspace(&ctx.cwd, p);
                if !inside {
                    return Err(escaped_workspace(p));
                }
                r
            }
            None => ctx.cwd.clone(),
        };

        // Bare filename patterns match at any depth (common expectation).
        let effective = if args.pattern.contains('/') || args.pattern.contains('\\') {
            args.pattern.clone()
        } else {
            format!("**/{}", args.pattern)
        };
        let re = RegexBuilder::new(&glob_to_regex(&effective))
            .build()
            .map_err(|e| ToolFailure::msg(format!("pattern error: {e}")))?;

        let norm = |p: &Path| p.to_string_lossy().replace('\\', "/");
        // The walker itself stops at `WALK_MAX_ENTRIES`, so these matches are drawn from at
        // most that many walked files; the `.take` is the belt to that braces and is what the
        // notice below tests against. This is a safety valve on the walk, not a result cap:
        // a reader who hit it did not ask for too much, the directory was too large to walk.
        let matches: Vec<String> = util::walk_files(&root)
            .iter()
            .map(|p| norm(p))
            .filter(|rel| re.is_match(rel))
            .take(util::WALK_MAX_ENTRIES)
            .collect();

        if matches.is_empty() {
            return Ok(ToolOutput::text("no matches"));
        }
        let walk_bound_hit = matches.len() >= util::WALK_MAX_ENTRIES;
        // pi has no `glob` tool, so there is no upstream shape to copy; the cap is `find`'s, and so
        // is the reason: a match list is a context-window payload, and 5000 paths reach the model
        // as ~150 KB where `find` stops at 50 KB (M118's flagged asymmetry). Applied the same way —
        // head-truncated whole lines at `DEFAULT_MAX_BYTES`, exactly as `find.ts:277` /
        // `ls.ts:139-153` do it through pi's `truncateHead`.
        let (listing, shown) = util::truncate_head_bytes(&matches, util::TOOL_OUTPUT_MAX_BYTES);
        let bytes_cut = shown < matches.len();
        // The count is the rows the reader can see, for the reason M118 gave `find`: a total that
        // includes results it cannot see is the same lie one line up. Unchanged when nothing was
        // cut, because then the two are the same number.
        let mut text = format!("{listing}\n[{shown} file(s)]");
        // One notice, the bound that actually fired — never both. Which one is not arbitrary:
        // when the walk stopped early, the byte notice's own claims ("N more file(s) match", "use
        // a narrower pattern to see the rest") are **false**, because the files it would be
        // talking about were never visited and no pattern at this root can reach them. The walk
        // notice's sentence is true in both cases, so it takes precedence; the byte cap is still
        // applied (the list is cut), and the count above still reports what is shown.
        if walk_bound_hit {
            // M117's rule, applied to the walker's own cap (G21b): a bound that changed the answer
            // says so. Reaching the cap means the walk stopped mid-directory, so files that were
            // never visited cannot appear however the pattern is refined — only a narrower search
            // root brings them back. The wording says that rather than naming an internal constant,
            // because the reader's next action is "search a subdirectory", not "raise 5000".
            text.push_str(&format!(
                "\n\n[{} files walked, so this list may be incomplete: the walk is a safety \
                 bound, not a match limit, and stopping it hid whatever it never visited. \
                 Search a subdirectory to see the rest]",
                util::WALK_MAX_ENTRIES
            ));
        } else if bytes_cut {
            // The bound that fired here is the byte budget, so the notice names it and what it
            // hid; the same sentence `find` prints, because it is the same bound.
            text.push_str(&format!(
                "\n\n[{} output limit reached: {} more file(s) match and are not shown. \
                 Use a narrower pattern to see the rest]",
                util::format_size(util::TOOL_OUTPUT_MAX_BYTES),
                matches.len() - shown
            ));
        }
        Ok(ToolOutput::text(text))
    }
}

// ---- ls -------------------------------------------------------------------------

/// Arguments for ls.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LsArgs {
    /// Directory to list (default: workspace root).
    #[serde(default)]
    pub path: Option<String>,
    /// Maximum number of entries to return (default: 500, pi `DEFAULT_LIMIT`). When the
    /// limit is reached the result says how many entries were held back — and names the byte
    /// budget instead when that is what cut the listing, because raising this one would not
    /// then show more.
    #[serde(default)]
    pub limit: Option<usize>,
}

pub const LS_DESC: &str = include_str!("../descriptions/ls.txt");

/// Shallow directory listing with sizes — orientation tool, not a walker.
pub struct LsTool {
    descriptor: ToolDescriptor,
}

impl LsTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "ls".into(),
                label: "List".into(),
                description: LS_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(LsArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for LsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for LsTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn resource_key(
        &self,
        arguments: &serde_json::Value,
        base: &std::path::Path,
    ) -> Option<std::path::PathBuf> {
        search_key(arguments, base)
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: LsArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'ls': {e}")))?;
        let root = match &args.path {
            Some(p) => {
                let (r, inside) = util::resolve_in_workspace(&ctx.cwd, p);
                if !inside {
                    return Err(escaped_workspace(p));
                }
                r
            }
            None => ctx.cwd.clone(),
        };

        let mut rows: Vec<String> = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(&root)
            .map_err(|e| ToolFailure::msg(format!("cannot list {}: {e}", root.display())))?
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(|e| e.file_name());

        // pi `ls.ts:84,114-118`: the caller's `limit` wins, otherwise `DEFAULT_LIMIT` entries
        // are returned and the drop is announced rather than left to look like the whole
        // directory.
        let limit = args.limit.unwrap_or(LS_DEFAULT_LIMIT).max(1);
        let total = entries.len();
        let mut limit_reached = false;
        for e in entries {
            if rows.len() >= limit {
                limit_reached = true;
                break;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let meta = e.metadata().ok();
            match meta {
                Some(m) if m.is_dir() => rows.push(format!("{name}/")),
                Some(m) => rows.push(format!("{name} ({} bytes)", m.len())),
                None => rows.push(name),
            }
        }
        if rows.is_empty() {
            return Ok(ToolOutput::text("(empty directory)"));
        }
        // pi `ls.ts:139-153`: after the entry cap, the joined listing goes through
        // `truncateHead` at `DEFAULT_MAX_BYTES`; the entry cap is the line cap, so the byte
        // budget is the only other bound. It is a real one — names and sizes are the payload,
        // and 500 of them are not bounded by any file size.
        let (listing, shown) = util::truncate_head_bytes(&rows, util::TOOL_OUTPUT_MAX_BYTES);
        let bytes_cut = shown < rows.len();
        let mut text = listing;
        // One bound is named, the one that actually stopped the listing: telling a reader the
        // entry cap was reached, when bytes cut inside it, would send them to raise a limit
        // that is not the binding one (`ls.ts:146-151` names both; this names the true one).
        if bytes_cut {
            text.push_str(&format!(
                "\n\n[{} output limit reached: {} of the {} entries in this directory are \
                 shown, {} more are not. List a subdirectory to see them]",
                util::format_size(util::TOOL_OUTPUT_MAX_BYTES),
                shown,
                total,
                total - shown
            ));
        } else if limit_reached {
            // pi says "N entries limit reached. Use limit=2N for more" (`ls.ts:146-148`); the
            // count of what is missing is added because "limit reached" alone does not tell a
            // reader what they are not seeing.
            text.push_str(&format!(
                "\n\n[{limit} entries limit reached: {} more entries in this directory are not \
                 shown. Use limit={} for more]",
                total - limit,
                limit * 2
            ));
        }
        Ok(ToolOutput::text(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_translation_covers_dialect() {
        let re = |p: &str| RegexBuilder::new(&glob_to_regex(p)).build().unwrap();
        // zero directories allowed by `**/`
        assert!(re("**/*.rs").is_match("lib.rs"));
        assert!(re("**/*.rs").is_match("src/deep/lib.rs"));
        assert!(!re("**/*.rs").is_match("src/lib.txt"));
        // `*` stays within a segment
        assert!(re("src/*.rs").is_match("src/lib.rs"));
        assert!(!re("src/*.rs").is_match("src/deep/lib.rs"));
        assert!(re("a?c").is_match("abc"));
    }
}
