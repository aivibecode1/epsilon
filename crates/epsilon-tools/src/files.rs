//! File-manipulation tools: read, write, edit.

use std::io::Read as _;

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::util;

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
    /// 1-based first line to return (default 1). Past the end of the file is an error
    /// naming the offset and the file's line count (pi `read.ts:281-283`).
    #[serde(default)]
    pub offset: Option<usize>,
    /// Max lines to return (default: the page's line cap).
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
        let path = util::resolve_or_refuse(&ctx.cwd, &args.path)?;
        // Images ride as attachments (pi `createReadTool` image branch): magic bytes first,
        // so a `.png` that is really text still reads as text and a text file with image
        // bytes never parses as text. Only the head is probed (M95): reading the file whole
        // just to look at its leading magic bytes made every read O(file) in memory before
        // the paged path ran, which is the bound this milestone claims. Detection needs only
        // the head — every signature is in its first bytes — but when the head *is* an image
        // the whole file is read, because the attachment below needs the bytes.
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .map_err(|e| ToolFailure::msg(format!("cannot open {}: {e}", path.display())))?
            .take(4096)
            .read_to_end(&mut bytes)
            .map_err(|e| ToolFailure::msg(format!("read error: {e}")))?;
        if crate::image::detect_supported_image_mime(&bytes).is_some() {
            bytes = std::fs::read(&path)
                .map_err(|e| ToolFailure::msg(format!("cannot open {}: {e}", path.display())))?;
        }
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
        let page = util::read_page(&path, args.offset.unwrap_or(1), args.limit)?;

        // Number the page's lines. The split is on '\n' and not `str::lines()` because the
        // text holds *exactly* the lines the page covered: a page is allowed to end with a
        // blank line, and `lines()` would drop it (and its number) from the output.
        let rows: Vec<&str> = if page.first_line == 0 {
            Vec::new()
        } else {
            page.text.split('\n').collect()
        };
        let mut body = rows
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>5}| {line}", page.first_line + i))
            .collect::<Vec<_>>()
            .join("\n");
        if let Some((line, bytes)) = page.long_line {
            // pi `read.ts:297-301`: a page whose *first* line alone exceeds the byte budget is
            // not shown at all — not even the prefix paging would have to decode lossily — and
            // the model is pointed at the one tool that can stream it. `read_page` sets
            // `long_line` only for that first-line case, so the page's already-decoded lines are
            // never thrown away for a line that merely stops the page later on.
            body = format!(
                "[Line {line} is {}, exceeds {} limit. Use bash: sed -n '{line}p' {} | head -c {}]",
                util::format_size(bytes),
                util::format_size(util::MAX_READ_BYTES),
                args.path,
                util::MAX_READ_BYTES
            );
        } else if let Some(next) = page.next_offset {
            // The continuation notices are pi's own strings (`read.ts:306-317`), with the
            // offsets and counts this page actually produced. `total_lines` is `Some` whenever
            // `next_offset` is (a page that is not the first-line over-cap case always scans to
            // EOF), so the numbers are exact rather than the byte-prefix count the old notice
            // reported.
            let total = page.total_lines.unwrap_or(page.last_line);
            let notice = match page.truncated_by {
                util::PageBound::Lines => format!(
                    "[Showing lines {}-{} of {total}. Use offset={next} to continue.]",
                    page.first_line, page.last_line
                ),
                util::PageBound::Bytes => format!(
                    "[Showing lines {}-{} of {total} ({} limit). Use offset={next} to continue.]",
                    page.first_line,
                    page.last_line,
                    util::format_size(util::MAX_READ_BYTES)
                ),
                _ => format!(
                    "[{} more lines in file. Use offset={next} to continue.]",
                    total.saturating_sub(page.last_line)
                ),
            };
            // pi always attaches the notice with a blank separator — `outputText =
            // `${truncation.content}\n\n[...]`` (`read.ts:317`, and its user-limit branch at
            // :313-317, which is what a `limit: 0` read lands in: pi answers that one with the
            // notice alone, never with silence). So there is no empty-body special case.
            body.push_str("\n\n");
            body.push_str(&notice);
        }
        // What this read actually covered decides whether a later wholesale overwrite is
        // legitimate (`crate::read_log`): the lines it showed, whether it reached the end of
        // the file, and whether the byte budget alone is what stopped it. `long_line` means the
        // page's *first* line alone was over the budget and nothing was shown — the one window
        // no read of this tool can widen; every other page, including one a later over-cap line
        // stopped, records the lines it actually showed and stays partial until reads reach EOF.
        let coverage = match page.long_line {
            Some(_) => crate::read_log::Coverage::long_line(),
            None => {
                crate::read_log::Coverage::page(page.first_line, page.last_line, page.total_lines)
            }
        };
        crate::read_log::record_read(&ctx.observation_scope, &path, coverage);
        let mut details = serde_json::json!({});
        details["read"] = serde_json::json!({
            "offset": page.first_line.max(1),
            "first_line": page.first_line,
            "last_line": page.last_line,
            "total_lines": page.total_lines,
            "next_offset": page.next_offset,
            "truncated_by": page.truncated_by.as_str(),
        });
        Ok(ToolOutput {
            content: vec![epsilon_contracts::content::ContentBlock::text(body)],
            details,
            usage: None,
            terminate: false,
        })
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
    util::resolve_or_refuse(&ctx.cwd, raw)
}

/// Turn the `path` argument into a planner key (M72).
///
/// Lexical normalisation against the same base the tool will execute with, so two calls that
/// spell one file two ways are seen as one file. A missing or non-string `path` yields `None`:
/// the call keeps the conservative scheduling rather than being assumed independent.
///
/// Lexical, not resolved: a planner key is an identity, and an identity that changed
/// when a symlink was retargeted would let two calls onto the same real file be
/// scheduled as if they were independent.
fn path_key(arguments: &serde_json::Value, base: &std::path::Path) -> Option<std::path::PathBuf> {
    let raw = arguments.get("path")?.as_str()?;
    Some(util::lexically_normalize(base, raw).0)
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
                            crate::read_log::record_edit(&ctx.observation_scope, &path);
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
                crate::read_log::record_edit(&ctx.observation_scope, &path);
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
