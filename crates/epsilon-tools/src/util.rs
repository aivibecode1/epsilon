//! Shared plumbing for built-in tools: workspace-boundary checks, directory walking,
//! and output truncation with spill-to-disk locators.

use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

use epsilon_contracts::tool::ToolFailure;

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

/// Resolve `raw` against `root`, normalizing `.`/`..` WITHOUT touching the filesystem,
/// then report whether the result stayed inside the workspace.
///
/// Lexical normalization only: symlink escapes are a Layer-2 (sandbox) concern, not ours.
pub fn resolve_in_workspace(root: &Path, raw: &str) -> (PathBuf, bool) {
    let candidate = Path::new(raw);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let mut normalized = PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::ParentDir => {
                if !normalized.pop() {
                    // Escaping above the root at join time: mark outside immediately.
                    return (joined, false);
                }
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    let inside = normalized.starts_with(root);
    (normalized, inside)
}

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

/// Read a file's bytes, refusing probable binaries (NUL in the first 4 KiB).
pub fn read_text_capped(path: &Path) -> Result<String, ToolFailure> {
    let f = fs::File::open(path)
        .map_err(|e| ToolFailure::msg(format!("cannot open {}: {e}", path.display())))?;
    let mut buf = Vec::new();
    f.take(MAX_READ_BYTES as u64)
        .read_to_end(&mut buf)
        .map_err(|e| ToolFailure::msg(format!("read error: {e}")))?;
    if buf.iter().take(4096).any(|b| *b == 0) {
        return Err(ToolFailure::msg("refused: file looks binary"));
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Outcome of truncating oversized tool output.
pub struct Truncated {
    /// What the model sees.
    pub text: String,
    /// True when content was cut.
    pub truncated: bool,
    /// Where the FULL output was spilled (when truncated).
    pub spilled_to: Option<PathBuf>,
}

/// Tail-truncate output to the line/byte caps, spilling the complete version to
/// `spill_dir` when cut (DSH locator pattern: the model gets a path it can `read`).
pub fn truncate_output(output: &str, spill_dir: &Path, stem: &str) -> Truncated {
    let over_bytes = output.len() > MAX_TOOL_OUTPUT_BYTES;
    let line_count = output.lines().count();
    let over_lines = line_count > MAX_TOOL_OUTPUT_LINES;

    if !over_bytes && !over_lines {
        return Truncated {
            text: output.to_string(),
            truncated: false,
            spilled_to: None,
        };
    }

    let _ = fs::create_dir_all(spill_dir);
    let spilled = spill_dir.join(format!(
        "{}-{}.out",
        stem,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let _ = fs::write(&spilled, output);

    let kept_lines: Vec<&str> = output
        .lines()
        .skip(line_count.saturating_sub(MAX_TOOL_OUTPUT_LINES))
        .collect();
    let mut text = kept_lines.join("\n");
    // Byte cap second pass keeps the tail within limits even for huge single lines.
    if text.len() > MAX_TOOL_OUTPUT_BYTES {
        // Manual char-boundary walk (floor_char_boundary needs rustc 1.91 > our MSRV).
        let mut cut = text.len() - MAX_TOOL_OUTPUT_BYTES;
        while cut < text.len() && !text.is_char_boundary(cut) {
            cut += 1;
        }
        text = text[cut..].to_string();
    }
    text.push_str(&format!(
        "\n[output truncated: {line_count} lines total; full output saved to {}]",
        spilled.display()
    ));
    Truncated {
        text,
        truncated: true,
        spilled_to: Some(spilled),
    }
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
        assert!(t.text.contains("[output truncated: 3000 lines"));
        assert!(t.spilled_to.is_some());
        assert!(
            std::fs::read_to_string(t.spilled_to.unwrap())
                .unwrap()
                .contains("line 0")
        );

        let small = truncate_output("tiny", dir.path(), "d2");
        assert!(!small.truncated && small.spilled_to.is_none());
    }
}
