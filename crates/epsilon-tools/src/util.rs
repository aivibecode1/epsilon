//! Shared plumbing for built-in tools: workspace-boundary checks, directory walking,
//! and output truncation with spill-to-disk locators.

use std::fs;
use std::io::Read as _;
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
pub use epsilon_core::paths::resolve_in_workspace;

/// The failure every tool returns when a path leaves the declared workspace.
pub fn escaped_workspace(raw: &str) -> ToolFailure {
    ToolFailure::msg(format!(
        "Refused: '{raw}' resolves outside the workspace. \
         Operate on paths inside the working directory."
    ))
}

/// Recursively collect files under `root`, honoring [`SKIP_DIRS`] and entry/depth caps.
/// Returns paths relative to `root` (forward-slash separated for stable rule matching).
pub fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut budget = WALK_MAX_ENTRIES;
    walk_inner(root, root, 0, &mut out, &mut budget);
    out
}

fn walk_inner(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>, budget: &mut usize) {
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
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) || name.starts_with('.') {
                continue;
            }
            walk_inner(root, &path, depth + 1, out, budget);
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

/// Read a file's bytes, refusing probable binaries (NUL in the first 4 KiB).
pub fn read_text_capped(path: &Path) -> Result<String, ToolFailure> {
    read_text_capped_ex(path).map(|(text, _)| text)
}

/// As [`read_text_capped`], but also reports whether the file was longer than the byte cap.
///
/// The caller needs to know: a truncated read is a *partial view*, and the read tool both says
/// so in its output and marks the path as seen-only-in-part (`crate::read_log`). Without the
/// flag the cap was silent, which made a 1 MB file look fully read.
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

    /// Whatever has arrived so far (used by tests and by a final flush).
    pub fn current_tail(&self) -> String {
        self.tail.lock().map(|b| b.clone()).unwrap_or_default()
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
pub fn truncate_output(output: &str, spill_dir: &Path, stem: &str) -> Truncated {
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
        let (p, inside) = resolve_in_workspace(&root, "a/../b.txt");
        assert!(inside);
        assert_eq!(p, root.join("b.txt"));

        let (_, inside) = resolve_in_workspace(&root, "../outside.txt");
        assert!(!inside);

        let elsewhere = if cfg!(windows) {
            r"C:\elsewhere\x"
        } else {
            "/elsewhere/x"
        };
        let (resolved, inside) = resolve_in_workspace(&root, elsewhere);
        assert!(!inside, "absolute path outside the root must be flagged");
        assert_eq!(resolved, Path::new(elsewhere));
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
}
