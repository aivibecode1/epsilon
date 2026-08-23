//! Search and listing tools: grep, glob, ls.

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

/// Arguments for grep.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrepArgs {
    /// Rust-regex pattern.
    pub pattern: String,
    /// Directory or file to search (default: workspace root).
    #[serde(default)]
    pub path: Option<String>,
    /// Case-insensitive match.
    #[serde(default)]
    pub ignore_case: bool,
}

pub const GREP_DESC: &str = include_str!("../descriptions/grep.txt");

const MAX_MATCH_LINES: usize = 200;

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

        let re = RegexBuilder::new(&args.pattern)
            .case_insensitive(args.ignore_case)
            .build()
            .map_err(|e| ToolFailure::msg(format!("bad regex: {e}")))?;

        let mut hits: Vec<String> = Vec::new();
        let mut files_scanned = 0usize;
        let mut truncated = false;
        'outer: for rel in util::walk_files(&root) {
            let abs = root.join(&rel);
            if abs.is_dir() {
                continue;
            }
            let Ok(text) = util::read_text_capped(&abs) else {
                continue;
            };
            files_scanned += 1;
            let rel_norm = rel.to_string_lossy().replace('\\', "/");
            for (i, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    hits.push(format!("{rel_norm}:{}: {}", i + 1, line.trim_end()));
                    if hits.len() >= MAX_MATCH_LINES {
                        truncated = true;
                        break 'outer;
                    }
                }
            }
        }

        if hits.is_empty() {
            return Ok(ToolOutput::text("no matches"));
        }
        let mut out = hits.join("\n");
        if truncated {
            out.push_str(&format!("\n[stopped at {MAX_MATCH_LINES} matches]"));
        }
        out.push_str(&format!("\n[{files_scanned} files scanned]"));
        Ok(ToolOutput::text(out))
    }
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
fn glob_to_regex(glob: &str) -> String {
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

#[async_trait]
impl Tool for GlobTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
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
        let matches: Vec<String> = util::walk_files(&root)
            .iter()
            .map(|p| norm(p))
            .filter(|rel| re.is_match(rel))
            .take(util::WALK_MAX_ENTRIES)
            .collect();

        if matches.is_empty() {
            return Ok(ToolOutput::text("no matches"));
        }
        Ok(ToolOutput::text(format!(
            "{}\n[{} file(s)]",
            matches.join("\n"),
            matches.len()
        )))
    }
}

// ---- ls -------------------------------------------------------------------------

/// Arguments for ls.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LsArgs {
    /// Directory to list (default: workspace root).
    #[serde(default)]
    pub path: Option<String>,
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

        for e in entries {
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
        Ok(ToolOutput::text(rows.join("\n")))
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
