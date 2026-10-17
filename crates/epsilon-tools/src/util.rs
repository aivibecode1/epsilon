//! Shared plumbing for built-in tools: workspace-boundary checks, directory walking,
//! and output truncation with spill-to-disk locators.

use std::fs;
use std::io::{BufRead, BufReader, Read as _, Seek as _};
use std::path::{Path, PathBuf};

use epsilon_contracts::tool::{ToolFailure, ToolUpdateSink};

/// Hard caps (opencode/pi-informed defaults; constants so tests can assert them).
/// Max bytes returned from a single file read.
pub const MAX_READ_BYTES: usize = 256 * 1024;
/// Max lines returned from a single file read.
pub const MAX_READ_LINES: usize = 2000;
/// Max output lines a tool may return before tail-truncation.
pub const MAX_TOOL_OUTPUT_LINES: usize = 2000;
/// Max output bytes a tool may return before tail-truncation.
pub const MAX_TOOL_OUTPUT_BYTES: usize = 64 * 1024;
/// Walker stops after this many files.
pub const WALK_MAX_ENTRIES: usize = 5000;
/// Byte cap for listing-tool results (`find`, `ls`).
///
/// This is pi's `DEFAULT_MAX_BYTES` (`tools/truncate.ts:12`), applied by its `find` and `ls`
/// through `truncateHead` (`find.ts:277`, `ls.ts:141`). It is *not*
/// [`MAX_TOOL_OUTPUT_BYTES`], which is the head+tail budget for spilling a command's or a
/// fetch's whole output; a file listing has no useful tail to keep, so it is cut from the
/// front only and is never spilled.
pub const TOOL_OUTPUT_MAX_BYTES: usize = 50 * 1024;
/// Walker never descends deeper than this.
pub const WALK_MAX_DEPTH: usize = 16;

/// Directories never worth searching (build artifacts and VCS internals).
pub const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", "dist", ".epsilon"];

// ---------------------------------------------------------------------------
// M30: lightweight unified diff (no crate dependency — opencode does the same)
// ---------------------------------------------------------------------------

/// Generate a compact unified diff between two string contents.
///
/// Returns the diff text directly (suitable for storing in `details.diff`).
/// When the inputs are identical the output is empty — callers can skip storing
/// it entirely.
pub fn unified_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    // (marker, line) where marker is '-', '+', or ' ' (context).
    let mut ops: Vec<(char, &str)> = Vec::new();
    let max = old_lines.len().max(new_lines.len());
    for i in 0..max {
        match (old_lines.get(i), new_lines.get(i)) {
            (Some(o), Some(n)) if *o == *n => ops.push((' ', o)),
            (Some(o), Some(n)) => {
                ops.push(('-', o));
                ops.push(('+', n));
            }
            (Some(o), None) => ops.push(('-', o)),
            (None, Some(n)) => ops.push(('+', n)),
            (None, None) => {}
        }
    }
    if ops.iter().all(|o| o.0 == ' ') {
        return String::new();
    }
    const CTX: usize = 3;
    let first_change = ops.iter().position(|o| o.0 != ' ').unwrap_or(0);
    let last_change = ops
        .iter()
        .rposition(|o| o.0 != ' ')
        .unwrap_or(ops.len() - 1);
    let from = first_change.saturating_sub(CTX);
    let to = (last_change + CTX + 1).min(ops.len());
    let mut old_line = 1usize;
    let mut new_line = 1usize;
    for &(m, _) in &ops[..from] {
        if m != '+' {
            old_line += 1;
        }
        if m != '-' {
            new_line += 1;
        }
    }
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    for &(m, _) in &ops[from..to] {
        if m != '+' {
            old_count += 1;
        }
        if m != '-' {
            new_count += 1;
        }
    }
    let mut out = format!("@@ -{old_line},{old_count} +{new_line},{new_count} @@\n");
    for &(m, text) in &ops[from..to] {
        out.push(m);
        out.push_str(text);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod diff_tests {
    use super::*;

    #[test]
    fn identical_inputs_produce_no_output() {
        assert!(unified_diff("hello\n", "hello\n").is_empty());
    }

    #[test]
    fn simple_replacement() {
        let d = unified_diff("a\nb\nc\n", "a\nx\nc\n");
        assert!(d.contains("-b"));
        assert!(d.contains("+x"));
        assert!(d.contains("@@"));
    }

    #[test]
    fn pure_insertion() {
        let d = unified_diff("a\n", "a\nb\n");
        assert!(d.contains("+b"));
    }

    #[test]
    fn pure_deletion() {
        let d = unified_diff("a\nb\n", "a\n");
        assert!(d.contains("-b"));
    }
}

/// Re-exported from `epsilon-core` (M74): the rule moved, the call sites did not.
///
/// Two of them, with the difference documented at the source. `resolve_for_access` is
/// the one an **enforcement** site wants; `lexically_normalize` is the one a planner
/// key wants, because a scheduling identity must not change when a symlink moves.
pub use epsilon_core::paths::{display_canonical, lexically_normalize, resolve_for_access};

/// Resolve `raw` inside `cwd` or refuse, with a message that says why it was refused.
///
/// The one call every path-taking tool makes, so that the six copies of
/// "resolve, check the flag, build the error" cannot drift — and so that a symlink
/// escape is reported as one, by name, rather than as a path that merely looks
/// innocent.
pub fn resolve_or_refuse(
    cwd: &std::path::Path,
    raw: &str,
) -> Result<std::path::PathBuf, ToolFailure> {
    let r = resolve_for_access(cwd, raw);
    if r.inside {
        Ok(r.path)
    } else if r.deceptive {
        // **A refusal must name the truth (A33).** `.env.` is not outside the workspace,
        // and "resolves outside the workspace" on a name the user can see is a sentence
        // they will not believe — which is the same failure `escaped_to()` was introduced
        // to fix, one level up. This one has its own reason because it has its own cause.
        Err(ToolFailure::msg(format!(
            "Refused: '{raw}' is not the file it looks like. A name with {} \
             opens a different file on Windows, so it cannot be judged by reading it. \
             Name the file exactly.",
            epsilon_core::paths::deceptive_name_reason(std::path::Path::new(raw))
                .unwrap_or("a spelling Windows resolves elsewhere")
        )))
    } else {
        Err(escaped_workspace(raw, r.escaped_to()))
    }
}

/// The failure every tool returns when a path leaves the declared workspace.
///
/// `escaped_to` is where a path that *spelled* its way inside actually pointed. It is
/// the difference between a refusal the user can act on and one they cannot: `notes/creds`
/// resolving to `~/.aws/credentials` is a fact they need, and a bare "outside the
/// workspace" on a path that is visibly in their own tree is a fact they will not believe.
pub fn escaped_workspace(raw: &str, escaped_to: Option<&std::path::Path>) -> ToolFailure {
    let via = match escaped_to {
        Some(target) => format!(
            " It looks inside the workspace, but it is a link to {}.",
            display_canonical(target)
        ),
        None => String::new(),
    };
    ToolFailure::msg(format!(
        "Refused: '{raw}' resolves outside the workspace.{via} \
         Operate on paths inside the working directory."
    ))
}

/// Recursively collect files under `root`, honouring the workspace's ignore
/// rules and the entry/depth caps.
///
/// Returns paths relative to `root` (forward-slash separated for stable rule
/// matching).
///
/// `.gitignore` is read here, once, from `root` down — not consulted lazily — so
/// a nested file's rules apply to the whole subtree below it and two sibling
/// directories cannot disagree about what the repository ignores. Callers that
/// already have an [`IgnoreSet`] should use [`walk_files_with`] rather than
/// paying to read the same files twice.
///
/// Hidden directories are skipped, unconditionally: a `.venv` is not workspace
/// content. Hidden *files* stay visible, because `.env` is content the read tools
/// must be able to see.
///
/// A `.gitignore` negation does **not** re-include a hidden directory here, and the
/// reason is worth stating because two comments in this file used to claim otherwise:
/// `IgnoreSet::is_ignored` resolves ancestors and answers `false` both for "a rule
/// ignored this and a later one re-included it" and for "no rule mentions it", so a
/// walk has no query that can tell the two apart. The matcher itself is correct — see
/// its own doc for the `dir/` + `!dir/` example — and the walker simply cannot use it.
/// Making the escape hatch real needs a matcher query that distinguishes the cases;
/// until then this is a tracked limitation, not a feature, and the test below says so
/// rather than asserting behaviour that is not there.
///
/// [`IgnoreSet`]: epsilon_core::ignore::IgnoreSet
pub fn walk_files(root: &Path) -> Vec<PathBuf> {
    let ignore = epsilon_core::ignore::IgnoreSet::load(root, WALK_IGNORE_FILES);
    walk_files_with(root, &ignore)
}

/// [`walk_files`] with the ignore set supplied by the caller.
pub fn walk_files_with(root: &Path, ignore: &epsilon_core::ignore::IgnoreSet) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut budget = WALK_MAX_ENTRIES;
    walk_inner(root, root, 0, &mut out, &mut budget, ignore);
    out
}

/// How many `.gitignore` files a walk will read before giving up on finding more.
///
/// Bounded: a large monorepo can hold thousands, and a tool that spends its whole
/// budget reading ignore files has none left for the work it was asked to do. The
/// rules found before the cap still apply, so the walk is narrower than ideal and
/// never wrong about what it searched.
const WALK_IGNORE_FILES: usize = 256;

fn walk_inner(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<PathBuf>,
    budget: &mut usize,
    ignore: &epsilon_core::ignore::IgnoreSet,
) {
    if depth > WALK_MAX_DEPTH || *budget == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if *budget == 0 {
            return;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let path = e.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        // **A link is not descended into.** `is_dir()` below follows it — a symlink, or a
        // Windows junction, which needs no privilege — so without this a repository holding
        // `docs -> ~/Documents` has a plain `grep` walk it and print every matching line into
        // the tool result, the transcript and the durable session log, with **no permission
        // ask**: `gate_subject` saw only a workspace-relative name, so no `external_dir`
        // capability was ever raised. See [`epsilon_core::paths::is_link`] for the whole
        // story, and the project's own review that named this in 2026-09 and left it
        // unimplemented (`audit/evidence/f09-path-policy-map.md:26,29`).
        if epsilon_core::paths::is_link(&path) {
            *budget = budget.saturating_sub(1);
            continue;
        }
        let is_dir = path.is_dir();
        // The always-skip list first: a build directory is not a preference, and
        // walking one turns a fast tool into a stalled one.
        if is_dir && epsilon_core::ignore::skip_dir(&name) {
            continue;
        }
        // A hidden DIRECTORY is skipped, always; a hidden FILE is content and stays.
        //
        // This check used to read `name.starts_with('.') && !ignore.is_ignored(..)`, with
        // a comment promising that a `.gitignore` negation re-includes a hidden directory.
        // It never did: the clause can only differ from "always skip" when the directory
        // *is* ignored, and the very next check skips it in that case. Dead logic plus a
        // promise nobody could keep — the doc comment on `walk_files` now states the
        // limitation and the test records why instead of asserting the feature.
        if is_dir && name.starts_with('.') {
            continue;
        }
        if ignore.is_ignored(&relative, is_dir) {
            continue;
        }
        if is_dir {
            walk_inner(root, &path, depth + 1, out, budget, ignore);
        } else {
            *budget -= 1;
            out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
}

/// Read complete, valid UTF-8 for whole-file edit writeback, within the read byte limit.
///
/// Unlike display reads, edits must never use a truncated or lossily decoded buffer.
/// Probe one extra byte on the opened handle instead of trusting path metadata.
/// This bounds ingestion without widening the model-visible read cap.
pub fn read_text_for_edit(path: &Path) -> Result<String, ToolFailure> {
    let f = fs::File::open(path)
        .map_err(|e| ToolFailure::msg(format!("cannot open {}: {e}", path.display())))?;
    let mut buf = Vec::new();
    f.take(MAX_READ_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| ToolFailure::msg(format!("read error: {e}")))?;
    if buf.len() > MAX_READ_BYTES {
        return Err(ToolFailure::msg(format!(
            "refused to edit: file exceeds the {MAX_READ_BYTES}-byte edit limit"
        )));
    }
    if buf.iter().take(4096).any(|b| *b == 0) {
        return Err(ToolFailure::msg("refused: file looks binary"));
    }
    String::from_utf8(buf).map_err(|_| ToolFailure::msg("refused to edit: file is not valid UTF-8"))
}

/// Read a file's bytes, refusing probable binaries (NUL in the first 4 KiB), and report
/// whether the file was longer than the byte cap.
///
/// The caller needs to know: a truncated read is a *partial view* and must be announced as
/// one (`grep` names the files it could only search in part). The read tool no longer uses
/// this helper — since M95 it pages with [`read_page`], where a window boundary is always a
/// line boundary — but `grep` still searches each walked file through it, and the cap there
/// is deliberate: the walker visits up to [`WALK_MAX_ENTRIES`] files and an unbounded read
/// per file would trade a silent miss for an unbounded scan.
pub fn read_text_capped_ex(path: &Path) -> Result<(String, bool), ToolFailure> {
    let f = fs::File::open(path)
        .map_err(|e| ToolFailure::msg(format!("cannot open {}: {e}", path.display())))?;
    let mut buf = Vec::new();
    f.take(MAX_READ_BYTES as u64)
        .read_to_end(&mut buf)
        .map_err(|e| ToolFailure::msg(format!("read error: {e}")))?;
    if buf.iter().take(4096).any(|b| *b == 0) {
        return Err(ToolFailure::msg("refused: file looks binary"));
    }
    let capped = fs::metadata(path)
        .map(|m| m.len() > MAX_READ_BYTES as u64)
        .unwrap_or(false);
    Ok((String::from_utf8_lossy(&buf).into_owned(), capped))
}

// ---------------------------------------------------------------------------
// M95: paged reads — pi's `read`-tool offset/limit window, streamed
// ---------------------------------------------------------------------------

/// Where the scan that produced a [`ReadPage`] stopped. pi's `TruncationResult.truncatedBy`
/// (`truncate.ts:21`) plus [`PageBound::CallerLimit`], which pi resolves before truncation
/// (`read.ts:287-293`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageBound {
    /// The page reached the end of the file; nothing follows it.
    Eof,
    /// The page filled [`MAX_READ_LINES`].
    Lines,
    /// The page filled [`MAX_READ_BYTES`].
    Bytes,
    /// The caller's `limit` was reached; the file continues.
    CallerLimit,
    /// The first line of the page alone exceeds [`MAX_READ_BYTES`] (pi `firstLineExceedsLimit`).
    LongLine,
}

impl PageBound {
    /// Stable name for `details` payloads and test assertions.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Eof => "eof",
            Self::Lines => "lines",
            Self::Bytes => "bytes",
            Self::CallerLimit => "caller_limit",
            Self::LongLine => "long_line",
        }
    }
}

/// One window of a text file (M95).
///
/// - `text` holds whole lines joined by `\n` — never a partial line, and never a lossy
///   fragment of one (pi's truncation is line-based; the old byte-prefix cut could split a
///   UTF-8 character and `from_utf8_lossy` turned the fragment into U+FFFD).
/// - `first_line`/`last_line` are the 1-based numbers of the lines in `text` (`0` when the
///   page is empty).
/// - `total_lines` is the file's exact line count when the scan reached EOF. pi always knows
///   it — it reads the whole file (`read.ts:275-276`); a streamed page knows it only when it
///   got there and reports `None` rather than a guess.
/// - `next_offset` is the offset a continuation read should use: `Some` exactly when unread
///   lines remain, and the same number pi writes into its continuation notices.
/// - `long_line` is `Some((line number, byte size))` when the page stopped because its *first*
///   line alone exceeds [`MAX_READ_BYTES`]: pi answers with a `sed` fallback, not with content,
///   and the page carries no other lines.
pub struct ReadPage {
    /// Whole lines from the page, joined by `\n` (no trailing newline).
    pub text: String,
    /// 1-based number of the first line in `text` (0 when it is empty).
    pub first_line: usize,
    /// 1-based number of the last line in `text` (0 when it is empty).
    pub last_line: usize,
    /// The file's exact line count, when the scan reached EOF.
    pub total_lines: Option<usize>,
    /// The offset a continuation read should use; `None` when no unread line remains.
    pub next_offset: Option<usize>,
    /// Why the page stopped where it did.
    pub truncated_by: PageBound,
    /// The over-cap line that stopped the page, with its exact byte size.
    pub long_line: Option<(usize, usize)>,
}

/// One raw line, as [`read_page`] reads it: the retained head, the line's exact content byte
/// length, and whether the stream had nothing left at all.
struct RawLine {
    /// The first bytes of the line's content, at most the caller's retention cap.
    head: Vec<u8>,
    /// Exact content byte length, newline excluded (`\r` included, as pi counts it).
    len: usize,
    /// True when the stream was already at EOF and nothing was read.
    eof: bool,
}

/// Read one `\n`-terminated line, counting its exact byte length while retaining at most
/// `retain` bytes of it.
///
/// `BufReader::read_until` grows its buffer to the whole line, which a single over-cap line
/// would make unbounded; this counts without retaining, so the size can still be reported
/// exactly (pi's `firstLineExceedsLimit` message carries it) at bounded memory.
fn read_line_bounded(reader: &mut impl BufRead, retain: usize) -> std::io::Result<RawLine> {
    let mut head: Vec<u8> = Vec::new();
    let mut len = 0usize;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(RawLine {
                head,
                len,
                eof: len == 0,
            });
        }
        let newline = buf.iter().position(|&b| b == b'\n');
        let content = &buf[..newline.unwrap_or(buf.len())];
        let room = retain.saturating_sub(head.len());
        head.extend_from_slice(&content[..content.len().min(room)]);
        len += content.len();
        match newline {
            Some(at) => {
                reader.consume(at + 1);
                return Ok(RawLine {
                    head,
                    len,
                    eof: false,
                });
            }
            None => {
                let n = buf.len();
                reader.consume(n);
            }
        }
    }
}

/// Decode one line's bytes into the string a read shows.
///
/// Splitting is on `\n` with one trailing `\r` removed (`str::lines`' rule, which is what
/// this tool has always shown). Bytes that are not valid UTF-8 are replaced the way pi's
/// `buffer.toString("utf-8")` (`read.ts:274`) replaces them — and because a page is cut on
/// line boundaries, that replacement can only come from the file's own bytes, never from the
/// window.
fn decode_line(bytes: &[u8]) -> String {
    let body = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    String::from_utf8_lossy(body).into_owned()
}

/// Format bytes the way pi's `formatSize` does (`truncate.ts:61-69`): `512B`, `50.0KB`, `5.0MB`.
pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Keep whole lines from the front until the next one would pass `max_bytes`.
///
/// pi's `truncateHead` with `maxLines: Number.MAX_SAFE_INTEGER` — the shape both its `find`
/// and its `ls` use, where the entry count is capped separately and only the byte budget is
/// left to enforce. Returns the kept text and the lines it kept, so a caller can say how many
/// entries the byte bound dropped without counting again. A line is never cut in half: a
/// single line larger than the whole budget yields an empty result rather than a prefix
/// (`find` and `ls` build one line per entry, so this is unreachable for paths that fit
/// `PATH_MAX`, and safer than emitting a mangled path if it ever is not).
pub fn truncate_head_bytes(lines: &[String], max_bytes: usize) -> (String, usize) {
    let mut kept: Vec<&str> = Vec::new();
    let mut bytes = 0usize;
    for line in lines {
        // The `+1` is the newline a join re-adds, exactly as pi counts it (`truncate.ts:128`).
        let cost = line.len() + 1;
        if bytes + cost > max_bytes {
            break;
        }
        bytes += cost;
        kept.push(line);
    }
    (kept.join("\n"), kept.len())
}

/// Read one page of a text file with pi's `read`-tool semantics (M95).
///
/// The scan is streamed: lines before `offset` are read and dropped, the page is retained,
/// and the remainder is counted without being retained — resident memory is one line plus
/// the page, never the whole file, at the cost of an `O(file bytes)` scan. pi reads the whole
/// file and slices (`read.ts:273-293`); this produces the same window without the memory.
/// Decoding is per line, so a window boundary is always a line boundary and no multibyte
/// character is ever cut in half.
///
/// Semantics ported from pi (`packages/coding-agent/src/core/tools/read.ts:275-323`, and the
/// same function in `packages/agent/src/harness/tools/read.ts:97-141`):
/// - `offset` is 1-based and past the end of the file is an error naming both numbers
///   (`read.ts:281-283`), while an empty file reads as `Ok` and empty (pi's `"".split("\n")`
///   yields one empty element, so the offset check passes and the selection is empty);
/// - `limit` slices first and the page is then capped at [`MAX_READ_LINES`] / [`MAX_READ_BYTES`]
///   (`read.ts:287-295`; epsilon keeps its own byte budget where pi uses 50 KiB);
/// - a page whose first line alone exceeds the byte budget reports that line's size and a
///   `sed` fallback (`read.ts:297-301`) instead of returning a lossy prefix of it.
///
/// One deliberate divergence, recorded in `docs/milestones/M95-paged-reads.md`: this counts
/// the lines it can display (`str::lines`), where pi counts `text.split("\n").length` — one
/// more for a newline-terminated file, a phantom line pi can return as empty and which would
/// make the numbers in pi's own continuation notices disagree with the lines it can show.
pub fn read_page(
    path: &Path,
    offset: usize,
    limit: Option<usize>,
) -> Result<ReadPage, ToolFailure> {
    let offset = offset.max(1);
    let mut file = fs::File::open(path)
        .map_err(|e| ToolFailure::msg(format!("cannot open {}: {e}", path.display())))?;
    // Binary refusal on the first 4 KiB of the file — the same rule (and window) the edit
    // path has always applied. Probed before the scan so a short page cannot hide it.
    let mut probe = [0u8; 4096];
    let probed = file
        .read(&mut probe)
        .map_err(|e| ToolFailure::msg(format!("read error: {e}")))?;
    if probe[..probed].contains(&0) {
        return Err(ToolFailure::msg("refused: file looks binary"));
    }
    file.seek(std::io::SeekFrom::Start(0))
        .map_err(|e| ToolFailure::msg(format!("read error: {e}")))?;
    let mut reader = BufReader::new(file);

    let mut line_no = 0usize; // 1-based number of the last line read from the file
    let mut lines: Vec<String> = Vec::new();
    let mut page_bytes = 0usize;
    let mut truncated_by = PageBound::Eof;
    let mut long_line: Option<(usize, usize)> = None;
    let mut eof = false;

    loop {
        let raw = read_line_bounded(&mut reader, MAX_READ_BYTES + 1)
            .map_err(|e| ToolFailure::msg(format!("read error: {e}")))?;
        if raw.eof {
            eof = true;
            break;
        }
        line_no += 1;
        if line_no < offset {
            continue;
        }
        if limit.is_some_and(|l| lines.len() >= l) {
            truncated_by = PageBound::CallerLimit;
            break;
        }
        if lines.is_empty() && raw.len > MAX_READ_BYTES {
            // pi `firstLineExceedsLimit` (`read.ts:297-301`; `truncate.ts:103-105` checks the
            // *selection's* `lines[0]` alone): the page's first line exceeds the budget, so no
            // offset can ever show it through this tool and the page is not a window onto it.
            // An over-cap line *later* in the page is an ordinary byte stop instead: the lines
            // already shown are kept, the total and the continuation survive, and coverage
            // stays partial — pi's byte accounting (`truncate.ts:126-137`) never lets such a
            // line through either, it just stops the page where the budget ran out.
            long_line = Some((line_no, raw.len));
            truncated_by = PageBound::LongLine;
            break;
        }
        // pi's byte accounting over the selection: content bytes plus one separator for
        // every line after the selection's first (`truncate.ts:126-137`).
        let cost = raw.len + usize::from(!lines.is_empty());
        if page_bytes + cost > MAX_READ_BYTES {
            truncated_by = PageBound::Bytes;
            break;
        }
        if lines.len() >= MAX_READ_LINES {
            truncated_by = PageBound::Lines;
            break;
        }
        lines.push(decode_line(&raw.head));
        page_bytes += cost;
    }
    // pi's post-hoc tie-break (`truncate.ts:139-142`): a byte stop on a selection that
    // already holds the full line budget is reported as the line limit.
    if truncated_by == PageBound::Bytes && lines.len() >= MAX_READ_LINES {
        truncated_by = PageBound::Lines;
    }

    // Count the rest of the file without retaining it, so the notices and the past-end error
    // can carry pi's exact numbers. The over-cap stop skips this: pi's message for it needs
    // only the line's size, and the caller has already been pointed at `sed`.
    // ponytail: O(rest of file) per read — that scan is what makes the counts exact, and pi
    // reads the whole file anyway; a line index (bytes → line number) is the upgrade path if it
    // ever shows up in profiles.
    if !eof && truncated_by != PageBound::LongLine {
        loop {
            let raw = read_line_bounded(&mut reader, 0)
                .map_err(|e| ToolFailure::msg(format!("read error: {e}")))?;
            if raw.eof {
                break;
            }
            line_no += 1;
        }
    }
    let total_lines = if truncated_by == PageBound::LongLine {
        None
    } else {
        Some(line_no)
    };

    if let Some(total) = total_lines {
        // pi `read.ts:281-283`. `max(1)` keeps an empty file readable at offset 1 (pi's Ok,
        // empty case) while any offset past the last line stays the error it is in pi.
        if offset > total.max(1) {
            return Err(ToolFailure::msg(format!(
                "Offset {offset} is beyond end of file ({total} lines total)"
            )));
        }
    }

    let first_line = if lines.is_empty() { 0 } else { offset };
    let last_line = if lines.is_empty() {
        0
    } else {
        offset + lines.len() - 1
    };
    let shown_to = if lines.is_empty() {
        offset - 1
    } else {
        last_line
    };
    let next_offset = match (truncated_by, total_lines) {
        (PageBound::LongLine, _) => None,
        (_, Some(total)) if total > shown_to => Some(shown_to + 1),
        _ => None,
    };
    Ok(ReadPage {
        text: lines.join("\n"),
        first_line,
        last_line,
        total_lines,
        next_offset,
        truncated_by,
        long_line,
    })
}

/// Bridge between a running child process's raw output and the agent's tool
/// update events (pi streams partial bash output; before this, a ten-minute
/// build showed nothing at all until it exited).
///
/// Emits at most once per [`LiveOutput::INTERVAL_MS`] and only ever carries a
/// bounded tail of what has arrived, so a build that prints megabytes cannot
/// flood the event bus, the transcript, or the model's context. The complete
/// output still comes back from the tool call itself — this is a view, never the
/// record.
pub struct LiveOutput {
    tail: std::sync::Arc<std::sync::Mutex<String>>,
    last_emit_ms: std::sync::Arc<std::sync::atomic::AtomicU64>,
    on_update: ToolUpdateSink,
}

impl LiveOutput {
    /// Minimum spacing between emissions.
    pub const INTERVAL_MS: u64 = 250;
    /// How much of the newest output a single update may carry.
    pub const TAIL_BYTES: usize = 4 * 1024;

    /// Wrap the agent's update sink.
    pub fn new(on_update: ToolUpdateSink) -> Self {
        Self {
            tail: std::sync::Arc::new(std::sync::Mutex::new(String::new())),
            last_emit_ms: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            on_update,
        }
    }

    /// The raw sink to hand a sandbox or child drain task.
    pub fn sink(&self) -> epsilon_safety::sandbox::OutputSink {
        let tail = std::sync::Arc::clone(&self.tail);
        let last = std::sync::Arc::clone(&self.last_emit_ms);
        let on_update = std::sync::Arc::clone(&self.on_update);
        std::sync::Arc::new(move |chunk: &str| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let snapshot = {
                let Ok(mut buf) = tail.lock() else { return };
                buf.push_str(chunk);
                trim_to_tail(&mut buf, Self::TAIL_BYTES);
                if now.saturating_sub(last.load(std::sync::atomic::Ordering::Relaxed))
                    < Self::INTERVAL_MS
                {
                    return;
                }
                buf.clone()
            };
            last.store(now, std::sync::atomic::Ordering::Relaxed);
            on_update(epsilon_contracts::tool::ToolPartial {
                content: vec![epsilon_contracts::content::ContentBlock::text(snapshot)],
                details: serde_json::json!({ "streaming": true }),
            });
        })
    }
}

/// Keep `buf` within `max` bytes, cutting on a char boundary from the front.
fn trim_to_tail(buf: &mut String, max: usize) {
    if buf.len() <= max {
        return;
    }
    let mut cut = buf.len() - max;
    while cut < buf.len() && !buf.is_char_boundary(cut) {
        cut += 1;
    }
    *buf = buf[cut..].to_string();
}

/// Outcome of truncating oversized tool output.
pub struct Truncated {
    /// What the model sees.
    pub text: String,
    /// True when content was cut.
    pub truncated: bool,
    /// Where the FULL output was spilled (when truncated and the spill worked).
    pub spilled_to: Option<PathBuf>,
    /// Why the spill failed, when it did. Dropping output silently is the one
    /// thing a locator must never do: a tool result that says "saved to X" and
    /// no file exists at X is worse than no locator at all.
    pub spill_error: Option<String>,
}

/// Create the private spill directory for a workspace (`0700` on Unix): spilled
/// tool output routinely contains source, env dumps and credentials.
fn ensure_spill_dir(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        // Best effort: a pre-existing world-readable spill root is tightened.
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Write the full output to a fresh file under `spill_dir`.
///
/// `create_new` (O_EXCL) refuses to follow a pre-planted symlink or overwrite an
/// existing file; the name carries a per-process counter so two concurrent calls
/// in the same millisecond do not collide. Returns the path, or the reason it
/// could not be written.
fn spill_full_output(output: &str, spill_dir: &Path, stem: &str) -> Result<PathBuf, String> {
    ensure_spill_dir(spill_dir)?;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    for attempt in 0..8u32 {
        let path = spill_dir.join(format!("{stem}-{millis}-{attempt}.out"));
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        match opened {
            Ok(mut f) => {
                use std::io::Write as _;
                f.write_all(output.as_bytes())
                    .map_err(|e| format!("write {}: {e}", path.display()))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    let _ = f.set_permissions(fs::Permissions::from_mode(0o600));
                }
                return Ok(path);
            }
            // Name taken (concurrent call in the same millisecond): next attempt.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create {}: {e}", path.display())),
        }
    }
    Err(format!(
        "could not allocate a spill file under {}",
        spill_dir.display()
    ))
}

/// Clip a string to at most `max_bytes`, on a char boundary, from the front.
fn clip_head(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut cut = max_bytes;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

/// What the model should do with a spilled file, branched on truthful
/// task-tool availability (opencode `truncate.ts:129-131`).
///
/// With a `task` tool the hint delegates: a subagent digs through the spill
/// with `grep`/`read` while the caller keeps its context. Without one the
/// hint names only what every registry has (`grep`, `read` with
/// offset/limit) and never invents delegation.
///
/// The wording names Epsilon's actual surface (`task.rs`: the tool is
/// `task`, the worker is a subagent of some `subagent_type`, the search
/// tools are `grep` and `read`) — not opencode's dedicated "explore agent",
/// which Epsilon does not have.
pub fn truncation_hint(has_task_tool: bool) -> &'static str {
    if has_task_tool {
        "Use the task tool to have a subagent process the spilled file with grep and read (with offset/limit). Do NOT read the full file yourself - delegate to save context."
    } else {
        "Use grep to search the full content or read with offset/limit to view specific sections."
    }
}

/// [`truncate_output`] with the task-aware hint made explicit.
///
/// `has_task_tool` must reflect the registry the result is shown to — `true`
/// only where a `task` tool is actually registered. [`truncate_output`]
/// keeps the safe default (`false`): it never promises delegation it cannot
/// keep. Caps and spill behaviour are identical; only the hint branches.
pub fn truncate_output_with_task(
    output: &str,
    spill_dir: &Path,
    stem: &str,
    has_task_tool: bool,
) -> Truncated {
    let mut t = truncate_inner(output, spill_dir, stem);
    if t.truncated {
        if !t.text.ends_with('\n') {
            t.text.push('\n');
        }
        t.text.push_str(truncation_hint(has_task_tool));
        t.text.push('\n');
    }
    t
}

/// Head + tail truncation with an exact omission report, spilling the complete
/// output to `spill_dir` when cut (DSH locator pattern: the model gets a path it
/// can `read`).
///
/// Why head *and* tail (DSH `output-retention`): tail-only truncation deletes
/// the most informative part of the most common oversized output there is. A
/// 5000-line compiler log keeps its summary at the bottom and the actual first
/// error at the top; a stack trace keeps the exception at the top. Cutting one
/// end makes the model re-run the command to see what it missed. The marker
/// states exactly how many lines and bytes were dropped, so the model can decide
/// whether reading the spill is worth it instead of guessing.
///
/// Safe default: the hint claims only `grep`/`read` ([`truncate_output_with_task`]
/// with `false`). Callers shown to a registry that actually has the `task` tool
/// should pass `true` through [`truncate_output_with_task`] instead — the branch
/// is the caller's truth to tell, not this helper's to guess.
pub fn truncate_output(output: &str, spill_dir: &Path, stem: &str) -> Truncated {
    truncate_output_with_task(output, spill_dir, stem, false)
}

fn truncate_inner(output: &str, spill_dir: &Path, stem: &str) -> Truncated {
    let lines: Vec<&str> = output.lines().collect();
    let total_lines = lines.len();
    let total_bytes = output.len();
    if total_bytes <= MAX_TOOL_OUTPUT_BYTES && total_lines <= MAX_TOOL_OUTPUT_LINES {
        return Truncated {
            text: output.to_string(),
            truncated: false,
            spilled_to: None,
            spill_error: None,
        };
    }

    // Spill FIRST so a locator announced in the text always exists.
    let (spilled_to, spill_error) = match spill_full_output(output, spill_dir, stem) {
        Ok(p) => (Some(p), None),
        Err(e) => (None, Some(e)),
    };

    // Half the budget each end.
    let head_line_budget = MAX_TOOL_OUTPUT_LINES / 2;
    let tail_line_budget = MAX_TOOL_OUTPUT_LINES - head_line_budget;
    let half_bytes = MAX_TOOL_OUTPUT_BYTES / 2;

    let mut head = String::new();
    let mut head_lines = 0usize;
    for l in &lines {
        if head_lines >= head_line_budget {
            break;
        }
        // +1 for the newline the join will re-add.
        if head.len() + l.len() + 1 > half_bytes {
            break;
        }
        head.push_str(l);
        head.push('\n');
        head_lines += 1;
    }

    let mut tail_parts: Vec<&str> = Vec::new();
    let mut tail_bytes = 0usize;
    for l in lines.iter().rev() {
        if tail_parts.len() >= tail_line_budget {
            break;
        }
        if tail_bytes + l.len() + 1 > half_bytes {
            break;
        }
        tail_bytes += l.len() + 1;
        tail_parts.push(l);
    }
    tail_parts.reverse();
    let tail = tail_parts.join("\n");
    let tail_lines = tail_parts.len();

    // Degenerate shape: the cut is by BYTES inside a single line (minified JSON,
    // a base64 blob, one gigantic log record). No whole line fits, so the
    // head+tail windows would both be empty and the model would receive a marker
    // and nothing else. Keep a straight byte window of the head instead; the
    // rest lives in the spill and the marker says so.
    if head_lines == 0 || head_lines + tail_lines >= total_lines {
        let kept = clip_head(output, MAX_TOOL_OUTPUT_BYTES).to_string();
        let mut text = kept.clone();
        text.push_str(&format!(
            "\n[... {total_bytes} bytes in one oversized block; kept the first {} — {}]\n",
            kept.len(),
            spill_sentence(&spilled_to, &spill_error)
        ));
        return Truncated {
            text,
            truncated: true,
            spilled_to,
            spill_error,
        };
    }

    // Exact omission accounting over the dropped slice rather than subtracting
    // retained lengths: `lines()` strips CR, so byte arithmetic on the kept
    // pieces undercounts CRLF output.
    let omitted = &lines[head_lines..total_lines - tail_lines];
    let omitted_lines = omitted.len();
    let omitted_bytes: usize = omitted.iter().map(|l| l.len() + 1).sum();

    let mut text = String::with_capacity(MAX_TOOL_OUTPUT_BYTES + 256);
    text.push_str(&head);
    text.push_str(&omission_marker(
        omitted_lines,
        omitted_bytes,
        &spilled_to,
        &spill_error,
    ));
    text.push_str(&tail);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Truncated {
        text,
        truncated: true,
        spilled_to,
        spill_error,
    }
}

/// `full output saved to X`, or the honest reason there is no such file.
fn spill_sentence(spilled_to: &Option<PathBuf>, spill_error: &Option<String>) -> String {
    match (spilled_to, spill_error) {
        (Some(p), _) => format!("full output saved to {}", p.display()),
        (None, Some(e)) => format!("full output could NOT be saved ({e})"),
        (None, None) => "full output not saved".to_string(),
    }
}

/// The one line that stands in for the dropped middle: exact counts, and either
/// the locator or the honest reason there is none.
fn omission_marker(
    lines: usize,
    bytes: usize,
    spilled_to: &Option<PathBuf>,
    spill_error: &Option<String>,
) -> String {
    format!(
        "\n[... {lines} lines / {bytes} bytes omitted — {}]\n",
        spill_sentence(spilled_to, spill_error)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_traversal_is_contained() {
        // Use a platform-absolute fixture root so absolute-vs-root comparisons behave
        // identically on POSIX and Windows.
        let root = std::env::temp_dir().join("epsilon-util-fixture");
        let (p, inside) = lexically_normalize(&root, "a/../b.txt");
        assert!(inside);
        assert_eq!(p, root.join("b.txt"));

        let (_, inside) = lexically_normalize(&root, "../outside.txt");
        assert!(!inside);

        let elsewhere = if cfg!(windows) {
            r"C:\elsewhere\x"
        } else {
            "/elsewhere/x"
        };
        let (resolved, inside) = lexically_normalize(&root, elsewhere);
        assert!(!inside, "absolute path outside the root must be flagged");
        assert_eq!(resolved, Path::new(elsewhere));
    }

    #[test]
    fn a_symlink_out_of_the_workspace_is_refused_and_named() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("id_rsa"), "PRIVATE KEY").unwrap();
        if !epsilon_core::test_support::dir_link(&outside, &root.join("creds")) {
            eprintln!("skipped: this platform would not create a directory link");
            return;
        }
        let err = resolve_or_refuse(&root, "creds/id_rsa").expect_err("a link out must be refused");
        let text = err.message.clone();
        assert!(text.contains("creds/id_rsa"), "{text}");
        assert!(
            text.contains("link to"),
            "the refusal must explain the symlink, not just repeat the path: {text}"
        );
    }

    #[test]
    fn walker_skips_ignored_dirs_but_keeps_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn a(){}").unwrap();
        fs::write(root.join(".env"), "SECRET=1").unwrap(); // hidden but searchable
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("target/junk.o"), "x").unwrap();
        // A hidden directory is skipped by default.
        fs::create_dir_all(root.join(".cache")).unwrap();
        fs::write(root.join(".cache/blob.bin"), "x").unwrap();

        let files = walk_files(root);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(names.contains(&"src/lib.rs".to_string()));
        assert!(
            names.contains(&".env".to_string()),
            "hidden files stay visible"
        );
        assert!(!names.iter().any(|n| n.starts_with("target/")));
        assert!(
            !names.iter().any(|n| n.starts_with(".cache/")),
            "a hidden directory is skipped by default"
        );
        // NOT asserted, because it does not work yet: a `.gitignore` negation is
        // meant to re-include a hidden directory. The matcher resolves that
        // correctly (`IgnoreSet::is_ignored` walks ancestors), but the walker
        // still does not descend into it. Tracked rather than papered over —
        // `audit/11` carries it as an open item.
    }

    #[test]
    fn walk_files_actually_honours_gitignore() {
        // The promise the grep and find descriptions make. A substring check
        // passes a toy test and then lists every file in a build output.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn a(){}").unwrap();
        fs::write(root.join("notes.log"), "noise").unwrap();
        fs::write(root.join("keep.log"), "signal").unwrap();
        fs::write(root.join(".gitignore"), "*.log\n!keep.log\n").unwrap();

        let names: Vec<String> = walk_files(root)
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(names.contains(&"src/lib.rs".to_string()));
        assert!(
            !names.contains(&"notes.log".to_string()),
            "an ignored file must not be walked: {names:?}"
        );
        assert!(
            names.contains(&"keep.log".to_string()),
            "a negated pattern must re-include: {names:?}"
        );
    }

    /// **The tool walk does not leave the workspace through a link.**
    ///
    /// This is the one that mattered: `grep` with no path argument walks `cwd`, and every path
    /// it then reports is workspace-relative — so the permission layer never raised an
    /// `external_dir` ask and the user never saw one. The only thing that could take it out
    /// of the workspace was a link, and `is_dir()` followed it.
    ///
    /// Named in the project's own review in 2026-09 and left unimplemented until now:
    /// `audit/evidence/f09-path-policy-map.md:26,29`.
    #[test]
    fn the_tool_walk_stops_at_a_link_to_the_outside() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "TOKEN = hunter2").unwrap();

        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("main.rs"), "fn main() {}").unwrap();
        let link = root.path().join("escape");
        if let Err(e) = make_dir_link(outside.path(), &link) {
            panic!(
                "a directory junction needs no elevation; if this fired the test cannot run: {e}"
            );
        }
        // The precondition, asserted: a test that never made the link would pass on the
        // strength of a walk that had nothing to escape through.
        assert!(
            epsilon_core::paths::is_link(&link),
            "the production predicate must call the junction a link, or this test proves nothing"
        );

        let names: Vec<String> = walk_files(root.path())
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(
            !names
                .iter()
                .any(|n| n.contains("secret.txt") || n.contains("escape")),
            "nothing behind a link may reach a tool walk: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "main.rs"),
            "and the real files must still be walked: {names:?}"
        );
    }

    /// A **junction** where privilege is required, a symlink where it is not. On Windows the
    /// junction is the form that matters — it is the one `is_symlink()` does not report, and
    /// the one that needs no elevation.
    #[cfg(unix)]
    fn make_dir_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn make_dir_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        let out = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("mklink /J refused"))
        }
    }

    /// G21: the byte budget shared by `find` and `ls`, tested on its own so both tools'
    /// notices rest on one proven unit. The cut is bytes, not entries: 4 lines of 30 bytes do
    /// not fit 100 bytes, and 3 lines of 30 do.
    #[test]
    fn head_bytes_keeps_only_whole_lines_that_fit() {
        let line30 = "y".repeat(30);
        let four = vec![line30.clone(); 4];
        let (kept, shown) = truncate_head_bytes(&four, 100);
        assert_eq!(
            shown, 3,
            "4 lines of 30 bytes + newlines exceed 100: {kept}"
        );
        assert_eq!(kept.lines().count(), 3);
        assert!(!kept.ends_with('\n'), "the join adds no trailing newline");

        let (all, shown_all) = truncate_head_bytes(&four, 4 * 31);
        assert_eq!(shown_all, 4, "exactly the budget keeps everything: {all}");

        // A first line larger than the whole budget yields nothing rather than a cut path.
        let (none, shown_none) = truncate_head_bytes(&[line30], 10);
        assert_eq!((none.as_str(), shown_none), ("", 0));
    }

    #[test]
    fn truncate_spills_and_notes_locator() {
        let dir = tempfile::tempdir().unwrap();
        let big = (0..3000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let t = truncate_output(&big, dir.path(), "demo");
        assert!(t.truncated);
        assert!(t.text.contains("lines / "), "marker reports the omission");
        assert!(t.spill_error.is_none());
        let spilled = t.spilled_to.expect("spill path");
        assert!(
            std::fs::read_to_string(&spilled)
                .unwrap()
                .contains("line 0"),
            "the spill holds the complete output"
        );
        assert!(
            t.text.contains(&spilled.display().to_string()),
            "the marker names the file that actually exists"
        );

        let small = truncate_output("tiny", dir.path(), "d2");
        assert!(!small.truncated && small.spilled_to.is_none());
    }

    #[test]
    fn keeps_the_head_and_the_tail_of_a_long_log() {
        // The whole point of head+tail: a compiler log's first error is at the
        // TOP and its summary is at the BOTTOM. Tail-only truncation used to
        // delete the diagnostic and leave the model re-running the command.
        let dir = tempfile::tempdir().unwrap();
        let mut lines: Vec<String> = Vec::new();
        lines.push("error[E0308]: mismatched types at src/first.rs:12".to_string());
        for i in 0..4000 {
            lines.push(format!("note: intermediate line {i}"));
        }
        lines.push("aborting due to 1 previous error".to_string());
        let big = lines.join("\n");

        let t = truncate_output(&big, dir.path(), "build");
        assert!(t.truncated);
        assert!(
            t.text.contains("error[E0308]"),
            "the head (first error) survives: {}",
            &t.text[..t.text.len().min(200)]
        );
        assert!(
            t.text.contains("aborting due to 1 previous error"),
            "the tail (summary) survives"
        );
        assert!(t.text.contains("omitted"), "the gap is named, not silent");
        assert!(
            t.text.len() <= MAX_TOOL_OUTPUT_BYTES + 512,
            "the retained window stays inside the byte cap"
        );
    }

    #[test]
    fn every_retained_window_is_char_boundary_safe() {
        // Multi-byte content at the cut points must not panic or split a char.
        let dir = tempfile::tempdir().unwrap();
        let big = "好".repeat(60_000); // 180 KB of CJK, one line
        let t = truncate_output(&big, dir.path(), "cjk");
        assert!(t.truncated);
        assert!(t.text.contains("好"), "content survives the clip");
        assert!(t.text.is_char_boundary(t.text.len()));

        // Many multi-byte lines exercise the head and tail loops at once.
        let many: Vec<String> = (0..4000).map(|i| format!("🌍 line {i} 好")).collect();
        let t2 = truncate_output(&many.join("\n"), dir.path(), "cjk2");
        assert!(t2.truncated);
        assert!(t2.text.contains("🌍 line 0"));
        assert!(t2.text.contains("🌍 line 3999"));
    }

    #[test]
    fn a_failed_spill_is_reported_instead_of_promised() {
        // Point the spill at a path that cannot be a directory (an existing
        // FILE) so creation fails. The tool result must say the full output was
        // NOT saved rather than naming a file that does not exist.
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, "x").unwrap();
        let big = (0..3000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let t = truncate_output(&big, &blocker, "demo");
        assert!(t.truncated);
        assert!(t.spilled_to.is_none());
        assert!(t.spill_error.is_some(), "the failure is carried out");
        assert!(
            t.text.contains("could NOT be saved"),
            "the model is told the locator is missing: {}",
            &t.text[t.text.len().saturating_sub(200)..]
        );
        assert!(!t.text.contains("saved to"), "no promise of a file");
    }

    #[test]
    fn spill_files_are_created_exclusively_under_a_private_dir() {
        let dir = tempfile::tempdir().unwrap();
        let spill = dir.path().join("spill");
        let big = (0..3000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let a = truncate_output(&big, &spill, "same");
        let b = truncate_output(&big, &spill, "same");
        assert_ne!(
            a.spilled_to, b.spilled_to,
            "concurrent spills must not overwrite each other"
        );
        assert!(spill.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&spill).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "spill root is private");
        }
    }

    #[test]
    fn oversized_hint_delegates_via_task_when_present() {
        // opencode `truncate.ts:129-131`: with a task tool the hint delegates
        // to a subagent; the wording must name Epsilon's actual `task` surface
        // (`task.rs`: `task` tool, `subagent_type`, `grep`/`read` with
        // offset/limit) — not opencode's "explore agent".
        let dir = tempfile::tempdir().unwrap();
        let big = (0..3000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let t = truncate_output_with_task(&big, dir.path(), "demo", true);
        assert!(t.truncated, "the fixture must actually truncate");
        assert!(
            t.text.contains("task"),
            "with-task hint names the task tool: {}",
            tail(&t.text)
        );
        assert!(
            t.text.to_lowercase().contains("subagent")
                || t.text.to_lowercase().contains("delegate"),
            "with-task hint delegates instead of reading inline: {}",
            tail(&t.text)
        );
    }

    #[test]
    fn oversized_hint_without_task_claims_only_read_grep() {
        // Negative: without a task tool the hint must not invent one — only
        // the tools that exist on every registry (`grep`, `read`).
        let dir = tempfile::tempdir().unwrap();
        let big = (0..3000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let t = truncate_output_with_task(&big, dir.path(), "demo", false);
        assert!(t.truncated, "the fixture must actually truncate");
        let lower = t.text.to_lowercase();
        assert!(
            lower.contains("grep") && lower.contains("read"),
            "without-task hint points at grep/read: {}",
            tail(&t.text)
        );
        assert!(
            !lower.contains("task tool")
                && !lower.contains("subagent")
                && !lower.contains("delegate"),
            "without-task hint must not claim delegation: {}",
            tail(&t.text)
        );
    }

    fn tail(text: &str) -> String {
        text.chars()
            .rev()
            .take(600)
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    }
}
