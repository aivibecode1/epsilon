//! File-finding tool: `find` (pi parity for `allToolNames: find`).
//!
//! Mirrors `packages/coding-agent/src/core/tools/find.ts` shape — glob → walk_files
//! with a `.gitignore`-respects-at-v1 approximation (skip dirs already honoured
//! by `walk_files`) and the standard truncation semantics.

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use regex::RegexBuilder;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::util::{self, escaped_workspace};

/// Arguments for `find`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindArgs {
    /// Glob pattern such as `*.rs`, `**/*.json`, or `src/**/*.spec.ts`.
    pub pattern: String,
    /// Directory to search from (default: workspace root).
    #[serde(default)]
    pub path: Option<String>,
    /// Maximum number of results (default: 1000, capped at 1000).
    #[serde(default)]
    pub limit: Option<usize>,
}

pub const FIND_DESC: &str = include_str!("../descriptions/find.txt");

/// Find files by glob pattern (pi parity).
pub struct FindTool {
    descriptor: ToolDescriptor,
}

impl FindTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "find".into(),
                label: "Find".into(),
                description: FIND_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(FindArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for FindTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FindTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: FindArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'find': {e}")))?;
        if args.pattern.trim().is_empty() {
            return Err(ToolFailure::msg("pattern must not be empty"));
        }
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
        let effective = if args.pattern.contains('/') || args.pattern.contains('\\') {
            args.pattern.clone()
        } else {
            format!("**/{}", args.pattern)
        };
        let re = RegexBuilder::new(&crate::search::glob_to_regex(&effective))
            .build()
            .map_err(|e| ToolFailure::msg(format!("pattern error: {e}")))?;
        let limit = args.limit.unwrap_or(1000).min(1000);
        let norm = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/");
        let matches: Vec<String> = util::walk_files(&root)
            .iter()
            .map(|p| norm(p))
            .filter(|rel| re.is_match(rel))
            .take(limit)
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

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::tool::{ToolContext, ToolInvocation};
    use std::fs;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    async fn run_find(pattern: &str, root: &std::path::Path, limit: Option<usize>) -> String {
        let tool = FindTool::new();
        let mut args = serde_json::json!({
            "pattern": pattern,
        });
        if let Some(l) = limit {
            args["limit"] = serde_json::json!(l);
        }
        let call = ToolInvocation {
            tool_call_id: "find-test".into(),
            args,
        };
        let ctx = ToolContext {
            session_id: "test".into(),
            observation_scope: std::sync::Arc::new(()),
            cwd: root.to_path_buf(),
            spill_dir: root.join(".epsilon/spill"),
            token: CancellationToken::new(),
            task_host: None,
        };
        let out = tool
            .execute(call, ctx, Arc::new(|_| {}))
            .await
            .expect("find must succeed");
        out.content
            .iter()
            .filter_map(|c| match c {
                epsilon_contracts::content::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn bare_pattern_matches_at_any_depth() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/c.rs"), "").unwrap();
        fs::write(root.join("c.rs"), "").unwrap();
        let out = run_find("*.rs", &root, None).await;
        assert!(out.contains("c.rs"), "bare *.rs must match at depth");
        assert!(out.contains("a/b/c.rs"), "bare *.rs must match nested");
    }

    #[tokio::test]
    async fn respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for i in 0..5 {
            fs::write(root.join(format!("f{i}.txt")), "").unwrap();
        }
        let out = run_find("*.txt", &root, Some(2)).await;
        let lines: Vec<&str> = out.lines().filter(|l| !l.starts_with('[')).collect();
        assert_eq!(lines.len(), 2, "limit 2 must return 2 files");
    }
}
