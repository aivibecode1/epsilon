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

/// How many *other* files the project sweep may name after a write/edit
/// (opencode `tool/write.ts:18`: `MAX_PROJECT_DIAGNOSTICS_FILES = 5`).
const MAX_PROJECT_DIAGNOSTICS_FILES: usize = 5;

/// Ask the diagnostics provider about a file this tool just wrote, and return the block to append.
///
/// Three rules, all of them about not letting a linter break an edit that already happened: no
/// provider means no block; a file that cannot be re-read means no block; a provider that is
/// silent, broken or wrong means no block. The caller appends what comes back and nothing else
/// (M79; opencode `tool/edit.ts:196-211`).
///
/// Past the touched-file block — which keeps its exact shape — comes a capped project sweep
/// (opencode `tool/write.ts:74-90`): the same seam is asked about the rest of the workspace so
/// an edit that breaks a *dependent* surfaces it the way opencode's write does. Same file
/// always first (it is already above), then other broken files, one line each, at most
/// [`MAX_PROJECT_DIAGNOSTICS_FILES`] of them with a `(+N more)` tail. Silence stays silent: a
/// clean rest of the project appends nothing.
async fn diagnostics_note(
    provider: Option<&Arc<dyn DiagnosticsProvider>>,
    path: &std::path::Path,
    workspace: &std::path::Path,
) -> String {
    let Some(provider) = provider else {
        return String::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let mut out = match provider.after_change(path, &text).await {
        Some(block) => format!("\n{block}"),
        None => String::new(),
    };
    out.push_str(&project_sweep(provider, path, workspace).await);
    out
}

/// The rest of the workspace, through the same [`DiagnosticsProvider`] seam — no new provider
/// method (the trait stays one method; a sweep composes `after_change` over walked files).
///
/// Relevance ordering: the touched file's own directory first (an edit breaks neighbours most
/// often), then everything else; alphabetical inside each group so the cap hides the same files
/// every run. Unreadable files are skipped exactly like the touched file is (no block).
async fn project_sweep(
    provider: &Arc<dyn DiagnosticsProvider>,
    touched: &std::path::Path,
    workspace: &std::path::Path,
) -> String {
    let touched_rel = touched
        .strip_prefix(workspace)
        .ok()
        .map(|p| p.to_path_buf());
    let touched_dir = touched_rel
        .as_ref()
        .and_then(|r| r.parent().map(|p| p.to_path_buf()));
    let mut same: Vec<std::path::PathBuf> = Vec::new();
    let mut rest: Vec<std::path::PathBuf> = Vec::new();
    for rel in crate::util::walk_files(workspace) {
        if Some(rel.as_path()) == touched_rel.as_deref() || workspace.join(&rel) == touched {
            continue;
        }
        if touched_dir.as_deref() == rel.parent() {
            same.push(rel);
        } else {
            rest.push(rel);
        }
    }
    same.sort();
    rest.sort();
    // Bound the WORK, not just the display: one `after_change` per walked
    // file is a language-server roundtrip each (didOpen/didChange + publish
    // wait), so an unbounded sweep turns every edit in a big workspace into
    // minutes of diagnostics. Same-directory candidates go first, so the
    // nearest files — where an edit's ripple lands — are the ones checked.
    // Reports name the scope they cover; silence still means "nothing found
    // in scope", the pre-existing silence contract.
    const MAX_PROJECT_SWEEP_QUERY: usize = 25;
    let candidates: Vec<std::path::PathBuf> = same.into_iter().chain(rest).collect();
    let total_candidates = candidates.len();
    let queried: Vec<std::path::PathBuf> = candidates
        .into_iter()
        .take(MAX_PROJECT_SWEEP_QUERY)
        .collect();
    let unchecked = total_candidates.saturating_sub(queried.len());
    // ponytail: opencode reads one cached diagnostics map (free) where this
    // re-queries per file — the bound above is the ceiling that keeps edits
    // interactive. Upgrade trigger: measured per-edit latency on a real
    // server, at which point the seam grows a cached project-diagnostics
    // query instead of more caps here.
    let mut broken: Vec<(String, String)> = Vec::new();
    for rel in queried {
        let abs = workspace.join(&rel);
        let Ok(text) = std::fs::read_to_string(&abs) else {
            continue;
        };
        if let Some(block) = provider.after_change(&abs, &text).await {
            let summary = block.lines().next().unwrap_or("").trim().to_string();
            if summary.is_empty() {
                continue;
            }
            broken.push((rel.to_string_lossy().replace('\\', "/"), summary));
        }
    }
    if broken.is_empty() {
        return String::new();
    }
    let total = broken.len();
    let mut out = format!(
        "\n\nLSP errors in {total} other {}:",
        if total == 1 { "file" } else { "files" }
    );
    for (name, summary) in broken.iter().take(MAX_PROJECT_DIAGNOSTICS_FILES) {
        out.push_str(&format!("\n{name}: {summary}"));
    }
    if total > MAX_PROJECT_DIAGNOSTICS_FILES {
        out.push_str(&format!(
            "\n(+{} more)",
            total - MAX_PROJECT_DIAGNOSTICS_FILES
        ));
    }
    if unchecked > 0 {
        out.push_str(&format!(
            "\n(sweep checked the nearest {queried} of {total_candidates} files)",
            queried = MAX_PROJECT_SWEEP_QUERY.min(total_candidates),
        ));
    }
    out
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

/// Suggest up to 3 near-miss names for a file that is not there
/// (opencode `read.ts:76-99`: substring match in both directions, case-insensitive).
///
/// Sorted for stable output (opencode uses readdir order); an exact match is
/// excluded so an existing file never "suggests" itself. An unreadable or
/// missing parent yields none — a miss with no neighbours is just a miss.
pub(crate) fn miss_suggestions(missing: &std::path::Path) -> Vec<String> {
    let Some(parent) = missing.parent() else {
        return Vec::new();
    };
    let base = missing
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if base.is_empty() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| {
            let lower = name.to_lowercase();
            lower != base && (lower.contains(base.as_str()) || base.contains(lower.as_str()))
        })
        .map(|name| parent.join(&name).display().to_string())
        .collect();
    out.sort();
    out.truncate(3);
    out
}

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
            .map_err(|e| {
                let mut msg = format!("cannot open {}: {e}", path.display());
                if e.kind() == std::io::ErrorKind::NotFound {
                    let suggestions = miss_suggestions(&path);
                    if !suggestions.is_empty() {
                        msg.push_str(&format!(
                            "\n\nDid you mean one of these?\n{}",
                            suggestions.join("\n")
                        ));
                    }
                }
                ToolFailure::msg(msg)
            })?
            .take(4096)
            .read_to_end(&mut bytes)
            .map_err(|e| ToolFailure::msg(format!("read error: {e}")))?;
        if crate::image::detect_supported_image_mime(&bytes).is_some()
            || crate::image::is_pdf(&bytes)
        {
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
            // Shrink toward the model's input limits (pi `read.ts:113-118`):
            // the fallback caps ARE the limits here — `ToolContext` carries no
            // model metadata for per-model inputLimits, so
            // `default_resize_options` is both the fallback and the policy.
            let resized = crate::image::resize_to_limits(
                &bytes,
                mime,
                &crate::image::default_resize_options(),
            );
            let mut header = format!("Read image file [{}]", resized.mime);
            if resized.mime != mime {
                // A resized JPEG is re-encoded as PNG (pi's conversion hint,
                // `image-process.ts:67-70`).
                header.push_str(&format!(
                    "\n[Image converted from {mime} to {}.]",
                    resized.mime
                ));
            }
            if let Some(note) = resized.dimension_note() {
                header.push('\n');
                header.push_str(&note);
            }
            return Ok(ToolOutput {
                content: vec![
                    epsilon_contracts::content::ContentBlock::text(header),
                    epsilon_contracts::content::ContentBlock::Image(
                        epsilon_contracts::content::ImageRef {
                            mime: resized.mime,
                            data: base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                &resized.bytes,
                            ),
                        },
                    ),
                ],
                details: serde_json::json!({}),
                usage: None,
                terminate: false,
            });
        }
        if crate::image::is_pdf(&bytes) {
            // opencode `read.ts:303-324`: a PDF rides as a data-URL attachment —
            // the same `ContentBlock` shape as the image branch above. Honest
            // boundary (verified against every provider wire, not assumed):
            // wires currently forward only `Text` from tool results, so the
            // model does not see these bytes yet — they are stored and
            // transcript-visible, ready for the wire-forwarding work that
            // makes them model-visible. Ungated like the image branch:
            // `ToolContext` has no model handle for pi's getNonVisionImageNote
            // capability check (`read.ts:59-64`), so there is nothing to gate on.
            return Ok(ToolOutput {
                content: vec![
                    epsilon_contracts::content::ContentBlock::text(
                        "Read PDF file [application/pdf]",
                    ),
                    epsilon_contracts::content::ContentBlock::Image(
                        epsilon_contracts::content::ImageRef {
                            mime: "application/pdf".into(),
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
        if crate::image::is_animated_png(&bytes) {
            // Animated PNG is detected but not sendable (animation frames break the
            // still-image contract) — say so explicitly instead of refusing as
            // "binary", mirroring the BMP arm above.
            return Ok(ToolOutput::text(
                "Read image file [image/png]\n[Image omitted: animated PNG images are not convertible in this build.]",
            ));
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

/// Read `value` as edit arguments, accepting the pi wire shape where `edits` arrives as a
/// JSON string.
///
/// **One implementation, two callers.** `execute` parses it this way and so must the
/// permission dialog's preview: a preview that parsed the arguments differently from the
/// tool would describe a different change from the one that lands, which is worse than
/// describing none.
fn parse_edit_args(value: serde_json::Value) -> Result<EditArgs, ToolFailure> {
    let mut value = value;
    // Pi parity (`prepareEditArguments`): some models send `edits` as a JSON string instead of
    // an array. Parse it in place; on failure leave it and let schema validation report the
    // shape error.
    if let Some(edits_str) = value.get("edits").and_then(|v| v.as_str())
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(edits_str)
    {
        value["edits"] = parsed;
    }
    serde_json::from_value(value)
        .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'edit': {e}")))
}

/// Normalise the two accepted argument shapes into one batch of `(old, new)` pairs.
///
/// The same validation `execute` performs, so a preview can only describe a call that would
/// actually be attempted.
fn edit_batch(args: &EditArgs) -> Result<Vec<(String, String)>, ToolFailure> {
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
    Ok(batch)
}

/// Apply a multi-edit batch to `content_lf`, or say why it cannot be applied.
///
/// **One implementation, two callers** — `execute`'s batch path and the preview a permission
/// dialog shows. They must agree byte for byte: a preview describing a different change from
/// the one that would land is worse than no preview, because the approver read it. So the
/// matching lives here rather than being copied, and both callers run this.
///
/// Exact-only, matched against the ORIGINAL content, applied reverse-order so offsets stay
/// stable. That is the batch rule the tool already enforces: a batch matches exactly, and a
/// call that needs the fuzzy stages goes on its own, where [`crate::edit_match::plan`] owns
/// them.
fn apply_batch_exact(
    content_lf: &str,
    batch: &[(String, String)],
    args_path: &str,
) -> Result<String, ToolFailure> {
    let mut spans: Vec<(usize, usize, String)> = Vec::with_capacity(batch.len());
    let mut seen_old: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (idx, (old_s, new_s)) in batch.iter().enumerate() {
        if !seen_old.insert(old_s.clone()) {
            return Err(ToolFailure::msg(format!(
                "edits[{idx}].old_string duplicates an earlier edit: merge overlapping edits"
            )));
        }
        let old_lf = crate::edit_match::normalize_to_lf(old_s);
        let (count, first) = crate::edit_match::find_exact_occurrences(content_lf, &old_lf);
        if count == 0 {
            let first_line = old_lf.lines().next().unwrap_or("");
            let hint = content_lf
                .lines()
                .find(|l| !first_line.is_empty() && l.contains(first_line.trim()))
                .unwrap_or("");
            return Err(ToolFailure::msg(format!(
                "edits[{idx}].old_string not found in {args_path}{}. A batch matches exactly — \
                 send this one on its own to use the fuzzy-match stages.",
                if hint.is_empty() {
                    String::new()
                } else {
                    format!(" (closest line: {hint})")
                }
            )));
        }
        if count != 1 {
            return Err(ToolFailure::msg(format!(
                "edits[{idx}].old_string matches {count} locations in {args_path}; widen the context"
            )));
        }
        let start = first.unwrap();
        let end = start + old_lf.len();
        spans.push((start, end, crate::edit_match::normalize_to_lf(new_s)));
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
    let mut new_content = content_lf.to_string();
    for (s, e, repl) in spans.into_iter().rev() {
        new_content.replace_range(s..e, &repl);
    }
    if new_content == content_lf {
        return Err(ToolFailure::msg(format!(
            "No changes made to {args_path}; the replacements produced identical content"
        )));
    }
    Ok(new_content)
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

    fn preview(&self, arguments: &serde_json::Value, base: &std::path::Path) -> Option<String> {
        let args: WriteArgs = serde_json::from_value(arguments.clone()).ok()?;
        let path = util::resolve_or_refuse(base, &args.path).ok()?;
        // A file that is not there yet has no current bytes, and the honest diff for it is
        // every line added — which is exactly what `unified_diff` produces from an empty old
        // side, so a new file needs no special case and no invented "(new file)" marker.
        //
        // A file that **is** there but cannot be read — binary, a permission failure, over
        // the edit byte cap — has no honest preview at all. Returning `None` keeps the
        // dialog on the one-line `what` it has always shown rather than claiming "(new file)"
        // for a file that plainly exists and would be overwritten. A reader who sees no diff
        // is in no worse a position than before this seam existed; a reader who sees a lie is.
        let current = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(_) => return None,
        };
        let diff = util::unified_diff(&current, &args.content);
        // Content identical to what is already there is not a change, and a diff of nothing
        // would be an empty frame where the reader expects the bytes they are approving.
        (!diff.is_empty()).then_some(diff)
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
                // Now that the bytes have landed, record what we left behind. Without this the
                // pre-image is the only hash on the entry, so the restore guard compares the
                // file against the state *before* this write and refuses every time — which is
                // what made `/restore` unusable for edits rather than merely untested.
                crate::checkpoint::CheckpointStore::open(&workspace)
                    .amend_with_post_image(&workspace, &path, "write");
                crate::read_log::record_write(&ctx.observation_scope, &path);
                let mut text = format!(
                    "{} {} ({} bytes)",
                    if existed { "overwrote" } else { "created" },
                    display,
                    content.len()
                );
                text.push_str(&diagnostics_note(provider.as_ref(), &path, &workspace).await);
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

    fn preview(&self, arguments: &serde_json::Value, base: &std::path::Path) -> Option<String> {
        let args: EditArgs = parse_edit_args(arguments.clone()).ok()?;
        let batch: Vec<(String, String)> = edit_batch(&args).ok()?;
        let path = util::resolve_or_refuse(base, &args.path).ok()?;
        // The same read `execute` makes, for the same reason: a preview built on a buffer
        // the tool itself would refuse (truncated, lossy, binary) would describe a change
        // that cannot land.
        let raw = util::read_text_for_edit(&path).ok()?;
        let eol = crate::edit_match::detect_eol(&raw);
        let content_lf = crate::edit_match::normalize_to_lf(&raw);
        // **The preview runs the matching the tool runs.** One edit — whether it arrived as
        // `edits:[one]` or as the legacy `old_string`/`new_string` — goes through
        // `edit_match::plan`, exactly as `execute` does, so the fuzzy stages are represented
        // rather than guessed at. A batch of more than one goes through `apply_batch_exact`,
        // which is the same function `execute`'s batch path calls.
        //
        // Every refusal returns `None`, not an approximation: "not found", "ambiguous", and
        // "suspicious" all mean the change that would land is not the one being described, so
        // the dialog falls back to the one-line `what` instead of showing a diff that will not
        // come true. A call the tool would refuse outright (missing file, unreadable text)
        // is the same answer.
        let updated_lf = if batch.len() == 1 {
            let (old, new) = &batch[0];
            match crate::edit_match::plan(
                &content_lf,
                &crate::edit_match::normalize_to_lf(old),
                &crate::edit_match::normalize_to_lf(new),
                args.replace_all,
            ) {
                crate::edit_match::EditOutcome::Applied { output, .. } => output,
                _ => return None,
            }
        } else {
            apply_batch_exact(&content_lf, &batch, &args.path).ok()?
        };
        let updated = crate::edit_match::restore_eol(&updated_lf, eol);
        let diff = util::unified_diff(&raw, &updated);
        (!diff.is_empty()).then_some(diff)
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: EditArgs = parse_edit_args(call.args)?;
        let batch: Vec<(String, String)> = edit_batch(&args)?;
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
                            // Same reason as the `write` tool: the entry's only hash so far is
                            // the pre-image, and the restore guard has to be able to tell this
                            // harness's edit from the user's own.
                            crate::checkpoint::CheckpointStore::open(&workspace)
                                .amend_with_post_image(&workspace, &path, "edit");
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
                            text.push_str(&diagnostics_note(provider.as_ref(), &path, &workspace).await);
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
                         indentation-flexible, escape-normalized, blank-line-trimmed, \
                         whitespace-normalized, context-anchor, and anchor-similarity matching all failed).{}",
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
                let new_content = apply_batch_exact(&content_lf, &batch, &args_path)?;
                let updated = crate::edit_match::restore_eol(&new_content, eol);
                let diff = crate::util::unified_diff(&raw, &updated);
                std::fs::write(&path, &updated)
                    .map_err(|e| ToolFailure::msg(format!("write failed: {e}")))?;
                // Third of the three write sites: the batch path. Same reason as the other two.
                crate::checkpoint::CheckpointStore::open(&workspace)
                    .amend_with_post_image(&workspace, &path, "edit");
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
                text.push_str(&diagnostics_note(provider.as_ref(), &path, &workspace).await);
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

#[cfg(test)]
mod miss_tests {
    use super::ReadTool;
    use super::miss_suggestions;
    use epsilon_contracts::tool::{Tool, ToolContext, ToolInvocation};
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn ctx_for(root: &std::path::Path) -> ToolContext {
        ToolContext {
            session_id: "miss-test".into(),
            observation_scope: Arc::new(()),
            cwd: root.to_path_buf(),
            spill_dir: root.join(".epsilon/spill"),
            token: CancellationToken::new(),
            task_host: None,
            goal_host: None,
        }
    }

    #[test]
    fn miss_suggestions_name_the_near_miss() {
        let dir = tempfile::tempdir().unwrap();
        for n in [
            "config.toml",
            "configuration.md",
            "other.rs",
            "unrelated.txt",
        ] {
            std::fs::write(dir.path().join(n), "x").unwrap();
        }
        let got = miss_suggestions(&dir.path().join("confi"));
        assert!(!got.is_empty(), "a near-miss must suggest");
        assert!(got.len() <= 3, "at most three suggestions: {got:?}");
        assert!(
            got.iter().any(|s| s.contains("config.toml")),
            "the near-miss is named: {got:?}"
        );
    }

    #[test]
    fn exact_hit_gives_no_suggestions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "x").unwrap();
        assert!(
            miss_suggestions(&dir.path().join("config.toml")).is_empty(),
            "an exact hit suggests nothing"
        );
    }

    #[test]
    fn empty_dir_gives_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            miss_suggestions(&dir.path().join("anything.rs")).is_empty(),
            "an empty dir suggests nothing"
        );
    }

    #[tokio::test]
    async fn read_error_names_the_near_miss() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "x = 1\n").unwrap();
        let tool = ReadTool::new();
        let call = ToolInvocation {
            tool_call_id: "miss-test".into(),
            // A true substring of the neighbour (opencode matches substrings).
            args: serde_json::json!({"path": "onfig.toml"}),
        };
        let err = tool
            .execute(call, ctx_for(dir.path()), Arc::new(|_| {}))
            .await
            .expect_err("a missing file must fail");
        assert!(
            err.message.contains("Did you mean"),
            "the error proposes names; got: {}",
            err.message
        );
        assert!(
            err.message.contains("config.toml"),
            "the near-miss is named; got: {}",
            err.message
        );
    }
}

#[cfg(test)]
mod attach_tests {
    use super::ReadTool;
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::tool::{Tool, ToolContext, ToolInvocation};
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn ctx_for(root: &std::path::Path) -> ToolContext {
        ToolContext {
            session_id: "attach-test".into(),
            observation_scope: Arc::new(()),
            cwd: root.to_path_buf(),
            spill_dir: root.join(".epsilon/spill"),
            token: CancellationToken::new(),
            task_host: None,
            goal_host: None,
        }
    }

    fn read_call(name: &str) -> ToolInvocation {
        ToolInvocation {
            tool_call_id: "attach-test".into(),
            args: serde_json::json!({"path": name}),
        }
    }

    #[tokio::test]
    async fn read_attaches_pdf_as_data_url_attachment() {
        // opencode `read.ts:303-324`: a PDF rides as a data-URL attachment, the
        // same shape as the image branch. The first block names the file, the
        // second carries the bytes.
        let dir = tempfile::tempdir().unwrap();
        let pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n".to_vec();
        std::fs::write(dir.path().join("doc.pdf"), &pdf).unwrap();
        let tool = ReadTool::new();
        let out = tool
            .execute(read_call("doc.pdf"), ctx_for(dir.path()), Arc::new(|_| {}))
            .await
            .unwrap();
        assert_eq!(out.content.len(), 2, "header + attachment: {out:?}");
        assert_eq!(
            out.content[0],
            ContentBlock::text("Read PDF file [application/pdf]")
        );
        let ContentBlock::Image(img) = &out.content[1] else {
            panic!(
                "second block must be the attachment, got {:?}",
                out.content[1]
            );
        };
        assert_eq!(img.mime, "application/pdf");
        assert_eq!(
            img.data,
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pdf)
        );
    }

    #[tokio::test]
    async fn read_still_refuses_non_pdf_binary() {
        // Guard pin: bytes that are neither image nor PDF keep the honest
        // binary refusal — no attachment, no decoded garbage.
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = vec![0x00, 0x01, 0x02, 0x03];
        bytes.extend(b"definitely not a document\r\n");
        bytes.push(0x00);
        std::fs::write(dir.path().join("blob.bin"), &bytes).unwrap();
        let tool = ReadTool::new();
        let err = tool
            .execute(read_call("blob.bin"), ctx_for(dir.path()), Arc::new(|_| {}))
            .await
            .expect_err("binary must still be refused");
        assert!(
            err.message.contains("refused: file looks binary"),
            "the binary refusal survives: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn read_shrinks_an_oversized_png_toward_limits() {
        // 3000px wide is over the fallback cap: the attached bytes must decode
        // to the shrunk dimensions and the header must say what changed.
        let dir = tempfile::tempdir().unwrap();
        let img = image::DynamicImage::ImageRgb8(image::ImageBuffer::from_fn(3000, 100, |x, y| {
            image::Rgb([(x % 251) as u8, (y % 251) as u8, 77u8])
        }));
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        std::fs::write(dir.path().join("wide.png"), &bytes).unwrap();
        let tool = ReadTool::new();
        let out = tool
            .execute(read_call("wide.png"), ctx_for(dir.path()), Arc::new(|_| {}))
            .await
            .unwrap();
        assert_eq!(out.content.len(), 2, "header + attachment: {out:?}");
        let ContentBlock::Image(attached) = &out.content[1] else {
            panic!("second block must be the image, got {:?}", out.content[1]);
        };
        let raw =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &attached.data)
                .unwrap();
        let decoded = image::load_from_memory(&raw).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2000, 67));
        let ContentBlock::Text { text } = &out.content[0] else {
            panic!("first block must be the header");
        };
        assert!(
            text.contains("original 3000x100"),
            "the header names the pre-shrink size: {text}"
        );
    }
}

#[cfg(test)]
mod sweep_tests {
    use super::*;
    use epsilon_contracts::tool::{Tool, ToolContext, ToolInvocation};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    /// Canned project diagnostics: file name -> block, consulted for every file the sweep
    /// asks about — the shape of opencode's `lsp.diagnostics()` map
    /// (`references/opencode-1.18.34/packages/opencode/src/tool/write.ts:76`), served
    /// through this harness's per-file `after_change` seam.
    struct CannedProject {
        blocks: HashMap<String, String>,
    }

    impl CannedProject {
        fn of(pairs: &[(&str, &str)]) -> Self {
            Self {
                blocks: pairs
                    .iter()
                    .map(|(n, b)| ((*n).to_string(), (*b).to_string()))
                    .collect(),
            }
        }
    }

    #[async_trait::async_trait]
    impl DiagnosticsProvider for CannedProject {
        async fn after_change(&self, path: &std::path::Path, _text: &str) -> Option<String> {
            path.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| self.blocks.get(n))
                .cloned()
        }
    }

    fn ctx_for(root: &std::path::Path) -> ToolContext {
        ToolContext {
            session_id: "sweep-test".into(),
            observation_scope: Arc::new(()),
            cwd: root.to_path_buf(),
            spill_dir: root.join(".epsilon/spill"),
            token: CancellationToken::new(),
            task_host: None,
            goal_host: None,
        }
    }

    async fn run_text(
        tool: &dyn Tool,
        ctx: &ToolContext,
        args: serde_json::Value,
    ) -> Result<String, String> {
        let sink: ToolUpdateSink = Arc::new(|_| {});
        tool.execute(
            ToolInvocation {
                tool_call_id: "sweep".into(),
                args,
            },
            ctx.clone(),
            sink,
        )
        .await
        .map(|o| {
            o.content
                .iter()
                .filter_map(|c| match c {
                    epsilon_contracts::content::ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .map_err(|f| f.message)
    }

    #[tokio::test]
    async fn write_surfaces_a_second_broken_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/other.rs"), "fn other() {}\n").unwrap();
        let ctx = ctx_for(root);
        let write = WriteTool::new().with_diagnostics(Arc::new(CannedProject::of(&[
            ("main.rs", "[lsp] main.rs: 1 error\n  ERROR [1:1] boom"),
            (
                "other.rs",
                "[lsp] other.rs: 1 error\n  ERROR [2:3] broken dep",
            ),
        ])));
        let text = run_text(
            &write,
            &ctx,
            serde_json::json!({"path": "src/main.rs", "content": "fn main() {}\n"}),
        )
        .await
        .expect("the write succeeds whether or not the linter has news");
        assert!(text.contains("created src/main.rs"), "{text}");
        assert!(
            text.contains("ERROR [1:1] boom"),
            "the touched-file block keeps its format: {text}"
        );
        assert!(
            text.contains("other.rs"),
            "the sweep names the second broken file: {text}"
        );
        assert!(
            text.contains("other file"),
            "the sweep groups the rest under one header: {text}"
        );
        assert!(
            text.find("boom").unwrap() < text.find("other file").unwrap(),
            "same file first, sweep after: {text}"
        );
    }

    #[tokio::test]
    async fn clean_project_keeps_only_the_touched_block() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/other.rs"), "fn other() {}\n").unwrap();
        let ctx = ctx_for(root);
        let write = WriteTool::new().with_diagnostics(Arc::new(CannedProject::of(&[(
            "main.rs",
            "[lsp] main.rs: 1 error\n  ERROR [1:1] boom",
        )])));
        let text = run_text(
            &write,
            &ctx,
            serde_json::json!({"path": "src/main.rs", "content": "fn main() {}\n"}),
        )
        .await
        .expect("the write succeeds");
        assert!(
            text.contains("ERROR [1:1] boom"),
            "the touched-file block is still there: {text}"
        );
        assert!(
            !text.contains("other file"),
            "no sweep section when the rest of the project is clean: {text}"
        );
    }

    #[tokio::test]
    async fn sweep_caps_at_five_other_files_with_a_count_note() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        for n in ["a", "b", "c", "d", "e", "f"] {
            std::fs::write(root.join(format!("src/{n}.rs")), "fn x() {}\n").unwrap();
        }
        let ctx = ctx_for(root);
        let pairs: Vec<(String, String)> = std::iter::once((
            "main.rs".to_string(),
            "[lsp] main.rs: 1 error\n  ERROR [1:1] boom".to_string(),
        ))
        .chain(
            ["a", "b", "c", "d", "e", "f"]
                .iter()
                .map(|n| (format!("{n}.rs"), format!("[lsp] {n}.rs: 1 error"))),
        )
        .collect();
        let provider = CannedProject {
            blocks: pairs.into_iter().collect(),
        };
        let write = WriteTool::new().with_diagnostics(Arc::new(provider));
        let text = run_text(
            &write,
            &ctx,
            serde_json::json!({"path": "src/main.rs", "content": "fn main() {}\n"}),
        )
        .await
        .expect("the write succeeds");
        assert!(
            text.contains("LSP errors in 6 other files:"),
            "the header counts every broken other file: {text}"
        );
        for n in ["a", "b", "c", "d", "e"] {
            assert!(
                text.contains(&format!("src/{n}.rs")),
                "the first five stay visible: {text}"
            );
        }
        assert!(
            text.contains("(+1 more)"),
            "the cap hides the sixth behind a count note: {text}"
        );
        assert!(
            !text.contains("f.rs"),
            "the sixth file is named nowhere: {text}"
        );
    }

    /// The sweep bounds its WORK, not just its display: past 25 candidates
    /// the rest go unchecked and the report says so. RED-first: an
    /// unbounded sweep turns every edit in a big workspace into one LSP
    /// roundtrip per file.
    #[tokio::test]
    async fn sweep_checks_the_nearest_files_and_names_its_scope() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let names: Vec<String> = (0..30).map(|n| format!("f{n:02}.rs")).collect();
        for n in &names {
            std::fs::write(root.join("src").join(n), "fn x() {}\n").unwrap();
        }
        let ctx = ctx_for(root);
        let pairs: Vec<(String, String)> =
            std::iter::once(("main.rs".to_string(), "[lsp] main.rs: 1 error".to_string()))
                .chain(
                    names
                        .iter()
                        .map(|n| (n.clone(), format!("[lsp] {n}: 1 error"))),
                )
                .collect();
        let provider = CannedProject {
            blocks: pairs.into_iter().collect(),
        };
        let write = WriteTool::new().with_diagnostics(Arc::new(provider));
        let text = run_text(
            &write,
            &ctx,
            serde_json::json!({"path": "src/main.rs", "content": "fn main() {}\n"}),
        )
        .await
        .expect("the write succeeds");
        assert!(
            text.contains("(sweep checked the nearest 25 of 30 files)"),
            "the report names its bounded scope: {text}"
        );
        assert!(
            !text.contains("f29.rs"),
            "files past the bound are never queried: {text}"
        );
    }

    #[tokio::test]
    async fn a_failed_edit_shows_no_sweep() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("src/other.rs"), "fn other() {}\n").unwrap();
        let ctx = ctx_for(root);
        let read = ReadTool::new();
        run_text(&read, &ctx, serde_json::json!({"path": "src/lib.rs"}))
            .await
            .expect("the prior read satisfies the stale-read guard");
        let edit = EditTool::new().with_diagnostics(Arc::new(CannedProject::of(&[
            ("lib.rs", "[lsp] lib.rs: 1 error\n  ERROR [1:1] boom"),
            (
                "other.rs",
                "[lsp] other.rs: 1 error\n  ERROR [2:3] broken dep",
            ),
        ])));
        let err = run_text(
            &edit,
            &ctx,
            serde_json::json!({
                "path": "src/lib.rs",
                "old_string": "fn missing() {}",
                "new_string": "fn b() {}",
            }),
        )
        .await
        .expect_err("the edit itself must fail: the old text is not there");
        assert!(err.contains("old_string not found"), "{err}");
        assert!(!err.contains("other file"), "no success, no sweep: {err}");
        assert!(
            !err.contains("other.rs"),
            "the failure names no project file: {err}"
        );
    }
}
