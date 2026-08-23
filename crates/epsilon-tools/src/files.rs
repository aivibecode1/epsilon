//! File-manipulation tools: read, write, edit.

use std::path::Path;

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::util::{self, escaped_workspace};

// ---- read -----------------------------------------------------------------------

/// Arguments for the read tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadArgs {
    /// File path (absolute or workspace-relative).
    pub path: String,
    /// 1-based first line to return (default 1).
    #[serde(default)]
    pub offset: Option<usize>,
    /// Max lines to return (default: cap).
    #[serde(default)]
    pub limit: Option<usize>,
}

pub const READ_DESC: &str = include_str!("../descriptions/read.txt");

/// Numbered-line file reader with size caps and spill locators.
pub struct ReadTool {
    descriptor: ToolDescriptor,
}

impl ReadTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "read".into(),
                label: "Read".into(),
                description: READ_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(ReadArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for ReadTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: ReadArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'read': {e}")))?;
        let (path, inside) = util::resolve_in_workspace(&ctx.cwd, &args.path);
        if !inside {
            return Err(escaped_workspace(&args.path));
        }
        let content = util::read_text_capped(&path)?;

        let all_lines: Vec<&str> = content.lines().collect();
        let start = args.offset.unwrap_or(1).max(1);
        let limit = args
            .limit
            .unwrap_or(util::MAX_READ_LINES)
            .min(util::MAX_READ_LINES);
        let slice: Vec<&str> = all_lines
            .iter()
            .skip(start.saturating_sub(1))
            .take(limit)
            .copied()
            .collect();

        let mut body = slice
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>5}| {}", start + i, line))
            .collect::<Vec<_>>()
            .join("\n");
        let total = all_lines.len();
        if total > util::MAX_READ_LINES && slice.len() == limit {
            body.push_str(&format!(
                "\n[file has {total} lines; showing {start}-{}; pass offset to continue]",
                start + slice.len() - 1
            ));
        }
        if body.is_empty() {
            body = "(empty or past end of file)".to_string();
        }
        Ok(ToolOutput::text(body))
    }
}

// ---- write ----------------------------------------------------------------------

/// Arguments for the write tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteArgs {
    /// File path; parent directories are created when missing.
    pub path: String,
    /// Full new content of the file.
    pub content: String,
}

pub const WRITE_DESC: &str = include_str!("../descriptions/write.txt");

/// Create-or-overwrite writer. Overwrites are intentional (`edit` exists for diffs).
pub struct WriteTool {
    descriptor: ToolDescriptor,
}

impl WriteTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "write".into(),
                label: "Write".into(),
                description: WRITE_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(WriteArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Sequential,
            },
        }
    }
}

impl Default for WriteTool {
    fn default() -> Self {
        Self::new()
    }
}

fn ensure_inside(ctx: &ToolContext, raw: &str) -> Result<std::path::PathBuf, ToolFailure> {
    let (path, inside) = util::resolve_in_workspace(&ctx.cwd, raw);
    if !inside {
        return Err(escaped_workspace(raw));
    }
    Ok(path)
}

#[async_trait]
impl Tool for WriteTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: WriteArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'write': {e}")))?;
        let path = ensure_inside(&ctx, &args.path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ToolFailure::msg(format!("cannot create parent dir: {e}")))?;
        }
        let existed = path.exists();
        std::fs::write(&path, &args.content)
            .map_err(|e| ToolFailure::msg(format!("write failed: {e}")))?;
        Ok(ToolOutput::text(format!(
            "{} {} ({} bytes)",
            if existed { "overwrote" } else { "created" },
            args.path,
            args.content.len()
        )))
    }
}

// ---- edit -----------------------------------------------------------------------

/// Arguments for the edit tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditArgs {
    /// File to edit.
    pub path: String,
    /// Exact text to find (must match exactly once unless replace_all).
    pub old_string: String,
    /// Replacement text.
    pub new_string: String,
    /// Replace every occurrence instead of failing on multiples.
    #[serde(default)]
    pub replace_all: bool,
}

pub const EDIT_DESC: &str = include_str!("../descriptions/edit.txt");

/// Exact-string replacement editor with occurrence-count safety (pi/opencode semantics).
pub struct EditTool {
    descriptor: ToolDescriptor,
}

impl EditTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "edit".into(),
                label: "Edit".into(),
                description: EDIT_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(EditArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Sequential,
            },
        }
    }
}

impl Default for EditTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for EditTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: EditArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'edit': {e}")))?;
        if args.old_string.is_empty() {
            return Err(ToolFailure::msg("old_string must not be empty"));
        }
        let path = ensure_inside(&ctx, &args.path)?;
        let content = util::read_text_capped(&path)?;

        let occurrences = content.matches(&args.old_string).count();
        match occurrences {
            0 => {
                // Show a nearby hint so the model can self-correct.
                let first_line = args.old_string.lines().next().unwrap_or("");
                let hint = content
                    .lines()
                    .find(|l| !first_line.is_empty() && l.contains(first_line.trim()))
                    .unwrap_or("");
                Err(ToolFailure::msg(format!(
                    "old_string not found in {}.{}",
                    args.path,
                    if hint.is_empty() {
                        String::new()
                    } else {
                        format!(" Closest matching line: {hint}")
                    }
                )))
            }
            n if n > 1 && !args.replace_all => Err(ToolFailure::msg(format!(
                "old_string matches {n} locations in {}; widen the context or set replace_all",
                args.path
            ))),
            _ => {
                let updated = if args.replace_all {
                    content.replace(&args.old_string, &args.new_string)
                } else {
                    content.replacen(&args.old_string, &args.new_string, 1)
                };
                std::fs::write(&path, &updated)
                    .map_err(|e| ToolFailure::msg(format!("write failed: {e}")))?;
                let count = if args.replace_all { occurrences } else { 1 };
                Ok(ToolOutput::text(format!(
                    "edited {} ({count} replacement(s))",
                    args.path
                )))
            }
        }
    }
}

/// Silence unused-import lint until Path gains a direct use in later milestones.
#[allow(dead_code)]
fn _path_witness(_: &Path) {}
