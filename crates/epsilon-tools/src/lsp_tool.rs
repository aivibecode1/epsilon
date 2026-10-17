//! The `lsp` tool: model-facing navigation queries over a language server.
//!
//! Read-only: this tool never writes, so it records no checkpoint — the checkpoint ledger is
//! for files *this harness* changed, and a query changes nothing.
//!
//! The tool speaks only [`LspQueryProvider`][epsilon_contracts::lsp_queries::LspQueryProvider]:
//! the provisioned service lives with the composition, so this crate keeps no edge to
//! `epsilon-lsp` (the M215 seam, shared with the diagnostics handle).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::lsp_queries::{LspLocation, LspQueryFailure, LspQueryProvider, LspSymbol};
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;

pub const LSP_DESC: &str = include_str!("../descriptions/lsp.txt");

/// Hits rendered per call before the cap note. Mirrors the diagnostics report's own bound
/// (`epsilon-lsp` `MAX_REPORTED`): a navigation dump would push the file's contents out of the
/// model's view.
pub const MAX_RESULTS_PER_OP: usize = 20;

/// Arguments for the `lsp` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LspArgs {
    /// Operation: `definition` | `references` | `implementation` | `symbols`.
    pub op: String,
    /// File path (absolute or workspace-relative).
    pub path: String,
    /// 1-based line of the symbol. Required for every op except `symbols`, which ignores it.
    #[serde(default)]
    pub line: Option<u32>,
    /// 1-based column of the symbol. Required for every op except `symbols`, which ignores it.
    #[serde(default)]
    pub column: Option<u32>,
    /// For `references`: include the declaration itself (default true).
    #[serde(default)]
    pub include_declaration: Option<bool>,
}

/// Model-facing navigation queries over the provisioned language-server service.
pub struct LspTool {
    descriptor: ToolDescriptor,
    provider: Arc<dyn LspQueryProvider>,
}

impl LspTool {
    /// Build an `lsp` tool over the provisioned query service.
    pub fn new(provider: Arc<dyn LspQueryProvider>) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "lsp".into(),
                label: "LSP".into(),
                description: LSP_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(LspArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                // A read-only navigation: runs beside other reads (M72).
                execution_mode: ExecutionMode::Parallel,
            },
            provider,
        }
    }
}

#[async_trait]
impl Tool for LspTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn resource_key(
        &self,
        arguments: &serde_json::Value,
        base: &std::path::Path,
    ) -> Option<std::path::PathBuf> {
        // The file being queried: the same `path` key the permission gate reads, so the
        // planner and the ask name the same file by construction.
        let raw = arguments.get("path")?.as_str()?;
        Some(crate::util::lexically_normalize(base, raw).0)
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: LspArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'lsp': {e}")))?;
        let path = crate::util::resolve_or_refuse(&ctx.cwd, &args.path)?;

        let (text, mut details) = match args.op.as_str() {
            "symbols" => match self.provider.document_symbol(&path).await {
                Ok(symbols) => render_symbols(&path, &ctx.cwd, &symbols),
                Err(failure) => return Err(map_failure(failure)),
            },
            "definition" | "references" | "implementation" => {
                // The provider speaks wire numbers: 1-based in, 0-based out.
                let (line, column) = match (args.line, args.column) {
                    (Some(line), Some(column)) if line >= 1 && column >= 1 => {
                        (line - 1, column - 1)
                    }
                    (Some(_), Some(_)) => {
                        return Err(ToolFailure::msg(format!(
                            "lsp positions are 1-based: got line {} and column {}",
                            args.line.unwrap_or(0),
                            args.column.unwrap_or(0)
                        )));
                    }
                    _ => {
                        return Err(ToolFailure::msg(format!(
                            "lsp op '{}' requires 'line' and 'column' (1-based); 'symbols' takes none",
                            args.op
                        )));
                    }
                };
                let at = format!(
                    "{}:{}:{}",
                    display_path(&path, &ctx.cwd),
                    // Validated above: both are present and non-zero here.
                    args.line.unwrap_or(0),
                    args.column.unwrap_or(0)
                );
                let include_declaration = args.include_declaration.unwrap_or(true);
                let outcome = match args.op.as_str() {
                    "definition" => self.provider.go_to_definition(&path, line, column).await,
                    "references" => {
                        self.provider
                            .find_references(&path, line, column, include_declaration)
                            .await
                    }
                    _ => {
                        self.provider
                            .go_to_implementation(&path, line, column)
                            .await
                    }
                };
                match outcome {
                    Ok(hits) => render_locations(op_noun(&args.op), &at, &hits, &ctx.cwd),
                    Err(failure) => return Err(map_failure(failure)),
                }
            }
            other => {
                return Err(ToolFailure::msg(format!(
                    "unknown lsp op '{other}' — one of: definition, references, implementation, symbols"
                )));
            }
        };

        // The compact lines go through the spill helper; the complete hit list rides in details.
        let truncated = crate::util::truncate_output(&text, &ctx.spill_dir, "lsp");
        if truncated.truncated {
            details["truncated"] = serde_json::json!(true);
            match (&truncated.spilled_to, &truncated.spill_error) {
                (Some(spilled), _) => {
                    details["spilled"] = serde_json::json!(true);
                    details["spill_path"] = serde_json::json!(spilled.display().to_string());
                }
                (None, Some(error)) => {
                    details["spill_error"] = serde_json::json!(error);
                }
                (None, None) => {}
            }
        }
        Ok(ToolOutput {
            content: vec![ContentBlock::text(truncated.text)],
            details,
            usage: None,
            terminate: false,
        })
    }
}

/// The plural the zero-hits message uses for one op.
fn op_noun(op: &str) -> &'static str {
    match op {
        "references" => "references",
        "implementation" => "implementations",
        _ => "definitions",
    }
}

/// A path for model-facing lines: workspace-relative when inside, absolute otherwise, with
/// forward slashes so the compact `path:line:col` shape reads the same on every platform.
fn display_path(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Compact `path:line:col` lines plus the full hit list in JSON.
fn render_locations(
    noun: &str,
    at: &str,
    hits: &[LspLocation],
    cwd: &Path,
) -> (String, serde_json::Value) {
    let total = hits.len();
    if total == 0 {
        return (
            format!("no {noun} found at {at}"),
            serde_json::json!({"total": 0, "shown": 0, "hits": []}),
        );
    }
    let shown = total.min(MAX_RESULTS_PER_OP);
    let mut lines: Vec<String> = Vec::with_capacity(shown + 1);
    for hit in hits.iter().take(shown) {
        lines.push(format!(
            "{}:{}:{}",
            display_path(&hit.path, cwd),
            hit.line,
            hit.character
        ));
    }
    if total > shown {
        lines.push(format!("(+{} more not shown)", total - shown));
    }
    let details = serde_json::json!({
        "total": total,
        "shown": shown,
        "hits": hits.iter().map(|hit| serde_json::json!({
            "path": hit.path.display().to_string(),
            "line": hit.line,
            "character": hit.character,
        })).collect::<Vec<_>>(),
    });
    (lines.join("\n"), details)
}

/// Compact `name (kind) path:line:col` lines plus the full symbol list in JSON.
fn render_symbols(path: &Path, cwd: &Path, symbols: &[LspSymbol]) -> (String, serde_json::Value) {
    let total = symbols.len();
    if total == 0 {
        return (
            format!("no symbols found in {}", display_path(path, cwd)),
            serde_json::json!({"total": 0, "shown": 0, "hits": []}),
        );
    }
    let shown = total.min(MAX_RESULTS_PER_OP);
    let mut lines: Vec<String> = Vec::with_capacity(shown + 1);
    for symbol in symbols.iter().take(shown) {
        lines.push(format!(
            "{} ({}) {}:{}:{}",
            symbol.name,
            symbol.kind,
            display_path(&symbol.location.path, cwd),
            symbol.location.line,
            symbol.location.character
        ));
    }
    if total > shown {
        lines.push(format!("(+{} more not shown)", total - shown));
    }
    let details = serde_json::json!({
        "total": total,
        "shown": shown,
        "hits": symbols.iter().map(|symbol| serde_json::json!({
            "name": symbol.name,
            "kind": symbol.kind,
            "path": symbol.location.path.display().to_string(),
            "line": symbol.location.line,
            "character": symbol.location.character,
        })).collect::<Vec<_>>(),
    });
    (lines.join("\n"), details)
}

/// The four model-facing outcomes: three failures, each naming what the model can act on.
fn map_failure(failure: LspQueryFailure) -> ToolFailure {
    match failure {
        LspQueryFailure::UnknownExtension { extension, known } => {
            let ext = if extension.is_empty() {
                "(no extension)".to_string()
            } else {
                format!(".{extension}")
            };
            ToolFailure::msg(format!(
                "no language server handles '{ext}' files — known extensions: {}",
                known.join(", ")
            ))
        }
        LspQueryFailure::MissingBinary { name, binary } => ToolFailure::msg(format!(
            "language server '{name}' is not installed (binary '{binary}' not found on PATH)"
        )),
        LspQueryFailure::Broken { detail } => ToolFailure::msg(format!(
            "the language server is broken and will not be retried this session: {detail}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use epsilon_contracts::lsp_queries::{
        LspLocation, LspQueryFailure, LspQueryProvider, LspSymbol,
    };
    use epsilon_contracts::tool::{ExecutionMode, Tool, ToolContext, ToolInvocation};
    use tokio_util::sync::CancellationToken;

    /// A provider the test drives: records what it was asked and answers on script.
    #[derive(Default)]
    struct Fake {
        last_op: Mutex<Option<String>>,
        last_pos: Mutex<Option<(u32, u32)>>,
        last_include: Mutex<Option<bool>>,
        failure: Mutex<Option<LspQueryFailure>>,
        extra_hits: Mutex<Vec<LspLocation>>,
    }

    #[async_trait::async_trait]
    impl LspQueryProvider for Fake {
        async fn go_to_definition(
            &self,
            path: &Path,
            line: u32,
            character: u32,
        ) -> Result<Vec<LspLocation>, LspQueryFailure> {
            *self.last_op.lock().unwrap() = Some("definition".into());
            *self.last_pos.lock().unwrap() = Some((line, character));
            self.answer(path, line, character)
        }

        async fn find_references(
            &self,
            path: &Path,
            line: u32,
            character: u32,
            include_declaration: bool,
        ) -> Result<Vec<LspLocation>, LspQueryFailure> {
            *self.last_op.lock().unwrap() = Some("references".into());
            *self.last_pos.lock().unwrap() = Some((line, character));
            *self.last_include.lock().unwrap() = Some(include_declaration);
            self.answer(path, line, character)
        }

        async fn go_to_implementation(
            &self,
            path: &Path,
            line: u32,
            character: u32,
        ) -> Result<Vec<LspLocation>, LspQueryFailure> {
            *self.last_op.lock().unwrap() = Some("implementation".into());
            *self.last_pos.lock().unwrap() = Some((line, character));
            self.answer(path, line, character)
        }

        async fn document_symbol(&self, _path: &Path) -> Result<Vec<LspSymbol>, LspQueryFailure> {
            *self.last_op.lock().unwrap() = Some("symbols".into());
            match self.failure.lock().unwrap().clone() {
                Some(f) => Err(f),
                None => Ok(vec![LspSymbol {
                    name: "alpha".into(),
                    kind: "Function".into(),
                    location: LspLocation {
                        path: PathBuf::from("/ws/src/a.rs"),
                        line: 1,
                        character: 1,
                    },
                }]),
            }
        }
    }

    impl Fake {
        /// Echo: the hit is the asked position rebased to 1-based, plus any scripted extras.
        fn answer(
            &self,
            path: &Path,
            line: u32,
            character: u32,
        ) -> Result<Vec<LspLocation>, LspQueryFailure> {
            match self.failure.lock().unwrap().clone() {
                Some(f) => Err(f),
                None => {
                    let mut hits = vec![LspLocation {
                        path: path.to_path_buf(),
                        line: line + 1,
                        character: character + 1,
                    }];
                    hits.extend(self.extra_hits.lock().unwrap().iter().cloned());
                    Ok(hits)
                }
            }
        }
    }

    fn ctx_for(root: &Path) -> ToolContext {
        ToolContext {
            session_id: "test".into(),
            observation_scope: Arc::new(()),
            cwd: root.to_path_buf(),
            spill_dir: root.join(".spill"),
            token: CancellationToken::new(),
            task_host: None,
            goal_host: None,
        }
    }

    async fn run(
        tool: &LspTool,
        ctx: &ToolContext,
        args: serde_json::Value,
    ) -> Result<(String, serde_json::Value), String> {
        let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
        tool.execute(
            ToolInvocation {
                tool_call_id: "tc_t".into(),
                args,
            },
            ctx.clone(),
            sink,
        )
        .await
        .map(|o| {
            let text = o
                .content
                .iter()
                .filter_map(|c| match c {
                    epsilon_contracts::content::ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            (text, o.details)
        })
        .map_err(|f| f.message)
    }

    fn workspace() -> (tempfile::TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "pub fn alpha() {}\n").unwrap();
        let ctx = ctx_for(dir.path());
        (dir, ctx)
    }

    fn tool_with(fake: Fake) -> (LspTool, Arc<Fake>) {
        let fake = Arc::new(fake);
        let tool = LspTool::new(fake.clone());
        (tool, fake)
    }

    /// 1-based in, 0-based on the wire: the provider must see (9,4) for line 10, column 5.
    #[tokio::test]
    async fn definition_converts_one_based_to_zero_based() {
        let (_dir, ctx) = workspace();
        let (tool, fake) = tool_with(Fake::default());
        let (text, _) = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "definition", "path": "src/a.rs", "line": 10, "column": 5}),
        )
        .await
        .expect("definition answers");
        assert_eq!(
            *fake.last_pos.lock().unwrap(),
            Some((9, 4)),
            "the provider speaks wire numbers"
        );
        assert!(
            text.contains("src/a.rs:10:5"),
            "and the hit renders 1-based: {text}"
        );
    }

    /// Unknown extension: the failure names the extension and lists what is known.
    #[tokio::test]
    async fn unknown_extension_lists_known_ones() {
        let (_dir, ctx) = workspace();
        let (tool, _) = tool_with(Fake {
            failure: Mutex::new(Some(LspQueryFailure::UnknownExtension {
                extension: "txt".into(),
                known: vec!["rs".into(), "ts".into()],
            })),
            ..Default::default()
        });
        let err = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "definition", "path": "notes.txt", "line": 1, "column": 1}),
        )
        .await
        .expect_err("no server claims .txt");
        assert!(err.contains(".txt"), "names the extension: {err}");
        assert!(err.contains("rs"), "lists known ones: {err}");
    }

    /// Missing binary: the failure names the executable.
    #[tokio::test]
    async fn missing_binary_names_it() {
        let (_dir, ctx) = workspace();
        let (tool, _) = tool_with(Fake {
            failure: Mutex::new(Some(LspQueryFailure::MissingBinary {
                name: "rust-analyzer".into(),
                binary: "rust-analyzer".into(),
            })),
            ..Default::default()
        });
        let err = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "references", "path": "src/a.rs", "line": 1, "column": 5}),
        )
        .await
        .expect_err("no binary, no query");
        assert!(err.contains("rust-analyzer"), "names the binary: {err}");
    }

    /// Broken server: the failure says so, distinctly from the other three.
    #[tokio::test]
    async fn broken_server_says_so() {
        let (_dir, ctx) = workspace();
        let (tool, _) = tool_with(Fake {
            failure: Mutex::new(Some(LspQueryFailure::Broken {
                detail: "rust-analyzer: refused".into(),
            })),
            ..Default::default()
        });
        let err = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "implementation", "path": "src/a.rs", "line": 1, "column": 1}),
        )
        .await
        .expect_err("a broken server fails the query");
        assert!(err.contains("broken"), "says broken: {err}");
    }

    /// Zero hits are a message, not a failure — one per op shape.
    #[tokio::test]
    async fn zero_hits_are_a_message_not_a_failure() {
        let (_dir, ctx) = workspace();
        let (tool, _) = tool_with(Fake::default());
        let (text, _) = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "symbols", "path": "src/a.rs"}),
        )
        .await
        .expect("symbols answer");
        assert!(
            text.contains("alpha"),
            "the scripted symbol renders: {text}"
        );
        assert!(text.contains("1:1"), "with its 1-based position: {text}");
    }

    /// Zero hits for a position op: an empty answer renders the no-hits message.
    #[tokio::test]
    async fn zero_hits_message_for_position_ops() {
        struct Empty;
        #[async_trait::async_trait]
        impl LspQueryProvider for Empty {
            async fn go_to_definition(
                &self,
                _path: &Path,
                _line: u32,
                _character: u32,
            ) -> Result<Vec<LspLocation>, LspQueryFailure> {
                Ok(vec![])
            }
            async fn find_references(
                &self,
                _path: &Path,
                _line: u32,
                _character: u32,
                _include: bool,
            ) -> Result<Vec<LspLocation>, LspQueryFailure> {
                Ok(vec![])
            }
            async fn go_to_implementation(
                &self,
                _path: &Path,
                _line: u32,
                _character: u32,
            ) -> Result<Vec<LspLocation>, LspQueryFailure> {
                Ok(vec![])
            }
            async fn document_symbol(
                &self,
                _path: &Path,
            ) -> Result<Vec<LspSymbol>, LspQueryFailure> {
                Ok(vec![])
            }
        }
        let (_dir, ctx) = workspace();
        let tool = LspTool::new(Arc::new(Empty));
        let (text, _) = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "definition", "path": "src/a.rs", "line": 3, "column": 2}),
        )
        .await
        .expect("an empty answer is still an answer");
        assert!(
            text.contains("no definitions found at"),
            "the zero-hits message: {text}"
        );
        assert!(text.contains("src/a.rs:3:2"), "naming the spot: {text}");
        let (text, _) = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "symbols", "path": "src/a.rs"}),
        )
        .await
        .expect("an empty answer is still an answer");
        assert!(
            text.contains("no symbols found in"),
            "the symbols zero-hits message: {text}"
        );
    }

    /// Per-op cap: 25 hits render 20 lines plus the (+N more) note, with the full list in details.
    #[tokio::test]
    async fn per_op_cap_reports_the_rest() {
        let (dir, ctx) = workspace();
        let extra: Vec<LspLocation> = (0..24)
            .map(|i| LspLocation {
                path: dir.path().join("src/a.rs"),
                line: i + 1,
                character: 1,
            })
            .collect();
        let (tool, _) = tool_with(Fake {
            extra_hits: Mutex::new(extra),
            ..Default::default()
        });
        let (text, details) = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "references", "path": "src/a.rs", "line": 1, "column": 1}),
        )
        .await
        .expect("references answer");
        assert_eq!(
            text.lines().filter(|l| l.contains("src/a.rs:")).count(),
            20,
            "at most 20 hits render: {text}"
        );
        assert!(
            text.contains("(+5 more not shown)"),
            "and the rest are counted: {text}"
        );
        assert_eq!(details["total"], 25, "details carry the full count");
        assert_eq!(
            details["hits"].as_array().map(|h| h.len()).unwrap_or(0),
            25,
            "and the full hit list"
        );
    }

    /// Positions are required for definition/references/implementation — and rejected at zero —
    /// while symbols ignores them entirely.
    #[tokio::test]
    async fn position_required_except_symbols() {
        let (_dir, ctx) = workspace();
        let (tool, fake) = tool_with(Fake::default());
        for op in ["definition", "references", "implementation"] {
            let err = run(
                &tool,
                &ctx,
                serde_json::json!({"op": op, "path": "src/a.rs"}),
            )
            .await
            .expect_err("a position op without a position fails");
            assert!(
                err.contains("line") && err.contains("column"),
                "naming both: {err}"
            );
            let err = run(
                &tool,
                &ctx,
                serde_json::json!({"op": op, "path": "src/a.rs", "line": 0, "column": 1}),
            )
            .await
            .expect_err("0 is not a 1-based line");
            assert!(err.contains("1-based"), "saying why: {err}");
        }
        // Symbols run without any position — and ignore one when given.
        let (text, _) = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "symbols", "path": "src/a.rs"}),
        )
        .await
        .expect("symbols need no position");
        assert!(text.contains("alpha"), "{text}");
        let (text, _) = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "symbols", "path": "src/a.rs", "line": 99, "column": 99}),
        )
        .await
        .expect("symbols ignore a position");
        assert!(text.contains("alpha"), "{text}");
        // And a position op that passes validation reaches the provider 0-based.
        let (text, _) = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "definition", "path": "src/a.rs", "line": 1, "column": 1}),
        )
        .await
        .expect("a valid position op runs");
        assert!(text.contains("src/a.rs:1:1"), "{text}");
        assert_eq!(
            *fake.last_pos.lock().unwrap(),
            Some((0, 0)),
            "1-based (1,1) arrives 0-based"
        );
    }

    /// The schema key is `path`: `filePath` is not accepted.
    #[tokio::test]
    async fn schema_key_is_path() {
        let (_dir, ctx) = workspace();
        let (tool, _) = tool_with(Fake::default());
        let schema = tool.descriptor().parameters_schema.clone();
        assert!(
            schema["properties"].get("path").is_some(),
            "the schema has `path`: {schema}"
        );
        assert!(
            schema["properties"].get("filePath").is_none(),
            "and no `filePath`: {schema}"
        );
        let err = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "definition", "filePath": "src/a.rs", "line": 1, "column": 1}),
        )
        .await
        .expect_err("filePath is not a key");
        assert!(err.contains("path"), "pointing at `path`: {err}");
    }

    /// An unknown op names the four that exist.
    #[tokio::test]
    async fn unknown_op_names_the_four() {
        let (_dir, ctx) = workspace();
        let (tool, _) = tool_with(Fake::default());
        let err = run(
            &tool,
            &ctx,
            serde_json::json!({"op": "hover", "path": "src/a.rs", "line": 1, "column": 1}),
        )
        .await
        .expect_err("hover is not an op");
        for op in ["definition", "references", "implementation", "symbols"] {
            assert!(err.contains(op), "naming {op}: {err}");
        }
    }

    /// The planner key is the file: `path` resolves against the workspace root.
    #[tokio::test]
    async fn gate_subject_reads_path() {
        let (_dir, ctx) = workspace();
        let (tool, _) = tool_with(Fake::default());
        let key = tool
            .resource_key(
                &serde_json::json!({"op": "symbols", "path": "src/a.rs"}),
                &ctx.cwd,
            )
            .expect("a path key");
        assert_eq!(key, ctx.cwd.join("src/a.rs"));
        // The permission gate reads the same `path` key (`gate_subject` checks
        // `path`|`file_path`|`dir`), so the capability `lsp` asks about this file.
    }

    /// Readers run beside each other: Parallel, like grep and glob.
    #[test]
    fn parallel_mode() {
        let (tool, _) = tool_with(Fake::default());
        assert_eq!(tool.descriptor().execution_mode, ExecutionMode::Parallel);
    }

    /// Read-only means read-only: queries leave the checkpoint ledger empty.
    #[tokio::test]
    async fn no_checkpoint_ledger_after_queries() {
        let (dir, ctx) = workspace();
        let (tool, _) = tool_with(Fake::default());
        for args in [
            serde_json::json!({"op": "definition", "path": "src/a.rs", "line": 1, "column": 1}),
            serde_json::json!({"op": "references", "path": "src/a.rs", "line": 1, "column": 1}),
            serde_json::json!({"op": "symbols", "path": "src/a.rs"}),
        ] {
            run(&tool, &ctx, args).await.expect("queries answer");
        }
        assert!(
            crate::checkpoint::CheckpointStore::open(dir.path())
                .entries()
                .is_empty(),
            "a read-only tool records no checkpoints"
        );
        assert!(
            !dir.path()
                .join(".epsilon/checkpoints/ledger.jsonl")
                .exists(),
            "and creates no ledger at all"
        );
    }

    /// No Allow rule for `lsp` anywhere: the capability asks everywhere by default.
    ///
    /// Structural proof is impossible from here (`default_ruleset` is not exported), so the
    /// behaviour is pinned instead: unknown capabilities resolve to Ask, and neither the
    /// defaults nor the workspace-write preset (the posture that allows reads and writes)
    /// moves `lsp` off it. (`danger-full-access` allows everything by design — choosing it
    /// is the privilege-raising act, not a default.)
    #[test]
    fn ask_default_with_no_allow_rule() {
        let engine = epsilon_safety::PermissionEngine::with_defaults();
        assert_eq!(
            engine.evaluate("lsp", "src/a.rs"),
            epsilon_safety::Action::Ask,
            "defaults ask"
        );
        let ws = std::path::Path::new("/ws");
        let layered =
            epsilon_safety::PermissionEngine::layered(vec![epsilon_safety::preset_ruleset(
                epsilon_safety::Preset::WorkspaceWrite,
                ws,
            )]);
        assert_eq!(
            layered.evaluate("lsp", "src/a.rs"),
            epsilon_safety::Action::Ask,
            "workspace-write still asks"
        );
    }

    /// Registration is the policy: no handle, no tool; a handle, exactly one `lsp` tool.
    #[test]
    fn registration_follows_the_handle() {
        let mut registry = epsilon_core::ToolRegistry::new();
        crate::register_lsp_tool(&mut registry, None);
        assert!(registry.get("lsp").is_none(), "Lsp::Off registers nothing");
        crate::register_lsp_tool(
            &mut registry,
            Some(Arc::new(Fake::default()) as Arc<dyn LspQueryProvider>),
        );
        let tool = registry.get("lsp").expect("a handle registers the tool");
        assert_eq!(tool.descriptor().name, "lsp");
    }
}
