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
use epsilon_contracts::diagnostics::DiagnosticsProvider;
use std::sync::Arc;

/// Ask the diagnostics provider about a file this tool just wrote, and return the block to append.
///
/// Three rules, all of them about not letting a linter break an edit that already happened: no
/// provider means no block; a file that cannot be re-read means no block; a provider that is
/// silent, broken or wrong means no block. The caller appends what comes back and nothing else
/// (M79; opencode `tool/edit.ts:196-211`).
async fn diagnostics_note(
    provider: Option<&Arc<dyn DiagnosticsProvider>>,
    path: &std::path::Path,
) -> String {
    let Some(provider) = provider else {
        return String::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    match provider.after_change(path, &text).await {
        Some(block) => format!("\n{block}"),
        None => String::new(),
    }
}

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

    fn resource_key(
        &self,
        arguments: &serde_json::Value,
        base: &std::path::Path,
    ) -> Option<std::path::PathBuf> {
        path_key(arguments, base)
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
        // Images ride as attachments (pi `createReadTool` image branch): magic
        // bytes first, so a `.png` that is really text still reads as text and
        // a text file with image bytes never parses as text.
        let bytes = std::fs::read(&path)
            .map_err(|e| ToolFailure::msg(format!("cannot open {}: {e}", path.display())))?;
        if let Some(mime) = crate::image::detect_supported_image_mime(&bytes) {
            if mime == "image/bmp" {
                // No BMP converter in the tree (pi needs an injected
                // imageProcessor for BMP too) — say so explicitly instead of
                // shipping undecodable bytes to the provider.
                return Ok(ToolOutput::text(
                    "Read image file [image/bmp]\n[Image omitted: BMP images are not convertible in this build.]",
                ));
            }
            return Ok(ToolOutput {
                content: vec![
                    epsilon_contracts::content::ContentBlock::text(format!(
                        "Read image file [{mime}]"
                    )),
                    epsilon_contracts::content::ContentBlock::Image(
                        epsilon_contracts::content::ImageRef {
                            mime: mime.into(),
                            data: base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                &bytes,
                            ),
                        },
                    ),
                ],
                details: serde_json::json!({}),
                usage: None,
                terminate: false,
            });
        }
        let (content, capped) = util::read_text_capped_ex(&path)?;

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
        // Was this a window onto the file rather than the whole of it? Either the caller asked
        // for a range, or the file is longer than the line cap. The answer decides whether a
        // later full overwrite is legitimate (`crate::read_log`).
        let windowed = start > 1 || slice.len() < total || total >= util::MAX_READ_LINES || capped;
        if total > util::MAX_READ_LINES && slice.len() == limit {
            body.push_str(&format!(
                "\n[file has {total} lines; showing {start}-{}; pass offset to continue]",
                start + slice.len() - 1
            ));
        }
        if capped {
            // Say it out loud: a silent cap makes a 1 MB file look fully read, and the model
            // then reasons about a file it has only seen the start of.
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            body.push_str(&format!(
                "\n[file is {len} bytes; this read is limited to the first {} bytes — \
                 use grep or bash to inspect the rest]",
                util::MAX_READ_BYTES
            ));
        }
        if body.is_empty() {
            body = "(empty or past end of file)".to_string();
        }
        crate::read_log::record_read(&ctx.observation_scope, &path, windowed, capped);
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
    /// Optional LSP feedback on what was just written (M79).
    diagnostics: Option<Arc<dyn DiagnosticsProvider>>,
}

impl WriteTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            diagnostics: None,
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

impl WriteTool {
    /// Attach a diagnostics provider: every successful write then carries what the language server
    /// thinks of the result.
    pub fn with_diagnostics(mut self, provider: Arc<dyn DiagnosticsProvider>) -> Self {
        self.diagnostics = Some(provider);
        self
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

/// Turn the `path` argument into a planner key (M72).
///
/// Lexical normalisation against the same base the tool will execute with, so two calls that
/// spell one file two ways are seen as one file. A missing or non-string `path` yields `None`:
/// the call keeps the conservative scheduling rather than being assumed independent.
fn path_key(arguments: &serde_json::Value, base: &std::path::Path) -> Option<std::path::PathBuf> {
    let raw = arguments.get("path")?.as_str()?;
    Some(util::resolve_in_workspace(base, raw).0)
}

#[async_trait]
impl Tool for WriteTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn resource_key(
        &self,
        arguments: &serde_json::Value,
        base: &std::path::Path,
    ) -> Option<std::path::PathBuf> {
        path_key(arguments, base)
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
        let queue_path = path.clone();
        let content = args.content.clone();
        let display = args.path.clone();
        let provider = self.diagnostics.clone();
        let workspace = ctx.cwd.clone();
        crate::mutation::with_file_mutation_queue(&queue_path, move || {
            let path = path.clone();
            let content = content.clone();
            let display = display.clone();
            let provider = provider.clone();
            let workspace = workspace.clone();
            async move {
                // Refuse to overwrite something that changed since the model read it, or that
                // the model never fully read (`read_log`). Runs inside the queue so no other
                // harness writer can slip between the check and the write.
                crate::read_log::verify(&ctx.observation_scope, &path, true)
                    .map_err(|s| ToolFailure::msg(s.diagnosis("write")))?;
                // Checkpoint before the bytes move (M80): the state a revert would restore.
                crate::checkpoint::CheckpointStore::open(&workspace)
                    .record(&workspace, &path, "write");
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| ToolFailure::msg(format!("cannot create parent dir: {e}")))?;
                }
                let existed = path.exists();
                std::fs::write(&path, &content)
                    .map_err(|e| ToolFailure::msg(format!("write failed: {e}")))?;
                crate::read_log::record_write(&ctx.observation_scope, &path);
                let mut text = format!(
                    "{} {} ({} bytes)",
                    if existed { "overwrote" } else { "created" },
                    display,
                    content.len()
                );
                text.push_str(&diagnostics_note(provider.as_ref(), &path).await);
                Ok(ToolOutput::text(text))
            }
        })
        .await
    }
}

// ---- edit -----------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditEntry {
    /// Exact text to find (unique unless replace_all is set on the parent).
    /// Accepts pi's `oldText` wire name as an alias.
    #[serde(alias = "oldText")]
    pub old_string: String,
    /// Replacement text. Accepts pi's `newText` wire name as an alias.
    #[serde(alias = "newText")]
    pub new_string: String,
}

/// Arguments for the edit tool.
///
/// Accepts both the pi-compatible batch shape `{path, edits:[{old_string,new_string}...]}` and
/// the legacy single-edit shape `{path, old_string, new_string}`. The model always sends `edits`;
/// `old_string`/`new_string` is kept for typed backward compat with existing tests.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditArgs {
    /// File to edit.
    pub path: String,
    /// Batch edits — each entry is an exact unique substring to replace.
    #[serde(default)]
    pub edits: Vec<EditEntry>,
    /// Legacy single-edit: exact text to find (must match once unless replace_all).
    /// Accepts pi's `oldText` wire name as an alias.
    #[serde(default, alias = "oldText")]
    pub old_string: Option<String>,
    /// Legacy single-edit: replacement text. Accepts pi's `newText` wire name as an alias.
    #[serde(default, alias = "newText")]
    pub new_string: Option<String>,
    /// Replace every occurrence instead of failing on multiples (single-edit only).
    #[serde(default)]
    pub replace_all: bool,
}

pub const EDIT_DESC: &str = include_str!("../descriptions/edit.txt");

/// Exact-string replacement editor with occurrence-count safety (pi/opencode semantics).
pub struct EditTool {
    descriptor: ToolDescriptor,
    /// Optional LSP feedback on what was just edited (M79).
    diagnostics: Option<Arc<dyn DiagnosticsProvider>>,
}

impl EditTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            diagnostics: None,
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

    /// Attach a diagnostics provider: every successful edit then carries what the language server
    /// thinks of the result.
    pub fn with_diagnostics(mut self, provider: Arc<dyn DiagnosticsProvider>) -> Self {
        self.diagnostics = Some(provider);
        self
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

    fn resource_key(
        &self,
        arguments: &serde_json::Value,
        base: &std::path::Path,
    ) -> Option<std::path::PathBuf> {
        path_key(arguments, base)
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let mut args_value = call.args;
        // Pi parity (`prepareEditArguments`): some models send `edits` as a JSON
        // string instead of an array. Parse it in place; on failure leave it and
        // let schema validation report the shape error.
        if let Some(edits_str) = args_value.get("edits").and_then(|v| v.as_str())
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(edits_str)
        {
            args_value["edits"] = parsed;
        }
        let args: EditArgs = serde_json::from_value(args_value)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'edit': {e}")))?;
        // Normalize to a batch: prefer `edits` when present, else single old/new.
        let mut batch: Vec<(String, String)> = Vec::new();
        if !args.edits.is_empty() {
            for e in &args.edits {
                if e.old_string.is_empty() {
                    return Err(ToolFailure::msg("edits[].old_string must not be empty"));
                }
                batch.push((e.old_string.clone(), e.new_string.clone()));
            }
            if args.replace_all {
                return Err(ToolFailure::msg(
                    "replace_all is not supported with batch edits[]; send one edit per call or split the batch",
                ));
            }
        } else if let (Some(o), Some(n)) = (args.old_string.clone(), args.new_string.clone()) {
            if o.is_empty() {
                return Err(ToolFailure::msg("old_string must not be empty"));
            }
            batch.push((o, n));
        } else {
            return Err(ToolFailure::msg(
                "edit requires either edits:[{old_string,new_string}...] (pi wire names oldText/newText also accepted) or old_string+new_string",
            ));
        }

        let path = ensure_inside(&ctx, &args.path)?;
        let queue_path = path.clone();
        let args_path = args.path.clone();
        let replace_all = args.replace_all;
        let provider = self.diagnostics.clone();
        let workspace = ctx.cwd.clone();
        crate::mutation::with_file_mutation_queue(&queue_path, move || {
            let path = path.clone();
            let args_path = args_path.clone();
            let batch = batch.clone();
            let provider = provider.clone();
            let workspace = workspace.clone();
            async move {
                // Same guard as `write`, with `full_overwrite: false` — an edit matches exact
                // text the model saw, so a partially-read file is fine, but a file that moved
                // under it is not (`read_log`).
                crate::read_log::verify(&ctx.observation_scope, &path, false)
                    .map_err(|s| ToolFailure::msg(s.diagnosis("edit")))?;
                let raw = util::read_text_for_edit(&path)?;
                // One checkpoint per file per edit call, before the first byte changes (M80) —
                // both the single-edit and the batch path below run after this point.
                crate::checkpoint::CheckpointStore::open(&workspace).record(&workspace, &path, "edit");
                let eol = crate::edit_match::detect_eol(&raw);
                let content_lf = crate::edit_match::normalize_to_lf(&raw);

                // Single-edit: preserve existing fuzzy + replace_all behavior (exact path).
                if batch.len() == 1 {
                    let (old_string, new_string) = &batch[0];
                    let old_lf = crate::edit_match::normalize_to_lf(old_string);
                    let new_lf = crate::edit_match::normalize_to_lf(new_string);
                    match crate::edit_match::plan(&content_lf, &old_lf, &new_lf, replace_all) {
                        crate::edit_match::EditOutcome::Applied {
                            output,
                            count,
                            strategy,
                        } => {
                            let updated = crate::edit_match::restore_eol(&output, eol);
                            let diff = crate::util::unified_diff(&raw, &updated);
                            std::fs::write(&path, &updated)
                                .map_err(|e| ToolFailure::msg(format!("write failed: {e}")))?;
                            crate::read_log::record_write(&ctx.observation_scope, &path);
                            let mut details = serde_json::json!({});
                            if !diff.is_empty() {
                                details["diff"] = serde_json::Value::String(diff);
                            }
                            let mut text = format!(
                                "edited {} ({count} replacement(s)){}",
                                args_path,
                                strategy.note()
                            );
                            text.push_str(&diagnostics_note(provider.as_ref(), &path).await);
                            return Ok(ToolOutput {
                                content: vec![epsilon_contracts::content::ContentBlock::text(text)],
                                details,
                                usage: None,
                                terminate: false,
                            });
                        }
                        crate::edit_match::EditOutcome::Ambiguous(n) => {
                            return Err(ToolFailure::msg(format!(
                                "old_string matches {n} locations in {}; widen the context or set replace_all",
                                args_path
                            )))
                        }
                        crate::edit_match::EditOutcome::Suspicious {
                            span_lines,
                            old_lines,
                            extra_chars,
                        } => {
                            // M71: a loose stage found *something*, but it is far bigger than
                            // the text the model asked to replace. Applying it would rewrite a
                            // block nobody asked to rewrite, so it is refused with the numbers.
                            return Err(ToolFailure::msg(format!(
                                "refused to edit {args_path}: matching on the first and last lines \
                                 found a region {span_lines} lines tall for a {old_lines}-line \
                                 old_string ({extra_chars} extra characters). This usually means \
                                 the anchors matched a different block. Re-read the file and give \
                                 the exact text to replace, or use a longer, more distinctive \
                                 old_string."
                            )));
                        }
                        crate::edit_match::EditOutcome::NotFound => {
                            let old_lf2 = crate::edit_match::normalize_to_lf(&batch[0].0);
                            let first_line = old_lf2.lines().next().unwrap_or("");
                            let hint = content_lf
                                .lines()
                                .find(|l| !first_line.is_empty() && l.contains(first_line.trim()))
                                .unwrap_or("");
                            return Err(ToolFailure::msg(format!(
                                "old_string not found in {} (exact, whitespace-trimmed, \
                         indentation-flexible, escape-normalized, blank-line-trimmed, and \
                         anchor-similarity matching all failed).{}",
                                args_path,
                                if hint.is_empty() {
                                    String::new()
                                } else {
                                    format!(" Closest matching line: {hint}")
                                }
                            )));
                        }
                    }
                }

                // Batch path: exact-only multi-span, matched against ORIGINAL content_lf,
                // applied reverse-order so offsets stay stable. Reuses the same mutation
                // queue that serializes competing writes — no new concurrency surface.
                // Each old_string must be unique (exactly-once) or the batch fails.
                let mut spans: Vec<(usize, usize, String)> = Vec::with_capacity(batch.len());
                let mut seen_old: std::collections::HashSet<String> = std::collections::HashSet::new();
                for (idx, (old_s, new_s)) in batch.iter().enumerate() {
                    if !seen_old.insert(old_s.clone()) {
                        return Err(ToolFailure::msg(format!(
                            "edits[{}].old_string duplicates an earlier edit: merge overlapping edits",
                            idx
                        )));
                    }
                    let old_lf = crate::edit_match::normalize_to_lf(old_s);
                    let (count, first) =
                        crate::edit_match::find_exact_occurrences(&content_lf, &old_lf);
                    if count == 0 {
                        let first_line = old_lf.lines().next().unwrap_or("");
                        let hint = content_lf
                            .lines()
                            .find(|l| !first_line.is_empty() && l.contains(first_line.trim()))
                            .unwrap_or("");
                        return Err(ToolFailure::msg(format!(
                            "edits[{}].old_string not found in {}{}. A batch matches exactly — \
                             send this one on its own to use the fuzzy-match stages.",
                            idx,
                            args_path,
                            if hint.is_empty() {
                                String::new()
                            } else {
                                format!(" (closest line: {hint})")
                            }
                        )));
                    }
                    if count != 1 {
                        return Err(ToolFailure::msg(format!(
                            "edits[{}].old_string matches {count} locations in {}; widen the context",
                            idx, args_path
                        )));
                    }
                    let start = first.unwrap();
                    let end = start + old_lf.len();
                    let new_lf = crate::edit_match::normalize_to_lf(new_s);
                    spans.push((start, end, new_lf));
                }
                // Order by start ascending, reject overlap.
                spans.sort_by_key(|(s, _, _)| *s);
                for w in spans.windows(2) {
                    let (_, e0, _) = w[0];
                    let (s1, _, _) = w[1];
                    if s1 < e0 {
                        return Err(ToolFailure::msg(
                            "edits overlap: two old_string regions intersect in the original file — merge them into one edit",
                        ));
                    }
                }
                // Apply reverse so later offsets don't shift earlier ones.
                let mut new_content = content_lf.clone();
                for (s, e, repl) in spans.into_iter().rev() {
                    new_content.replace_range(s..e, &repl);
                }
                if new_content == content_lf {
                    return Err(ToolFailure::msg(format!(
                        "No changes made to {}; the replacements produced identical content",
                        args_path
                    )));
                }
                let updated = crate::edit_match::restore_eol(&new_content, eol);
                let diff = crate::util::unified_diff(&raw, &updated);
                std::fs::write(&path, &updated)
                    .map_err(|e| ToolFailure::msg(format!("write failed: {e}")))?;
                crate::read_log::record_write(&ctx.observation_scope, &path);
                let mut details = serde_json::json!({});
                if !diff.is_empty() {
                    details["diff"] = serde_json::Value::String(diff);
                }
                let mut text = format!(
                    "edited {} ({} replacement(s))",
                    args_path,
                    batch.len()
                );
                text.push_str(&diagnostics_note(provider.as_ref(), &path).await);
                Ok(ToolOutput {
                    content: vec![epsilon_contracts::content::ContentBlock::text(text)],
                    details,
                    usage: None,
                    terminate: false,
                })
            }
        })
        .await
    }
}

/// Silence unused-import lint until Path gains a direct use in later milestones.
#[allow(dead_code)]
fn _path_witness(_: &Path) {}
