//! Multi-file atomic patch tool: the opencode `apply_patch` envelope, Epsilon-flavoured.
//!
//! Envelope (verbatim, opencode `apply_patch.txt` + `patch/index.ts`):
//!
//! ```text
//! *** Begin Patch
//! *** Add File: <path>
//! +<line>
//! *** Update File: <path>
//! *** Move to: <new-path>
//! @@ <optional context>
//!  <context line>
//! -<old line>
//! +<new line>
//! *** End of File
//! *** Delete File: <path>
//! *** End Patch
//! ```
//!
//! What is deliberately NOT accepted: unified diffs (`---`/`+++`/`diff --git`)
//! and heredoc wrappers (`cat <<'EOF'` / `<<EOF`). Both are refused with text
//! that names the envelope instead of guessing — a patch that is parsed
//! differently from what the model meant is worse than no patch.
//!
//! Transaction (5 phases, all-or-nothing):
//!
//! 1. **parse** the envelope (unknown `***` headers fail with a line offset);
//! 2. **verify everything in memory** — resolve every path, read every file,
//!    derive every new content (Exact + LineTrimmed + WhitespaceNormalized,
//!    forward from the previous hunk) — before touching disk, so a bad file B
//!    leaves file A untouched;
//! 3. **checkpoint N pre-images** with `tool="apply_patch"` (one ledger entry
//!    per path, so one turn anchor covers N entries — no new anchor type);
//! 4. **apply under the global patch mutex** (`mutation::with_patch_mutex`),
//!    amending each file's post-image as its bytes land so a later failure
//!    can roll back through the existing `restore()`;
//! 5. **amend + advisory diagnostics** (language-server feedback rides along,
//!    never fails the patch).
//!
//! Any write failure rolls back every file already written via `restore()`,
//! disclosing per file whether it was restored or left with the reason it
//! could not be.
//!
//! Approval is single: capability `edit`, pattern the sorted comma-joined
//! relpaths (see `gate_subject`'s multi-path arm), preview the concatenated
//! per-file unified diffs capped through `truncate_output` (`None` when the
//! patch cannot be honestly previewed).
//!
//! ponytail: no new dep, no new anchor type, no second matcher — the three
//! match stages are closed-form string compares, and the diff/cap/spill/
//! checkpoint seams already exist.

use async_trait::async_trait;
use epsilon_contracts::diagnostics::DiagnosticsProvider;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{checkpoint::CheckpointStore, util};

/// Model-facing description, kept beside the tool (not inlined) like every other tool.
pub const APPLY_PATCH_DESC: &str = include_str!("../descriptions/apply_patch.txt");

/// Arguments: the single opencode envelope string.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApplyPatchArgs {
    /// The full patch text that describes all changes to be made.
    #[serde(rename = "patchText", alias = "patch_text")]
    pub patch_text: String,
}

/// One file operation in the envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParsedHunk {
    Add {
        path: String,
        contents: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        chunks: Vec<ParsedChunk>,
    },
}

/// One `@@` section inside an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedChunk {
    pub context: Option<String>,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    pub is_end_of_file: bool,
}

/// Which of the three allowed stages located a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatchStrategy {
    Exact,
    LineTrimmed,
    WhitespaceNormalized,
}

impl PatchStrategy {
    fn note(self) -> &'static str {
        match self {
            Self::Exact => "",
            Self::LineTrimmed => " (matched after ignoring trailing whitespace)",
            Self::WhitespaceNormalized => " (matched after ignoring whitespace differences)",
        }
    }
}

/// Collapse every whitespace run to one space (and trim): the only
/// normalization looser than line-trim this tool allows.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn lines_equal(a: &str, b: &str, strategy: PatchStrategy) -> bool {
    match strategy {
        PatchStrategy::Exact => a == b,
        PatchStrategy::LineTrimmed => a.trim_end() == b.trim_end(),
        PatchStrategy::WhitespaceNormalized => normalize_ws(a) == normalize_ws(b),
    }
}

const STRATEGIES: [PatchStrategy; 3] = [
    PatchStrategy::Exact,
    PatchStrategy::LineTrimmed,
    PatchStrategy::WhitespaceNormalized,
];

/// Forward search for `pattern` in `lines` from `start`, trying the EOF anchor
/// first when `eof` is set. Returns the match index and the strategy that won.
fn seek_sequence(
    lines: &[String],
    pattern: &[String],
    start: usize,
    eof: bool,
) -> Option<(usize, PatchStrategy)> {
    if pattern.is_empty() {
        return None;
    }
    for strategy in STRATEGIES {
        if eof && lines.len() >= pattern.len() {
            let from_end = lines.len() - pattern.len();
            if from_end >= start
                && pattern
                    .iter()
                    .enumerate()
                    .all(|(j, p)| lines_equal(&lines[from_end + j], p, strategy))
            {
                return Some((from_end, strategy));
            }
        }
        let mut i = start;
        while i + pattern.len() <= lines.len() {
            if pattern
                .iter()
                .enumerate()
                .all(|(j, p)| lines_equal(&lines[i + j], p, strategy))
            {
                return Some((i, strategy));
            }
            i += 1;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Envelope parser (verbatim opencode grammar, minus heredoc, plus offsets)
// ---------------------------------------------------------------------------

const BEGIN_PATCH: &str = "*** Begin Patch";
const END_PATCH: &str = "*** End Patch";
const ADD_FILE: &str = "*** Add File:";
const DELETE_FILE: &str = "*** Delete File:";
const UPDATE_FILE: &str = "*** Update File:";
const MOVE_TO: &str = "*** Move to:";
const END_OF_FILE: &str = "*** End of File";

/// Refuse unified-diff input with text that names the envelope.
fn refuse_unified_diff(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    for line in trimmed.lines().take(20) {
        let t = line.trim_start();
        if t.starts_with("diff --git ")
            || t.starts_with("--- ")
            || t.starts_with("+++ ")
            || t.starts_with("@@ -")
        {
            return Some(
                "refused: unified diffs are not accepted — send the *** Begin Patch envelope \
                 (*** Add File / *** Update File / *** Delete File with @@ hunks of ' '/-/+ lines) \
                 instead of a ---/+++/@@ diff"
                    .to_string(),
            );
        }
    }
    None
}

/// Refuse heredoc wrappers with text that names the envelope.
fn refuse_heredoc(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    // opencode `stripHeredoc` shape: optional `cat`, then `<<'MARKER'`, envelope, `MARKER`.
    let first = trimmed.lines().next().unwrap_or("");
    let f = first.trim();
    if f.starts_with("cat <<") || f.starts_with("<<") {
        return Some(
            "refused: heredoc wrapping (cat <<'EOF' / <<EOF) is not accepted — send the \
             *** Begin Patch envelope directly as patchText, without shell quoting"
                .to_string(),
        );
    }
    // A trailing lone marker line (the closing EOF) is the same wrapper.
    let lines: Vec<&str> = trimmed.lines().collect();
    if lines.len() >= 2 {
        let last = lines.last().unwrap_or(&"").trim();
        if !last.is_empty()
            && !last.starts_with("***")
            && !last.starts_with('+')
            && !last.starts_with('-')
            && !last.starts_with(' ')
            && !last.starts_with("@@")
            && lines.iter().any(|l| l.trim() == BEGIN_PATCH)
        {
            // Only fire when the last line looks like a bare heredoc terminator
            // (single word, no spaces) — a content line with spaces is not one.
            if !last.contains(' ') && !last.contains('\t') {
                return Some(
                    "refused: heredoc wrapping (trailing EOF marker) is not accepted — send the \
                     *** Begin Patch envelope directly as patchText, without shell quoting"
                        .to_string(),
                );
            }
        }
    }
    None
}

/// Parse the envelope into hunks, or say why it cannot be applied.
///
/// Errors carry a 1-based line number wherever the offense has one, so the
/// model can find the header it misspelled.
pub(crate) fn parse_patch(patch_text: &str) -> Result<Vec<ParsedHunk>, String> {
    if let Some(msg) = refuse_unified_diff(patch_text) {
        return Err(msg);
    }
    if let Some(msg) = refuse_heredoc(patch_text) {
        return Err(msg);
    }
    // Line numbers below are 1-based over the raw input (what the model sent),
    // so an offset points at the line it actually wrote.
    let raw_lines: Vec<String> = patch_text.split('\n').map(|l| l.to_string()).collect();
    // Markers match trimmed (opencode does), but offsets count raw lines.
    let begin = raw_lines.iter().position(|l| l.trim() == BEGIN_PATCH);
    let end = raw_lines.iter().position(|l| l.trim() == END_PATCH);
    let (begin, end) = match (begin, end) {
        (Some(b), Some(e)) if b < e => (b, e),
        _ => {
            return Err(
                "Invalid patch format: missing Begin/End markers — start with *** Begin Patch, \
                 end with *** End Patch"
                    .to_string(),
            );
        }
    };
    let mut hunks: Vec<ParsedHunk> = Vec::new();
    let mut i = begin + 1;
    while i < end {
        let line = raw_lines[i].clone();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix(ADD_FILE) {
            let path = rest.trim().to_string();
            if path.is_empty() {
                return Err(format!("line {}: *** Add File: names no path", i + 1));
            }
            let mut contents: Vec<String> = Vec::new();
            i += 1;
            while i < end && !raw_lines[i].starts_with("***") {
                let body = &raw_lines[i];
                if let Some(stripped) = body.strip_prefix('+') {
                    contents.push(stripped.to_string());
                } else if body.is_empty() {
                    // A truly empty line inside an Add section is an empty content
                    // line only when written as `+`; a bare blank line is ignored
                    // (it is the envelope's own formatting, not file content).
                } else {
                    return Err(format!(
                        "line {}: Add File content must prefix every line with `+`",
                        i + 1
                    ));
                }
                i += 1;
            }
            hunks.push(ParsedHunk::Add {
                path,
                contents: contents.join("\n"),
            });
        } else if let Some(rest) = line.strip_prefix(DELETE_FILE) {
            let path = rest.trim().to_string();
            if path.is_empty() {
                return Err(format!("line {}: *** Delete File: names no path", i + 1));
            }
            hunks.push(ParsedHunk::Delete { path });
            i += 1;
        } else if let Some(rest) = line.strip_prefix(UPDATE_FILE) {
            let path = rest.trim().to_string();
            if path.is_empty() {
                return Err(format!("line {}: *** Update File: names no path", i + 1));
            }
            i += 1;
            let mut move_to: Option<String> = None;
            if let Some(dest_rest) = raw_lines.get(i).and_then(|l| l.strip_prefix(MOVE_TO)) {
                let dest = dest_rest.trim().to_string();
                if dest.is_empty() {
                    return Err(format!("line {}: *** Move to: names no path", i + 1));
                }
                move_to = Some(dest);
                i += 1;
            }
            let mut chunks: Vec<ParsedChunk> = Vec::new();
            while i < end && !raw_lines[i].starts_with("***") {
                if let Some(ctx_rest) = raw_lines[i].strip_prefix("@@") {
                    let context = ctx_rest.trim().to_string();
                    i += 1;
                    let mut old_lines: Vec<String> = Vec::new();
                    let mut new_lines: Vec<String> = Vec::new();
                    let is_end_of_file = false;
                    while i < end
                        && !raw_lines[i].starts_with("@@")
                        && !raw_lines[i].starts_with("***")
                    {
                        // *** End of File arrives here (it starts with ***): the
                        // check below never fires — handle it in the outer loop.
                        let change = &raw_lines[i];
                        if let Some(stripped) = change.strip_prefix(' ') {
                            let content = stripped.to_string();
                            old_lines.push(content.clone());
                            new_lines.push(content);
                        } else if let Some(stripped) = change.strip_prefix('-') {
                            old_lines.push(stripped.to_string());
                        } else if let Some(stripped) = change.strip_prefix('+') {
                            new_lines.push(stripped.to_string());
                        } else if change.is_empty() {
                            // Blank separator between hunks (the shape opencode's
                            // "missing second chunk context" test pins): ignored,
                            // so the two sides merge into one non-contiguous
                            // pattern that fails verification rather than two
                            // hunks that silently half-apply.
                        } else {
                            return Err(format!(
                                "line {}: update hunks use only ' ' / '-' / '+' lines \
                                 (and @@ headers); found {change:?}",
                                i + 1
                            ));
                        }
                        i += 1;
                    }
                    chunks.push(ParsedChunk {
                        context: if context.is_empty() {
                            None
                        } else {
                            Some(context)
                        },
                        old_lines,
                        new_lines,
                        is_end_of_file,
                    });
                    // A trailing *** End of File belongs to the chunk just closed.
                    if i < end && raw_lines[i].trim() == END_OF_FILE {
                        if let Some(last) = chunks.last_mut() {
                            last.is_end_of_file = true;
                        }
                        i += 1;
                    }
                } else if raw_lines[i].trim() == END_OF_FILE {
                    // End-of-file anchor with no preceding @@ (marks an append
                    // point): attach to the previous chunk when there is one,
                    // otherwise remember it as an empty trailing chunk.
                    if let Some(last) = chunks.last_mut() {
                        last.is_end_of_file = true;
                    } else {
                        chunks.push(ParsedChunk {
                            context: None,
                            old_lines: Vec::new(),
                            new_lines: Vec::new(),
                            is_end_of_file: true,
                        });
                    }
                    i += 1;
                } else {
                    i += 1;
                }
            }
            hunks.push(ParsedHunk::Update {
                path,
                move_to,
                chunks,
            });
        } else if trimmed.starts_with("***") {
            return Err(format!(
                "line {}: unknown patch header {trimmed:?} — want *** Add File:, *** Update File: (with optional *** Move to:), or *** Delete File:",
                i + 1
            ));
        } else {
            i += 1;
        }
    }
    if hunks.is_empty() {
        let normalized = patch_text.replace("\r\n", "\n").replace('\r', "\n");
        let normalized = normalized.trim();
        if normalized == "*** Begin Patch\n*** End Patch" {
            return Err("patch rejected: empty patch".to_string());
        }
        return Err("apply_patch verification failed: no hunks found".to_string());
    }
    Ok(hunks)
}

/// Lightweight header scan for the permission gate (no filesystem, no reads).
///
/// `gate_subject` lives in the core, which must not depend on this crate, so
/// this helper's grammar (the four `***` prefixes) is duplicated there by
/// hand. Keep the prefixes identical — a header one side accepts and the
/// other ignores is a prompt that describes a different patch from the one
/// that runs.
pub(crate) fn relpaths_in_patch(patch_text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in patch_text.lines() {
        if let Some(rest) = line.strip_prefix(ADD_FILE) {
            let p = rest.trim();
            if !p.is_empty() {
                out.push(p.replace('\\', "/"));
            }
        } else if let Some(rest) = line.strip_prefix(DELETE_FILE) {
            let p = rest.trim();
            if !p.is_empty() {
                out.push(p.replace('\\', "/"));
            }
        } else if let Some(rest) = line.strip_prefix(UPDATE_FILE) {
            let p = rest.trim();
            if !p.is_empty() {
                out.push(p.replace('\\', "/"));
            }
        } else if let Some(rest) = line.strip_prefix(MOVE_TO) {
            let p = rest.trim();
            if !p.is_empty() {
                out.push(p.replace('\\', "/"));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------------------
// In-memory verification: derive every new content before touching disk
// ---------------------------------------------------------------------------

pub(crate) struct VerifiedFile {
    pub rel: String,
    pub abs: PathBuf,
    pub kind: &'static str,
    pub new_content: String,
    pub diff: String,
    pub strategies: Vec<PatchStrategy>,
    pub move_dest_rel: Option<String>,
    pub move_dest_abs: Option<PathBuf>,
}

fn display_rel(workspace: &Path, abs: &Path) -> String {
    abs.strip_prefix(workspace)
        .map(|r| r.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| abs.to_string_lossy().replace('\\', "/"))
}

/// Derive the new LF content for one update's chunks against LF `original`.
fn derive_update(
    file_label: &str,
    chunks: &[ParsedChunk],
    original_lf: &str,
) -> Result<(String, Vec<PatchStrategy>), String> {
    let mut original_lines: Vec<String> = original_lf.split('\n').map(|s| s.to_string()).collect();
    if original_lines.last().is_some_and(|l| l.is_empty()) {
        original_lines.pop();
    }
    // An update with no @@ hunks changes nothing — refuse rather than write
    // an identical file and record a checkpoint for it.
    if chunks
        .iter()
        .all(|c| c.old_lines.is_empty() && c.new_lines.is_empty())
    {
        return Err(format!(
            "apply_patch verification failed: {file_label} has no hunks to apply"
        ));
    }
    let mut replacements: Vec<(usize, usize, Vec<String>)> = Vec::new();
    let mut strategies: Vec<PatchStrategy> = Vec::new();
    let mut cursor = 0usize;
    for chunk in chunks {
        if let Some(ctx) = chunk.context.as_deref() {
            let found = seek_sequence(&original_lines, &[ctx.to_string()], cursor, false);
            let Some((at, _)) = found else {
                return Err(format!(
                    "apply_patch verification failed: Failed to find context '{ctx}' in {file_label}"
                ));
            };
            cursor = at + 1;
        }
        if chunk.old_lines.is_empty() {
            // Pure insertion (only `+` lines): append at the end, like opencode.
            if chunk.new_lines.is_empty() {
                continue;
            }
            let at = if original_lines.last().is_some_and(|l| l.is_empty()) {
                original_lines.len().saturating_sub(1)
            } else {
                original_lines.len()
            };
            replacements.push((at, 0, chunk.new_lines.clone()));
            // Insertions carry no strategy (nothing was matched).
            continue;
        }
        let mut pattern = chunk.old_lines.clone();
        let mut new_slice = chunk.new_lines.clone();
        let mut found = seek_sequence(&original_lines, &pattern, cursor, chunk.is_end_of_file);
        // Retry without a trailing empty line (opencode parity): a hunk whose
        // old side ends at EOF often carries the newline as content.
        if found.is_none() && pattern.last().is_some_and(|l| l.is_empty()) {
            pattern.pop();
            if new_slice.last().is_some_and(|l| l.is_empty()) {
                new_slice.pop();
            }
            found = seek_sequence(&original_lines, &pattern, cursor, chunk.is_end_of_file);
        }
        let Some((at, strategy)) = found else {
            return Err(format!(
                "apply_patch verification failed: Failed to find expected lines in {file_label}:\n{}",
                chunk.old_lines.join("\n")
            ));
        };
        strategies.push(strategy);
        replacements.push((at, pattern.len(), new_slice));
        cursor = at + pattern.len();
    }
    replacements.sort_by_key(|(at, _, _)| *at);
    // Overlapping replacements would apply twice to one region: refuse.
    let mut cover_end = 0usize;
    for (at, len, _) in &replacements {
        if *at < cover_end {
            return Err(format!(
                "apply_patch verification failed: overlapping hunks in {file_label}"
            ));
        }
        cover_end = at + len;
    }
    let mut new_lines = original_lines.clone();
    for (at, len, slice) in replacements.into_iter().rev() {
        new_lines.splice(at..at + len, slice);
    }
    if new_lines.last().is_none_or(|l| !l.is_empty()) {
        new_lines.push(String::new());
    }
    Ok((new_lines.join("\n"), strategies))
}

/// Verify the whole patch in memory: every path resolves, every read
/// succeeds, every hunk locates. Returns the derived contents; writes nothing.
pub(crate) fn verify_patch(
    workspace: &Path,
    hunks: &[ParsedHunk],
    observation_scope: &Arc<()>,
) -> Result<Vec<VerifiedFile>, ToolFailure> {
    let mut out: Vec<VerifiedFile> = Vec::new();
    for hunk in hunks {
        match hunk {
            ParsedHunk::Add { path, contents } => {
                let abs = util::resolve_or_refuse(workspace, path)?;
                if abs.is_dir() {
                    return Err(ToolFailure::msg(format!(
                        "apply_patch verification failed: {path} is a directory"
                    )));
                }
                // Overwriting an existing file through Add is still an overwrite:
                // the file must have been read (write's rule), and a file too
                // large or too binary to edit honestly cannot be overwritten
                // honestly either.
                let existed = abs.exists();
                if existed {
                    let raw = util::read_text_for_edit(&abs).map_err(|e| {
                        ToolFailure::msg(format!(
                            "apply_patch verification failed: cannot overwrite {path}: {}",
                            e.message
                        ))
                    })?;
                    let _ = raw;
                    crate::read_log::verify(observation_scope, &abs, true).map_err(|s| {
                        ToolFailure::msg(format!(
                            "apply_patch verification failed: {}",
                            s.diagnosis("apply_patch")
                        ))
                    })?;
                }
                let new_content = if contents.is_empty() {
                    String::new()
                } else {
                    format!("{contents}\n")
                };
                if existed {
                    let old_content = std::fs::read_to_string(&abs).map_err(|e| {
                        ToolFailure::msg(format!(
                            "apply_patch verification failed: cannot read {path}: {e}"
                        ))
                    })?;
                    if old_content == new_content {
                        return Err(ToolFailure::msg(format!(
                            "apply_patch verification failed: {path} already holds identical content — nothing to do"
                        )));
                    }
                    let diff = util::unified_diff(&old_content, &new_content);
                    out.push(VerifiedFile {
                        rel: display_rel(workspace, &abs),
                        abs,
                        kind: "add",
                        new_content,
                        diff,
                        strategies: Vec::new(),
                        move_dest_rel: None,
                        move_dest_abs: None,
                    });
                } else {
                    let diff = util::unified_diff("", &new_content);
                    out.push(VerifiedFile {
                        rel: display_rel(workspace, &abs),
                        abs,
                        kind: "add",
                        new_content,
                        diff,
                        strategies: Vec::new(),
                        move_dest_rel: None,
                        move_dest_abs: None,
                    });
                }
            }
            ParsedHunk::Delete { path } => {
                let abs = util::resolve_or_refuse(workspace, path)?;
                let raw = util::read_text_for_edit(&abs).map_err(|e| {
                    ToolFailure::msg(format!(
                        "apply_patch verification failed: cannot delete {path}: {}",
                        e.message
                    ))
                })?;
                if abs.is_dir() {
                    return Err(ToolFailure::msg(format!(
                        "apply_patch verification failed: {path} is a directory"
                    )));
                }
                crate::read_log::verify(observation_scope, &abs, false).map_err(|s| {
                    ToolFailure::msg(format!(
                        "apply_patch verification failed: {}",
                        s.diagnosis("apply_patch")
                    ))
                })?;
                let diff = util::unified_diff(&raw, "");
                out.push(VerifiedFile {
                    rel: display_rel(workspace, &abs),
                    abs,
                    kind: "delete",
                    new_content: String::new(),
                    diff,
                    strategies: Vec::new(),
                    move_dest_rel: None,
                    move_dest_abs: None,
                });
            }
            ParsedHunk::Update {
                path,
                move_to,
                chunks,
            } => {
                let abs = util::resolve_or_refuse(workspace, path)?;
                if abs.is_dir() {
                    return Err(ToolFailure::msg(format!(
                        "apply_patch verification failed: {path} is a directory"
                    )));
                }
                let raw = util::read_text_for_edit(&abs).map_err(|e| {
                    ToolFailure::msg(format!(
                        "apply_patch verification failed: Failed to read file to update: {path}: {}",
                        e.message
                    ))
                })?;
                crate::read_log::verify(observation_scope, &abs, false).map_err(|s| {
                    ToolFailure::msg(format!(
                        "apply_patch verification failed: {}",
                        s.diagnosis("apply_patch")
                    ))
                })?;
                let eol = crate::edit_match::detect_eol(&raw);
                let content_lf = crate::edit_match::normalize_to_lf(&raw);
                let (new_lf, strategies) =
                    derive_update(path, chunks, &content_lf).map_err(ToolFailure::msg)?;
                let new_content = crate::edit_match::restore_eol(&new_lf, eol);
                if new_content == raw {
                    return Err(ToolFailure::msg(format!(
                        "apply_patch verification failed: {path} already holds identical content — nothing to do"
                    )));
                }
                let diff = util::unified_diff(&raw, &new_content);
                if let Some(dest) = move_to {
                    let dest_abs = util::resolve_or_refuse(workspace, dest)?;
                    if dest_abs.is_dir() {
                        return Err(ToolFailure::msg(format!(
                            "apply_patch verification failed: {dest} is a directory"
                        )));
                    }
                    if dest_abs.exists() {
                        // Overwriting the destination destroys work never named
                        // in the hunk: it gets the write rule, not the edit one.
                        let _ = util::read_text_for_edit(&dest_abs).map_err(|e| {
                            ToolFailure::msg(format!(
                                "apply_patch verification failed: cannot overwrite move destination {dest}: {}",
                                e.message
                            ))
                        })?;
                        crate::read_log::verify(observation_scope, &dest_abs, true).map_err(
                            |s| {
                                ToolFailure::msg(format!(
                                    "apply_patch verification failed: {}",
                                    s.diagnosis("apply_patch")
                                ))
                            },
                        )?;
                    }
                    out.push(VerifiedFile {
                        rel: display_rel(workspace, &abs),
                        abs,
                        kind: "move",
                        new_content,
                        diff,
                        strategies,
                        move_dest_rel: Some(display_rel(workspace, &dest_abs)),
                        move_dest_abs: Some(dest_abs),
                    });
                } else {
                    out.push(VerifiedFile {
                        rel: display_rel(workspace, &abs),
                        abs,
                        kind: "update",
                        new_content,
                        diff,
                        strategies,
                        move_dest_rel: None,
                        move_dest_abs: None,
                    });
                }
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The tool
// ---------------------------------------------------------------------------

/// Advisory language-server feedback, never a failure (write/edit parity).
async fn diagnostics_note(provider: Option<&Arc<dyn DiagnosticsProvider>>, path: &Path) -> String {
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

/// Atomic multi-file patch tool.
pub struct ApplyPatchTool {
    descriptor: ToolDescriptor,
    diagnostics: Option<Arc<dyn DiagnosticsProvider>>,
}

impl ApplyPatchTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            diagnostics: None,
            descriptor: ToolDescriptor {
                name: "apply_patch".into(),
                label: "Apply patch".into(),
                description: APPLY_PATCH_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(ApplyPatchArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Sequential,
            },
        }
    }

    /// Attach a diagnostics provider: every successful patch then carries what
    /// the language server thinks of the result (advisory only).
    pub fn with_diagnostics(mut self, provider: Arc<dyn DiagnosticsProvider>) -> Self {
        self.diagnostics = Some(provider);
        self
    }
}

impl Default for ApplyPatchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    /// **The patch tool acts under the `edit` capability.**
    ///
    /// It writes files, so a rule the user wrote for `edit` covers it —
    /// including the ones the user wrote *before* the patch tool existed.
    /// Without this, `apply_patch` was its own capability: it matched no
    /// default rule, so it asked for permission on every call, and a rule for
    /// `edit` did not cover it.
    ///
    /// The patch tool still has its own *name*, so the prompt, the transcript
    /// and the session history all say `apply_patch`; only the permission
    /// identity is shared.
    fn capability(&self) -> &str {
        "edit"
    }

    fn resource_key(&self, _arguments: &serde_json::Value, _base: &Path) -> Option<PathBuf> {
        // Multi-file by construction: no single resource names it, so the
        // batch planner keeps the conservative barrier rather than pretending
        // two patches to disjoint file sets are independent (the global patch
        // mutex is what actually serializes them).
        None
    }

    fn preview(&self, arguments: &serde_json::Value, base: &Path) -> Option<String> {
        let args: ApplyPatchArgs = serde_json::from_value(arguments.clone()).ok()?;
        let hunks = parse_patch(&args.patch_text).ok()?;
        // Preview reads the current bytes and derives the diffs — the same
        // derivation `execute` runs. Anything that cannot be honestly shown
        // (missing file, binary, oversize, unlocatable hunk, identical
        // content) is `None`, so the dialog falls back to naming the files
        // rather than showing a diff that will not come true. No checkpoints,
        // no read-log entries, no writes: a preview is read-only.
        // A preview that cannot resolve a path inside the workspace has no
        // honest diff to show for that file.
        let mut parts: Vec<String> = Vec::new();
        for hunk in &hunks {
            match hunk {
                ParsedHunk::Add { path, contents } => {
                    let abs = util::resolve_or_refuse(base, path).ok()?;
                    if abs.is_dir() {
                        return None;
                    }
                    let new_content = if contents.is_empty() {
                        String::new()
                    } else {
                        format!("{contents}\n")
                    };
                    let old_content = std::fs::read_to_string(&abs).unwrap_or_default();
                    if old_content == new_content {
                        return None;
                    }
                    let diff = util::unified_diff(&old_content, &new_content);
                    if diff.is_empty() {
                        return None;
                    }
                    parts.push(format!("--- {path}\n{diff}"));
                }
                ParsedHunk::Delete { path } => {
                    let abs = util::resolve_or_refuse(base, path).ok()?;
                    let raw = util::read_text_for_edit(&abs).ok()?;
                    parts.push(format!("--- {path}\n{}", util::unified_diff(&raw, "")));
                }
                ParsedHunk::Update {
                    path,
                    move_to,
                    chunks,
                } => {
                    let abs = util::resolve_or_refuse(base, path).ok()?;
                    let raw = util::read_text_for_edit(&abs).ok()?;
                    let eol = crate::edit_match::detect_eol(&raw);
                    let content_lf = crate::edit_match::normalize_to_lf(&raw);
                    let (new_lf, _) = derive_update(path, chunks, &content_lf).ok()?;
                    let new_content = crate::edit_match::restore_eol(&new_lf, eol);
                    if new_content == raw {
                        return None;
                    }
                    let diff = util::unified_diff(&raw, &new_content);
                    if diff.is_empty() {
                        return None;
                    }
                    let label = match move_to {
                        Some(dest) => format!("{path} -> {dest}"),
                        None => path.clone(),
                    };
                    parts.push(format!("--- {label}\n{diff}"));
                }
            }
        }
        if parts.is_empty() {
            return None;
        }
        let joined = parts.join("\n");
        let spill_dir = base.join(".epsilon").join("spill");
        let t = util::truncate_output(&joined, &spill_dir, "apply_patch");
        Some(t.text)
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: ApplyPatchArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'apply_patch': {e}")))?;
        if args.patch_text.trim().is_empty() {
            return Err(ToolFailure::msg("patchText is required"));
        }
        let hunks = parse_patch(&args.patch_text).map_err(ToolFailure::msg)?;

        // Phase 2: verify EVERYTHING in memory before touching disk. A bad file
        // B fails here, and file A is untouched (its mtime and bytes prove it).
        let verified = verify_patch(&ctx.cwd, &hunks, &ctx.observation_scope)?;

        // Phase 3: checkpoint N pre-images (one turn anchor covers N entries).
        let workspace = ctx.cwd.clone();
        let store = CheckpointStore::open(&workspace);
        {
            let mut distinct: Vec<PathBuf> = Vec::new();
            for f in &verified {
                if !distinct.contains(&f.abs) {
                    distinct.push(f.abs.clone());
                }
                if let Some(dest) = &f.move_dest_abs
                    && !distinct.contains(dest)
                {
                    distinct.push(dest.clone());
                }
            }
            for abs in &distinct {
                store.record(&workspace, abs, "apply_patch");
            }
        }

        // Phase 4+5: apply under the global patch mutex, amending each
        // post-image as its bytes land (so a later failure can roll back
        // through the existing restore()), then advisory diagnostics.
        let provider = self.diagnostics.clone();
        let spill_dir = ctx.spill_dir.clone();
        let scope = ctx.observation_scope.clone();
        crate::mutation::with_patch_mutex(|| {
            let verified = &verified;
            let workspace = workspace.clone();
            let provider = provider.clone();
            let spill_dir = spill_dir.clone();
            let scope = scope.clone();
            async move {
                let store = CheckpointStore::open(&workspace);
                // Paths already applied, newest first for rollback.
                let mut applied: Vec<&VerifiedFile> = Vec::new();
                for f in verified {
                    let step: Result<(), String> = (|| match f.kind {
                        "add" | "update" => {
                            if let Some(parent) = f.abs.parent() {
                                std::fs::create_dir_all(parent)
                                    .map_err(|e| format!("cannot create parent dir: {e}"))?;
                            }
                            std::fs::write(&f.abs, &f.new_content)
                                .map_err(|e| format!("write failed for {}: {e}", f.rel))?;
                            store.amend_with_post_image(&workspace, &f.abs, "apply_patch");
                            crate::read_log::record_edit(&scope, &f.abs);
                            Ok(())
                        }
                        "delete" => {
                            std::fs::remove_file(&f.abs)
                                .map_err(|e| format!("remove failed for {}: {e}", f.rel))?;
                            Ok(())
                        }
                        "move" => {
                            let dest = f
                                .move_dest_abs
                                .as_ref()
                                .ok_or_else(|| format!("move {} names no destination", f.rel))?;
                            if let Some(parent) = dest.parent() {
                                std::fs::create_dir_all(parent)
                                    .map_err(|e| format!("cannot create parent dir: {e}"))?;
                            }
                            std::fs::write(dest, &f.new_content)
                                .map_err(|e| format!("write failed for {}: {e}", f.rel))?;
                            store.amend_with_post_image(&workspace, dest, "apply_patch");
                            crate::read_log::record_edit(&scope, dest);
                            std::fs::remove_file(&f.abs)
                                .map_err(|e| format!("remove failed for {}: {e}", f.rel))?;
                            Ok(())
                        }
                        other => Err(format!("unknown change kind {other}")),
                    })();
                    if let Err(why) = step {
                        // Roll back everything already written, newest first,
                        // through the existing restore(). Each file discloses
                        // whether it came back or why it could not.
                        let mut notes: Vec<String> = Vec::new();
                        for done in applied.iter().rev() {
                            notes.extend(rollback_one(&workspace, &store, done));
                        }
                        let mut msg = format!(
                            "apply_patch failed: {why}; rolled back {} file(s)",
                            applied.len()
                        );
                        if !notes.is_empty() {
                            msg.push_str(": ");
                            msg.push_str(&notes.join("; "));
                        }
                        return Err(ToolFailure::msg(msg));
                    }
                    applied.push(f);
                }

                // All N writes landed: summary + strategy notes + full diff.
                let mut summary: Vec<String> = Vec::new();
                for f in verified {
                    let note: String = f
                        .strategies
                        .iter()
                        .filter(|s| **s != PatchStrategy::Exact)
                        .map(|s| s.note().trim().to_string())
                        .collect::<Vec<_>>()
                        .join("; ");
                    let line = match f.kind {
                        "add" => format!("A {}", f.rel),
                        "delete" => format!("D {}", f.rel),
                        "move" => {
                            format!("M {}", f.move_dest_rel.as_deref().unwrap_or(f.rel.as_str()))
                        }
                        _ => format!("M {}", f.rel),
                    };
                    if note.is_empty() {
                        summary.push(line);
                    } else {
                        summary.push(format!("{line} ({note})"));
                    }
                }
                let total_diff = verified
                    .iter()
                    .map(|f| f.diff.clone())
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut diagnostics_text = String::new();
                for f in verified {
                    if f.kind == "delete" {
                        continue;
                    }
                    let target = match f.kind {
                        "move" => f.move_dest_abs.as_deref().unwrap_or(&f.abs),
                        _ => &f.abs,
                    };
                    let rel = match f.kind {
                        "move" => f.move_dest_rel.as_deref().unwrap_or(f.rel.as_str()),
                        _ => f.rel.as_str(),
                    };
                    let block = diagnostics_note(provider.as_ref(), target).await;
                    if !block.trim().is_empty() {
                        diagnostics_text.push_str(&format!("\n\nLSP notes for {rel}:{block}"));
                    }
                }
                let mut full = format!(
                    "Success. Updated the following files:\n{}",
                    summary.join("\n")
                );
                full.push_str(&diagnostics_text);
                if !total_diff.trim().is_empty() {
                    full.push_str("\n\n");
                    full.push_str(&total_diff);
                }
                let t = util::truncate_output(&full, &spill_dir, "apply_patch");
                // The single-approval pattern, recorded for audit: the same
                // sorted comma-joined relpaths the gate approved (grammar twins
                // with `relpaths_in_patch`, which the gate duplicates because
                // the core must not depend on this crate).
                let pattern = relpaths_in_patch(&args.patch_text).join(",");
                let mut details = serde_json::json!({
                    "diff": total_diff,
                    "truncated": t.truncated,
                    "files": summary,
                    "pattern": pattern,
                });
                if let Some(spill) = t.spilled_to {
                    details["full_output_path"] =
                        serde_json::Value::String(spill.display().to_string());
                }
                Ok(ToolOutput {
                    content: vec![epsilon_contracts::content::ContentBlock::text(t.text)],
                    details,
                    usage: None,
                    terminate: false,
                })
            }
        })
        .await
    }
}

/// Roll back one already-applied file through the existing `restore()`,
/// returning the disclosure lines for the failure message.
fn rollback_one(workspace: &Path, store: &CheckpointStore, f: &VerifiedFile) -> Vec<String> {
    let mut notes: Vec<String> = Vec::new();
    // A move touches two paths: the destination (written) and the source
    // (removed). Restore the destination first, then recreate the source.
    if f.kind == "move"
        && let (Some(dest_abs), Some(dest_rel)) = (&f.move_dest_abs, &f.move_dest_rel)
    {
        // The destination's newest entry is this patch's post-image amend.
        if let Some(entry) = store.latest_for(dest_rel) {
            match store.restore(workspace, &entry) {
                Ok(_) => notes.push(format!("restored {dest_rel}")),
                Err(why) => notes.push(format!("could not restore {dest_rel}: {why}")),
            }
        } else {
            // No checkpoint (e.g. the amend was skipped): remove what was
            // created when it is safe to say so, otherwise disclose.
            if dest_abs.exists() {
                match std::fs::remove_file(dest_abs) {
                    Ok(()) => notes.push(format!("restored {dest_rel}")),
                    Err(e) => notes.push(format!("could not restore {dest_rel}: {e}")),
                }
            }
        }
    }
    let target_rel = f.rel.clone();
    if let Some(entry) = store.latest_for(&target_rel) {
        match store.restore(workspace, &entry) {
            Ok(_) => notes.push(format!("restored {target_rel}")),
            Err(why) => notes.push(format!("could not restore {target_rel}: {why}")),
        }
    } else if f.kind == "add" && f.abs.exists() {
        match std::fs::remove_file(&f.abs) {
            Ok(()) => notes.push(format!("restored {target_rel}")),
            Err(e) => notes.push(format!("could not restore {target_rel}: {e}")),
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::tool::{Tool, ToolInvocation};
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn ctx_for(root: &Path, scope: &Arc<()>) -> ToolContext {
        ToolContext {
            session_id: "apply-patch-test".into(),
            observation_scope: Arc::clone(scope),
            cwd: root.to_path_buf(),
            spill_dir: root.join(".epsilon").join("spill"),
            token: CancellationToken::new(),
            task_host: None,
            goal_host: None,
        }
    }

    fn call(patch_text: &str) -> ToolInvocation {
        ToolInvocation {
            tool_call_id: "apply-patch-test".into(),
            args: serde_json::json!({"patchText": patch_text}),
        }
    }

    fn ws_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in files {
            let p = dir.path().join(name);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&p, body).unwrap();
        }
        dir
    }

    // ---- T1: parser (markers, empty, round-trip, unknown header with offset) ----

    #[test]
    fn t1_parser_rejects_missing_markers_empty_and_unknown_header_with_offset() {
        // Missing markers.
        let err = parse_patch("*** Add File: a.txt\n+hi\n").expect_err("markers required");
        assert!(err.contains("missing Begin/End markers"), "{err}");

        // Empty envelope.
        let err = parse_patch("*** Begin Patch\n*** End Patch").expect_err("empty refused");
        assert!(err.contains("empty patch"), "{err}");

        // Unknown header names its line (1-based over the raw input).
        let err = parse_patch("*** Begin Patch\n*** Frobnicate File: foo\n*** End Patch")
            .expect_err("unknown header refused");
        assert!(err.contains("line 2"), "offset required: {err}");
        assert!(err.contains("unknown patch header"), "{err}");

        // Round-trip: add + update (+move) + delete survive parsing.
        let patch = "*** Begin Patch\n*** Add File: new.txt\n+created\n*** Update File: old.txt\n*** Move to: renamed.txt\n@@\n-old\n+new\n*** Delete File: gone.txt\n*** End Patch";
        let hunks = parse_patch(patch).expect("round-trip parses");
        assert_eq!(hunks.len(), 3, "{hunks:?}");
        assert!(matches!(&hunks[0], ParsedHunk::Add { path, .. } if path == "new.txt"));
        match &hunks[1] {
            ParsedHunk::Update {
                path,
                move_to,
                chunks,
            } => {
                assert_eq!(path, "old.txt");
                assert_eq!(move_to.as_deref(), Some("renamed.txt"));
                assert_eq!(chunks.len(), 1);
            }
            other => panic!("expected update, got {other:?}"),
        }
        assert!(matches!(&hunks[2], ParsedHunk::Delete { path } if path == "gone.txt"));

        // Unified diffs and heredocs are refused with actionable text.
        let err = parse_patch("diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-a\n+b\n")
            .expect_err("unified diff refused");
        assert!(err.contains("unified diffs are not accepted"), "{err}");
        assert!(err.contains("*** Begin Patch"), "{err}");
        let err = parse_patch(
            "cat <<'EOF'\n*** Begin Patch\n*** Add File: h.txt\n+hi\n*** End Patch\nEOF",
        )
        .expect_err("heredoc refused");
        assert!(err.contains("heredoc"), "{err}");
        assert!(err.contains("*** Begin Patch"), "{err}");
    }

    // ---- T2: verify-upfront (bad file B leaves A untouched: mtime+bytes) ----

    #[tokio::test]
    async fn t2_bad_file_b_leaves_a_untouched() {
        let dir = ws_with(&[("a.txt", "line1\nline2\n")]);
        let a = dir.path().join("a.txt");
        let meta_before = std::fs::metadata(&a).unwrap();
        let mtime_before = meta_before.modified().unwrap();
        let len_before = meta_before.len();
        // Let enough time pass that a write would move mtime on coarse clocks.
        std::thread::sleep(std::time::Duration::from_millis(25));

        let tool = ApplyPatchTool::new();
        let scope = Arc::new(());
        let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-line2\n+changed\n*** Update File: missing.txt\n@@\n-old\n+new\n*** End Patch";
        let err = tool
            .execute(call(patch), ctx_for(dir.path(), &scope), Arc::new(|_| {}))
            .await
            .expect_err("bad file B must fail the whole patch");
        assert!(
            err.message.contains("verification failed"),
            "upfront failure: {}",
            err.message
        );
        let meta_after = std::fs::metadata(&a).unwrap();
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "line1\nline2\n",
            "A is byte-identical"
        );
        assert_eq!(meta_after.len(), len_before, "A length untouched");
        assert_eq!(
            meta_after.modified().unwrap(),
            mtime_before,
            "A mtime untouched — nothing was written before verification finished"
        );
    }

    // ---- T3: rollback (injected write failure restores file 1) ----

    #[tokio::test]
    async fn t3_write_failure_rolls_back_file_1_through_restore() {
        let dir = ws_with(&[("a.txt", "orig a\n")]);
        // "blocked" is a FILE, so creating blocked/nested.txt fails at
        // create_dir_all — the injected write failure for the second file.
        std::fs::write(dir.path().join("blocked"), "i am a file, not a dir").unwrap();
        let tool = ApplyPatchTool::new();
        let scope = Arc::new(());
        let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-orig a\n+changed a\n*** Add File: blocked/nested.txt\n+hello\n*** End Patch";
        let err = tool
            .execute(call(patch), ctx_for(dir.path(), &scope), Arc::new(|_| {}))
            .await
            .expect_err("second write must fail");
        assert!(err.message.contains("rolled back"), "{}", err.message);
        assert!(
            err.message.contains("restored"),
            "per-file disclosure: {}",
            err.message
        );
        assert!(
            err.message.contains("a.txt"),
            "file 1 named in the rollback: {}",
            err.message
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "orig a\n",
            "file 1 restored byte-for-byte via restore()"
        );
    }

    // ---- T4: single approval (pattern + preview) ----

    #[test]
    fn t4_pattern_holds_every_path_sorted_and_preview_holds_every_diff() {
        let patch = "*** Begin Patch\n*** Update File: b.txt\n@@\n-old-b\n+new-b\n*** Add File: a.txt\n+created\n*** Delete File: c.txt\n*** End Patch";
        let rels = relpaths_in_patch(patch);
        assert_eq!(
            rels,
            vec!["a.txt", "b.txt", "c.txt"],
            "sorted, every path: {rels:?}"
        );
        // The gate arm joins them with commas under capability `edit`
        // (asserted here through the twin grammar; the core arm duplicates
        // these four prefixes verbatim).
        assert_eq!(rels.join(","), "a.txt,b.txt,c.txt");

        let dir = ws_with(&[("b.txt", "old-b\n"), ("c.txt", "gone\n")]);
        let tool = ApplyPatchTool::new();
        let preview = tool
            .preview(&serde_json::json!({"patchText": patch}), dir.path())
            .expect("verifiable patch previews");
        assert!(preview.contains("-old-b"), "update diff: {preview}");
        assert!(preview.contains("+new-b"), "update diff: {preview}");
        assert!(preview.contains("+created"), "add diff: {preview}");

        // Unverifiable (missing update target) previews as None, so the
        // dialog falls back to naming files rather than showing a lie.
        let bad = "*** Begin Patch\n*** Update File: missing.txt\n@@\n-x\n+y\n*** End Patch";
        assert!(
            tool.preview(&serde_json::json!({"patchText": bad}), dir.path())
                .is_none(),
            "preview None on unverifiable"
        );
    }

    // ---- T5: caps (truncated + spill + locator; identical refused) ----

    #[tokio::test]
    async fn t5_oversized_diff_truncates_spills_and_identical_hunks_refuse() {
        // Identical hunk: old and new derive to the same bytes.
        let dir = ws_with(&[("same.txt", "same\n")]);
        let tool = ApplyPatchTool::new();
        let scope = Arc::new(());
        let identical =
            "*** Begin Patch\n*** Update File: same.txt\n@@\n-same\n+same\n*** End Patch";
        let err = tool
            .execute(
                call(identical),
                ctx_for(dir.path(), &scope),
                Arc::new(|_| {}),
            )
            .await
            .expect_err("identical content must refuse");
        assert!(err.message.contains("identical"), "{}", err.message);

        // Oversized: 3000 added lines exceed the 2000-line output cap.
        let dir = tempfile::tempdir().unwrap();
        let mut big = String::from("*** Begin Patch\n*** Add File: big.txt\n");
        for n in 0..3000 {
            big.push_str(&format!("+line {n}\n"));
        }
        big.push_str("*** End Patch");
        let out = tool
            .execute(
                call(&big),
                ctx_for(dir.path(), &Arc::new(())),
                Arc::new(|_| {}),
            )
            .await
            .expect("big patch applies");
        assert_eq!(out.details["truncated"], true, "capped");
        let spill = out.details["full_output_path"]
            .as_str()
            .expect("spill locator in details");
        assert!(
            std::path::Path::new(spill).exists(),
            "spilled full diff exists at {spill}"
        );
        let text = out
            .content
            .iter()
            .filter_map(|b| match b {
                epsilon_contracts::content::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("omitted"),
            "locator names the omission: {text:?}"
        );
        assert!(text.contains("full output saved to"), "locator: {text:?}");
    }

    // ---- T6: fuzzy bounds (drift / paraphrase / duplicates) ----

    #[tokio::test]
    async fn t6_drift_applies_with_note_paraphrase_refuses_duplicates_apply() {
        let tool = ApplyPatchTool::new();

        // Drift (trailing whitespace the model dropped) applies, with a note.
        let dir = ws_with(&[("drift.txt", "hello  \nworld\n")]);
        let scope = Arc::new(());
        let patch = "*** Begin Patch\n*** Update File: drift.txt\n@@\n-hello\n+hi\n*** End Patch";
        let out = tool
            .execute(call(patch), ctx_for(dir.path(), &scope), Arc::new(|_| {}))
            .await
            .expect("whitespace drift applies");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("drift.txt")).unwrap(),
            "hi\nworld\n"
        );
        let text = out
            .content
            .iter()
            .filter_map(|b| match b {
                epsilon_contracts::content::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("whitespace"),
            "strategy note rides the summary: {text}"
        );

        // Paraphrase (different words, not just spacing) is refused.
        let dir = ws_with(&[("fox.txt", "the quick brown fox\njumps\n")]);
        let patch = "*** Begin Patch\n*** Update File: fox.txt\n@@\n-the slow red fox\n+changed\n*** End Patch";
        let err = tool
            .execute(
                call(patch),
                ctx_for(dir.path(), &Arc::new(())),
                Arc::new(|_| {}),
            )
            .await
            .expect_err("paraphrase must not match");
        assert!(
            err.message.contains("Failed to find expected lines"),
            "{}",
            err.message
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("fox.txt")).unwrap(),
            "the quick brown fox\njumps\n",
            "refusal leaves the file alone"
        );

        // Duplicate lines with distinct @@ context both apply (forward search,
        // no cross-file uniqueness requirement).
        let dir = ws_with(&[("multi.txt", "fn a\nx=10\ny=2\nfn b\nx=10\ny=20\n")]);
        let patch = "*** Begin Patch\n*** Update File: multi.txt\n@@ fn a\n-x=10\n+x=11\n@@ fn b\n-x=10\n+x=12\n*** End Patch";
        tool.execute(
            call(patch),
            ctx_for(dir.path(), &Arc::new(())),
            Arc::new(|_| {}),
        )
        .await
        .expect("both duplicates apply");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("multi.txt")).unwrap(),
            "fn a\nx=11\ny=2\nfn b\nx=12\ny=20\n"
        );
    }

    // ---- T7: undo (N entries, one anchor range, restores all) ----

    #[tokio::test]
    async fn t7_n_files_share_one_ledger_range_and_all_restore() {
        let dir = ws_with(&[("one.txt", "one\n"), ("two.txt", "two\n")]);
        let store = CheckpointStore::open(dir.path());
        let start = store.entries().last().map(|e| e.seq + 1).unwrap_or(1);
        let tool = ApplyPatchTool::new();
        let patch = "*** Begin Patch\n*** Update File: one.txt\n@@\n-one\n+ONE\n*** Update File: two.txt\n@@\n-two\n+TWO\n*** Add File: three.txt\n+THREE\n*** End Patch";
        let out = tool
            .execute(
                call(patch),
                ctx_for(dir.path(), &Arc::new(())),
                Arc::new(|_| {}),
            )
            .await
            .expect("three-file patch applies");
        let text = out
            .content
            .iter()
            .filter_map(|b| match b {
                epsilon_contracts::content::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("M one.txt"), "{text}");
        assert!(text.contains("M two.txt"), "{text}");
        assert!(text.contains("A three.txt"), "{text}");

        // One turn anchor covers N entries: the turn's [start, end] ledger
        // range holds a checkpoint for every touched path (no new anchor type).
        let end = store.entries().last().map(|e| e.seq).unwrap_or(0);
        let range = store.entries_between(start, end);
        let mut paths: Vec<String> = range.iter().map(|e| e.path.clone()).collect();
        paths.sort();
        paths.dedup();
        for want in ["one.txt", "two.txt", "three.txt"] {
            assert!(
                paths.contains(&want.to_string()),
                "range covers {want}: {paths:?}"
            );
        }
        assert!(
            range.iter().all(|e| e.tool == "apply_patch"),
            "every entry in the range is this patch's"
        );

        // Undo restores all three through the existing restore().
        for rel in ["three.txt", "two.txt", "one.txt"] {
            let entry = store.latest_for(rel).expect("checkpoint for {rel}");
            store.restore(dir.path(), &entry).expect("restores");
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("one.txt")).unwrap(),
            "one\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("two.txt")).unwrap(),
            "two\n"
        );
        assert!(
            !dir.path().join("three.txt").exists(),
            "created file removed"
        );
    }

    // ---- T8: guards (oversize / binary / outside-workspace / stale) ----

    #[tokio::test]
    async fn t8_guards_refuse_oversize_binary_escape_and_stale() {
        let tool = ApplyPatchTool::new();

        // Oversize: over the 256 KiB honest-edit limit.
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.txt");
        std::fs::write(&big, vec![b'x'; 300 * 1024]).unwrap();
        // Pad to a newline-terminated text file so the refusal is the size
        // cap, not a binary/UTF-8 complaint.
        std::fs::write(&big, format!("{}\n", "x".repeat(300 * 1024))).unwrap();
        let patch = "*** Begin Patch\n*** Update File: big.txt\n@@\n-xxx\n+yyy\n*** End Patch";
        let err = tool
            .execute(
                call(patch),
                ctx_for(dir.path(), &Arc::new(())),
                Arc::new(|_| {}),
            )
            .await
            .expect_err("oversize refused");
        assert!(err.message.contains("exceeds"), "{}", err.message);

        // Binary: NUL in the head.
        let dir = ws_with(&[]);
        let blob = dir.path().join("blob.bin");
        std::fs::write(&blob, b"hello\x00world\n").unwrap();
        let patch = "*** Begin Patch\n*** Update File: blob.bin\n@@\n-hello\n+hi\n*** End Patch";
        let err = tool
            .execute(
                call(patch),
                ctx_for(dir.path(), &Arc::new(())),
                Arc::new(|_| {}),
            )
            .await
            .expect_err("binary refused");
        assert!(err.message.contains("binary"), "{}", err.message);

        // Outside the workspace.
        let dir = ws_with(&[("in.txt", "in\n")]);
        let patch = "*** Begin Patch\n*** Update File: ../outside.txt\n@@\n-x\n+y\n*** End Patch";
        let err = tool
            .execute(
                call(patch),
                ctx_for(dir.path(), &Arc::new(())),
                Arc::new(|_| {}),
            )
            .await
            .expect_err("escape refused");
        assert!(
            err.message.contains("outside the workspace"),
            "{}",
            err.message
        );

        // Stale: read, then changed on disk, then patched.
        let dir = ws_with(&[("stale.txt", "original content\n")]);
        let abs = dir.path().join("stale.txt");
        let scope = Arc::new(());
        crate::read_log::record_read(&scope, &abs, crate::read_log::Coverage::page(1, 1, Some(1)));
        std::thread::sleep(std::time::Duration::from_millis(25));
        std::fs::write(&abs, "someone else rewrote this file entirely\n").unwrap();
        let patch = "*** Begin Patch\n*** Update File: stale.txt\n@@\n-original content\n+patched\n*** End Patch";
        let err = tool
            .execute(call(patch), ctx_for(dir.path(), &scope), Arc::new(|_| {}))
            .await
            .expect_err("stale refused");
        assert!(
            err.message.contains("changed on disk"),
            "stale diagnosis: {}",
            err.message
        );
    }
}
