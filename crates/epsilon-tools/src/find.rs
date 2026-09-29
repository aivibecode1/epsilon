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

use crate::util;

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

    fn resource_key(
        &self,
        arguments: &serde_json::Value,
        base: &std::path::Path,
    ) -> Option<std::path::PathBuf> {
        crate::search::search_key(arguments, base)
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
            Some(p) => util::resolve_or_refuse(&ctx.cwd, p)?,
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
        // Every match is counted, then the cap is applied: a capped result has to be able to
        // say how many it is holding back, and pi's notice names the cap rather than the
        // total (pi `find.ts:281-285`). Counting costs nothing here — `walk_files` has
        // already materialized the whole walk.
        let all: Vec<String> = util::walk_files(&root)
            .iter()
            .map(|p| norm(p))
            .filter(|rel| re.is_match(rel))
            .collect();
        if all.is_empty() {
            return Ok(ToolOutput::text("no matches"));
        }
        let total = all.len();
        let count_shown = total.min(limit);
        // Two independent bounds, and the notice has to name the one that actually cut the
        // list: "results limit reached … Use limit=N" is advice that does nothing when the
        // byte budget is what stopped it (pi `find.ts:275-293` applies both, head-truncating
        // the joined paths at `DEFAULT_MAX_BYTES`).
        let (listing, byte_shown) =
            util::truncate_head_bytes(&all[..count_shown], util::TOOL_OUTPUT_MAX_BYTES);
        let shown = byte_shown;
        let mut text = format!("{}\n[{} file(s)]", listing, shown);
        if shown < total {
            let hidden = total - shown;
            text.push_str("\n\n[");
            if shown < count_shown {
                // The byte bound fired first, so raising `limit` would not show more.
                text.push_str(&format!(
                    "{} output limit reached: {} more file(s) match and are not shown. \
                     Use a narrower pattern to see the rest",
                    util::format_size(util::TOOL_OUTPUT_MAX_BYTES),
                    hidden
                ));
            } else {
                // pi says "N results limit reached. Use limit=2N …"; the count of what is
                // missing is added because "limit reached" alone does not tell a reader what
                // they are not seeing (the class of silent truncation G19 turned up).
                text.push_str(&format!(
                    "{} results limit reached: {} more file(s) match and are not shown. \
                     Use limit={} for more, or refine pattern",
                    limit,
                    hidden,
                    limit * 2
                ));
            }
            text.push(']');
        }
        Ok(ToolOutput::text(text))
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

    #[test]
    fn resource_key_covers_default_and_explicit_search_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let tool = FindTool::new();
        assert_eq!(
            tool.resource_key(&serde_json::json!({"pattern": "*.rs"}), root),
            Some(root.to_path_buf())
        );
        for path in ["src", "./src", "nested/../src"] {
            let key = tool
                .resource_key(&serde_json::json!({"pattern": "*.rs", "path": path}), root)
                .expect("find must declare its search directory");
            assert_eq!(key, root.join("src"));
            assert!(root.join("src/lib.rs").starts_with(&key));
            assert!(!root.join("tests/main.rs").starts_with(&key));
        }
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
        // The result is `f0…\nf1…\n[2 file(s)]` followed by a blank line and the capped notice,
        // so the filter stops at that blank line rather than at every bracketed line.
        let files: Vec<&str> = out.lines().take_while(|l| !l.is_empty()).collect();
        assert_eq!(files.len(), 3, "2 files plus the [2 file(s)] count line");
        assert_eq!(files[2], "[2 file(s)]", "got: {out}");
        // The walker sorts by name, so "the first 2" is f0 and f1 and the cap is exact.
        assert!(
            out.contains("f0.txt") && out.contains("f1.txt"),
            "got: {out}"
        );
        assert!(
            !out.contains("f2.txt") && !out.contains("f3.txt") && !out.contains("f4.txt"),
            "limit 2 must return only 2 files: {out}"
        );
        assert!(
            out.contains("2 results limit reached: 3 more file(s)"),
            "the cap is announced with what it hides: {out}"
        );
    }

    /// G21a: the description advertises a byte budget, so it has to exist, and a result whose
    /// size crosses it has to say *that* bound rather than the entry count — the reader's next
    /// move differs (a narrower pattern versus `limit=2N`).
    ///
    /// The fixture crosses the threshold in real bytes: 400 entries of ~169 bytes each is
    /// ~66 KiB of listing, well over [`util::TOOL_OUTPUT_MAX_BYTES`], while 400 is far under
    /// the 1000-result default, so the count bound cannot be what fires. Asserting the real
    /// constant also proves this is bytes and not entries.
    #[tokio::test]
    async fn announces_the_byte_budget_when_bytes_cut_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        // Sorted by name, so `row000` … `row399` are the order the walker produces.
        for i in 0..400 {
            fs::write(root.join(format!("row{i:03} {}.txt", "x".repeat(150))), "").unwrap();
        }
        // The fixture size is the point of this test, so it is checked at compile time:
        // 400 entries of `rowNNN ` + 150 chars + `.txt` + newline cross the budget.
        const _: () = assert!(400 * (150 + 9) > util::TOOL_OUTPUT_MAX_BYTES);
        let out = run_find("*.txt", &root, None).await;
        assert!(
            out.contains("50.0KB output limit reached"),
            "the byte bound must name itself: {out}"
        );
        assert!(
            !out.contains("results limit reached"),
            "400 results is under the 1000 cap, so the count bound did not fire: {out}"
        );
        assert!(
            out.contains("more file(s) match and are not shown. Use a narrower pattern"),
            "and say what to do instead of raising a limit that is not the bound: {out}"
        );
        // The listing itself stopped at the byte bound: the last entry cannot appear.
        assert!(
            !out.contains("row399 "),
            "a byte-capped listing must not contain entries past the cap: {out}"
        );
    }

    /// The other half of G21a's rule: a result set under the bound is returned unchanged.
    #[tokio::test]
    async fn under_the_byte_budget_says_nothing_about_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for i in 0..3 {
            fs::write(root.join(format!("small{i}.txt")), "").unwrap();
        }
        let out = run_find("*.txt", &root, None).await;
        assert!(out.contains("[3 file(s)]"), "got: {out}");
        assert!(
            !out.contains("output limit reached"),
            "a complete result set must not invent a bound notice: {out}"
        );
    }
}
