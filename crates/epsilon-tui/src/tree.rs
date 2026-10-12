//! Workspace tree sidebar (Phase 3, TUI as IDE).
//!
//! Pure row-building over the shared core walk
//! ([`epsilon_core::walk::walk_files`]) — the traversal `at_files.rs` says the
//! editor's tree wants with its own budget. This module owns the budget, the
//! visible-row computation and the draw; key handling lives in `app/keys.rs`.
//!
//! Paths are workspace-relative with `/` separators (the walk's own spelling),
//! so no conversion happens at this boundary.

use std::collections::{BTreeMap, BTreeSet};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

/// The tree's own row budget from the shared traversal: larger than `@`
/// completion's 200 (a tree shows a workspace, a popup ranks a keystroke),
/// still bounded so a huge tree cannot eat the toggle.
pub const TREE_ROW_BUDGET: usize = 500;

/// Lines a viewer keeps from one file: the whole small file, the head of a big one.
/// A viewer pages text for reading, it is not an editor buffer — and an unbounded
/// read hands one pasted log file the whole frame budget on every draw.
pub const VIEWER_LINE_CAP: usize = 2000;

/// One visible row: a directory derived from path prefixes, or a walked file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// Workspace-relative path, `/`-joined; for dirs without a trailing slash.
    pub rel: String,
    /// Final component, as drawn.
    pub name: String,
    /// Nesting depth: top-level rows draw at zero indent.
    pub depth: usize,
    /// Directories expand and collapse; files open (insert `@path`).
    pub is_dir: bool,
}

/// Visible rows for `paths` with `collapsed` directories folded away.
///
/// Directories are derived from file-path prefixes (the walk yields files
/// only) and rendered depth-first, siblings sorted directories-first then by
/// name; everything starts expanded and `collapsed` names the exceptions.
/// Deterministic by construction (`BTreeMap`), so the tree never jumps between
/// refreshes and a test is not flaky rather than wrong.
pub fn build_visible_rows(paths: &[String], collapsed: &BTreeSet<String>) -> Vec<TreeRow> {
    let mut is_dir: BTreeMap<String, bool> = BTreeMap::new();
    for p in paths {
        let parts: Vec<&str> = p.split('/').collect();
        let mut prefix = String::new();
        for part in &parts[..parts.len().saturating_sub(1)] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            is_dir.entry(prefix.clone()).or_insert(true);
        }
        // A walked path is a file; a prefix that is also walked whole stays a dir.
        if !is_dir.contains_key(p) {
            is_dir.insert(p.clone(), false);
        }
    }
    let mut kids: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for rel in is_dir.keys() {
        kids.entry(parent_of(rel)).or_default().push(rel.clone());
    }
    for v in kids.values_mut() {
        v.sort_by(|a, b| {
            let da = is_dir.get(a).copied().unwrap_or(false);
            let db = is_dir.get(b).copied().unwrap_or(false);
            db.cmp(&da)
                .then_with(|| short_name(a).cmp(short_name(b)))
                .then_with(|| a.cmp(b))
        });
    }
    let mut out = Vec::new();
    render_level(&kids, &is_dir, collapsed, String::new(), 0, &mut out);
    out
}

/// Short name: the final component.
fn short_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// Depth-first rows under `parent`, folding collapsed directories (whose own
/// row stays visible; only their children hide).
fn render_level(
    kids: &BTreeMap<String, Vec<String>>,
    is_dir: &BTreeMap<String, bool>,
    collapsed: &BTreeSet<String>,
    parent: String,
    depth: usize,
    out: &mut Vec<TreeRow>,
) {
    let Some(list) = kids.get(&parent) else {
        return;
    };
    for rel in list {
        let dir = is_dir.get(rel).copied().unwrap_or(false);
        out.push(TreeRow {
            depth,
            name: short_name(rel).to_string(),
            rel: rel.clone(),
            is_dir: dir,
        });
        if dir && !collapsed.contains(rel) {
            render_level(kids, is_dir, collapsed, rel.clone(), depth + 1, out);
        }
    }
}

fn parent_of(rel: &str) -> String {
    match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    }
}

/// Draw the tree into `rect`: bordered box, cursor row marked, empty workspace
/// named rather than blank. One function for both placements (sidebar rect when
/// wide, full area when narrow) so the two cannot drift apart.
pub(crate) fn draw_tree(
    f: &mut Frame<'_>,
    rect: Rect,
    rows: &[TreeRow],
    cursor: usize,
    capped: bool,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from("  (empty workspace)"));
    }
    for (i, row) in rows.iter().enumerate() {
        let indent = "  ".repeat(row.depth);
        let marker = if i == cursor { "> " } else { "  " };
        let label = if row.is_dir {
            format!("{marker}{indent}{}/", row.name)
        } else {
            format!("{marker}{indent}{}", row.name)
        };
        lines.push(Line::from(label));
    }
    if capped {
        lines.push(Line::from(format!(
            "  ... capped at {TREE_ROW_BUDGET} rows"
        )));
    }
    let block = Block::default().borders(Borders::ALL).title("tree");
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(paths: &[&str], collapsed: &[&str]) -> Vec<TreeRow> {
        let owned: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        let shut: BTreeSet<String> = collapsed.iter().map(|s| s.to_string()).collect();
        build_visible_rows(&owned, &shut)
    }

    #[test]
    fn nested_paths_produce_dir_rows_above_their_files() {
        let got = rows(&["main.rs", "src/lib.rs", "src/util.rs"], &[]);
        let rels: Vec<&str> = got.iter().map(|r| r.rel.as_str()).collect();
        assert_eq!(rels, vec!["src", "src/lib.rs", "src/util.rs", "main.rs"]);
        assert!(got[0].is_dir);
        assert_eq!(got[0].depth, 0);
        assert_eq!(got[1].depth, 1);
        assert!(!got[3].is_dir);
    }

    #[test]
    fn a_collapsed_dir_keeps_its_row_and_hides_its_children() {
        let got = rows(&["main.rs", "src/lib.rs", "src/nested/deep.rs"], &["src"]);
        let rels: Vec<&str> = got.iter().map(|r| r.rel.as_str()).collect();
        assert_eq!(rels, vec!["src", "main.rs"]);
    }

    #[test]
    fn collapsing_a_nested_dir_hides_only_its_subtree() {
        let got = rows(&["src/a.rs", "src/nested/deep.rs"], &["src/nested"]);
        let rels: Vec<&str> = got.iter().map(|r| r.rel.as_str()).collect();
        assert_eq!(rels, vec!["src", "src/nested", "src/a.rs"]);
    }

    #[test]
    fn empty_workspace_is_no_rows_not_a_panic() {
        assert!(rows(&[], &[]).is_empty());
    }

    #[test]
    fn row_order_is_stable_and_dirs_come_first() {
        let a = rows(&["b.rs", "a.rs", "src/z.rs", "src/a.rs"], &[]);
        let b = rows(&["src/a.rs", "a.rs", "src/z.rs", "b.rs"], &[]);
        assert_eq!(a, b);
        let rels: Vec<&str> = a.iter().map(|r| r.rel.as_str()).collect();
        assert_eq!(rels, vec!["src", "src/a.rs", "src/z.rs", "a.rs", "b.rs"]);
    }
}

/// A read-only file view: the workspace-relative path, its text lines, the
/// first visible line, and whether the file continued past the cap.
#[derive(Debug, Clone)]
pub struct FileViewer {
    /// Workspace-relative path, `/`-joined — the panel title.
    pub rel: String,
    /// Text lines (no trailing newlines), capped at `VIEWER_LINE_CAP`.
    pub lines: Vec<String>,
    /// First visible line index.
    pub scroll: usize,
    /// True when the file held more lines than the cap.
    pub truncated: bool,
    /// Exact bytes at open time: the changed-underneath guard compares against
    /// these, so a file without a trailing newline does not read as changed.
    pub opened_bytes: Vec<u8>,
    /// Edit mode (Phase 3 editor slice): the buffer below is live.
    pub editing: bool,
    /// Editable copy of the file; written back on save.
    pub edit_lines: Vec<String>,
    /// Cursor row within `edit_lines`.
    pub edit_row: usize,
    /// Cursor column within the row, in characters.
    pub edit_col: usize,
    /// First Esc with a dirty buffer arms; the second discards.
    pub discard_armed: bool,
}

/// Read `rel` under `root` for viewing. Refuses (rather than guessing) when the
/// file cannot be read as text or is a directory — a viewer that showed decoded
/// garbage or a listing would be a different feature wearing this one's name.
pub fn open_viewer(root: &std::path::Path, rel: &str) -> Result<FileViewer, String> {
    let path = root.join(rel);
    if path.is_dir() {
        return Err(format!("{rel} is a directory, not a file"));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {rel}: {e}"))?;
    if bytes.contains(&0) {
        return Err(format!("{rel} is not text"));
    }
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<String> = text.lines().map(str::to_string).collect();
    let truncated = all.len() > VIEWER_LINE_CAP;
    let lines = all.into_iter().take(VIEWER_LINE_CAP).collect();
    Ok(FileViewer {
        rel: rel.to_string(),
        lines,
        scroll: 0,
        truncated,
        opened_bytes: bytes,
        editing: false,
        edit_lines: Vec::new(),
        edit_row: 0,
        edit_col: 0,
        discard_armed: false,
    })
}

impl FileViewer {
    /// Enter edit mode: the buffer starts as the file on screen.
    pub fn start_edit(&mut self) {
        self.editing = true;
        self.edit_lines = self.lines.clone();
        if self.edit_lines.is_empty() {
            self.edit_lines.push(String::new());
        }
        self.edit_row = 0;
        self.edit_col = 0;
        self.discard_armed = false;
    }

    /// True when the buffer differs from what was opened.
    pub fn is_dirty(&self) -> bool {
        self.editing && self.edit_lines != self.lines
    }

    /// Byte index of character `col` in `line` (clamped at line end).
    fn byte_idx(line: &str, col: usize) -> usize {
        line.char_indices()
            .nth(col)
            .map(|(i, _)| i)
            .unwrap_or(line.len())
    }

    /// Insert `c` at the cursor; grapheme clusters are never split because
    /// movement and edits share character indices, not byte offsets.
    pub fn insert_char(&mut self, c: char) {
        if self.edit_row >= self.edit_lines.len() {
            return;
        }
        let line = &mut self.edit_lines[self.edit_row];
        let at = Self::byte_idx(line, self.edit_col);
        line.insert(at, c);
        self.edit_col += 1;
        self.discard_armed = false;
    }

    /// Split the line at the cursor (Enter).
    pub fn split_line(&mut self) {
        if self.edit_row >= self.edit_lines.len() {
            return;
        }
        let line = &mut self.edit_lines[self.edit_row];
        let at = Self::byte_idx(line, self.edit_col);
        let tail = line.split_off(at);
        self.edit_lines.insert(self.edit_row + 1, tail);
        self.edit_row += 1;
        self.edit_col = 0;
        self.discard_armed = false;
    }

    /// Backspace: delete left, or join with the previous line at column zero.
    pub fn backspace(&mut self) {
        if self.edit_row >= self.edit_lines.len() {
            return;
        }
        if self.edit_col > 0 {
            let line = &mut self.edit_lines[self.edit_row];
            let at = Self::byte_idx(line, self.edit_col);
            let prev = line[..at]
                .chars()
                .next_back()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            line.drain(at - prev..at);
            self.edit_col -= 1;
        } else if self.edit_row > 0 {
            let tail = self.edit_lines.remove(self.edit_row);
            self.edit_row -= 1;
            let prev_len = self.edit_lines[self.edit_row].chars().count();
            self.edit_lines[self.edit_row].push_str(&tail);
            self.edit_col = prev_len;
        }
        self.discard_armed = false;
    }

    /// Clamp the cursor after any structural change.
    pub fn clamp_cursor(&mut self) {
        if self.edit_lines.is_empty() {
            self.edit_lines.push(String::new());
        }
        self.edit_row = self.edit_row.min(self.edit_lines.len() - 1);
        let len = self.edit_lines[self.edit_row].chars().count();
        self.edit_col = self.edit_col.min(len);
    }

    /// The buffer as file text (trailing newline, like every text file here).
    pub fn buffer_text(&self) -> String {
        let mut out = self.edit_lines.join("\n");
        out.push('\n');
        out
    }
}

/// Draw the read-only viewer as a centered overlay: title is the path, body is
/// a scroll window over the lines, footer names the keys. Records the visible
/// capacity for the page keys, the same measured-window rule the pickers use.
pub(crate) fn draw_viewer(
    f: &mut Frame<'_>,
    outer: Rect,
    theme: &crate::ui::theme::Theme,
    viewer: &mut FileViewer,
    capacity_out: &mut usize,
) {
    use crate::ui::overlay::{self, Panel, Width};
    let width = Width::PANEL.resolve(outer);
    let height = outer.height.saturating_sub(4).max(10);
    let title = if viewer.editing {
        format!(
            "edit {} — alt+s saves, esc cancels{}",
            viewer.rel,
            if viewer.is_dirty() { " (modified)" } else { "" }
        )
    } else {
        format!("view {}", viewer.rel)
    };
    let panel = Panel {
        theme,
        area: overlay::centered(outer, width, height),
        title: &title,
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    // One line is the footer; the rest is text.
    let capacity = inner.height.saturating_sub(1).max(1) as usize;
    *capacity_out = capacity;
    let body: &[String] = if viewer.editing {
        &viewer.edit_lines
    } else {
        &viewer.lines
    };
    // The cursor stays visible: editing scrolls the window after itself.
    if viewer.editing {
        if viewer.edit_row < viewer.scroll {
            viewer.scroll = viewer.edit_row;
        } else if viewer.edit_row >= viewer.scroll + capacity {
            viewer.scroll = viewer.edit_row + 1 - capacity;
        }
    }
    let start = viewer.scroll.min(body.len().saturating_sub(1));
    let end = (start + capacity).min(body.len());
    let mut lines: Vec<Line<'static>> = Vec::new();
    if body.is_empty() {
        lines.push(Line::from(ratatui::text::Span::styled(
            "(empty file)",
            theme.dim(),
        )));
    }
    for line in body.iter().take(end).skip(start) {
        // Plain text, unstyled: a viewer shows the file, it does not interpret
        // it — markdown stays markdown, and a width fit keeps long lines from
        // pushing the panel wider than the rect it was given.
        lines.push(Line::from(crate::wrap::fit_width(
            line,
            inner.width as usize,
        )));
    }
    if viewer.truncated && !viewer.editing {
        lines.push(Line::from(ratatui::text::Span::styled(
            format!("... showing the first {VIEWER_LINE_CAP} lines"),
            theme.dim(),
        )));
    }
    // Pad to the box so short files do not leave the footer floating.
    while lines.len() < capacity {
        lines.push(Line::from(""));
    }
    let hints: &[(&str, &str)] = if viewer.editing {
        &[("type/enter", "edit"), ("alt+s", "save"), ("esc", "cancel")]
    } else {
        &[("↑↓/pgup/pgdn", "scroll"), ("e", "edit"), ("esc", "close")]
    };
    lines.push(Line::from(overlay::hint_row(theme, hints).spans));
    f.render_widget(Paragraph::new(lines), inner);
    // The caret, in panel coordinates from the character column.
    if viewer.editing
        && viewer.edit_row >= start
        && viewer.edit_row < start + capacity
        && let Some(line) = viewer.edit_lines.get(viewer.edit_row)
    {
        let col_cells: usize = line
            .chars()
            .take(viewer.edit_col)
            .map(|c| crate::ui::layout::display_width(&c.to_string()))
            .sum();
        f.set_cursor_position((
            inner.x + col_cells.min(inner.width as usize - 1) as u16,
            inner.y + (viewer.edit_row - start) as u16,
        ));
    }
}

#[cfg(test)]
mod viewer_tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn an_empty_file_opens_with_no_lines_not_an_error() {
        let dir = workspace();
        std::fs::write(dir.path().join("empty.txt"), "").unwrap();
        let viewer = open_viewer(dir.path(), "empty.txt").expect("empty opens");
        assert!(viewer.lines.is_empty());
        assert!(!viewer.truncated);
    }

    #[test]
    fn a_long_file_is_capped_and_says_so() {
        let dir = workspace();
        let big: String = (0..VIEWER_LINE_CAP + 50)
            .map(|i| format!("line {i}\n"))
            .collect();
        std::fs::write(dir.path().join("big.log"), &big).unwrap();
        let viewer = open_viewer(dir.path(), "big.log").expect("big opens");
        assert_eq!(viewer.lines.len(), VIEWER_LINE_CAP);
        assert!(viewer.truncated, "the cap must be announced, not silent");
    }

    #[test]
    fn a_directory_and_a_binary_are_refused_not_shown() {
        let dir = workspace();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        assert!(open_viewer(dir.path(), "sub").is_err());
        std::fs::write(dir.path().join("bin"), [0u8, 1, 2, 3]).unwrap();
        let err = open_viewer(dir.path(), "bin").expect_err("binary refuses");
        assert!(err.contains("not text"), "refusal names the reason: {err}");
    }

    #[test]
    fn a_missing_file_is_a_named_error() {
        let dir = workspace();
        let err = open_viewer(dir.path(), "gone.txt").expect_err("missing refuses");
        assert!(err.contains("gone.txt"), "error names the file: {err}");
    }
}
