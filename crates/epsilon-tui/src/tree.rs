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
    /// Undo history for the edit buffer. The same
    /// [`crate::undo::UndoStack`] the composer uses, over a snapshot shape that
    /// fits a line buffer rather than a single string — see the note on the
    /// generic parameter there.
    pub(super) undo_stack: crate::undo::UndoStack<FileSnapshot>,
    /// Find state (Phase 3 editor slice): `/` opens in view mode, typing narrows.
    pub find_open: bool,
    /// Current find query, plain substring (the transcript search's rule).
    pub find_query: String,
    /// Line indices in `lines` containing the query, in file order.
    pub find_hits: Vec<usize>,
    /// Position within `find_hits`.
    pub find_index: usize,
    /// Replacement input, opened with Tab while find is open. Pairs with
    /// `find_query` so one run replaces what the query just found.
    pub replace_open: bool,
    /// The replacement text, applied to the buffer being edited.
    pub replace_query: String,
}

/// One point-in-time state of the file editor's buffer.
///
/// The mirror of the composer's [`crate::undo::EditorSnapshot`] over a shape that
/// actually matches a file: a line buffer instead of a string, and a `(row, col)`
/// cursor instead of a char index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileSnapshot {
    /// Buffer lines at snapshot time.
    pub(super) lines: Vec<String>,
    /// Cursor row.
    pub(super) row: usize,
    /// Cursor column, in characters.
    pub(super) col: usize,
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
        undo_stack: crate::undo::UndoStack::new(),
        find_open: false,
        find_query: String::new(),
        replace_open: false,
        replace_query: String::new(),
        find_hits: Vec::new(),
        find_index: 0,
    })
}

impl FileViewer {
    /// Enter edit mode: the buffer starts as the file on screen. Refuses
    /// (returns false and changes nothing) when the file continued past
    /// `VIEWER_LINE_CAP`: the buffer would be the capped head, and saving
    /// it would cut the tail off the file.
    pub fn start_edit(&mut self) -> bool {
        if self.truncated {
            return false;
        }
        self.editing = true;
        self.edit_lines = self.lines.clone();
        if self.edit_lines.is_empty() {
            self.edit_lines.push(String::new());
        }
        self.edit_row = 0;
        self.edit_col = 0;
        self.discard_armed = false;
        // A fresh edit session starts with a fresh history: the pre-existing
        // state IS the baseline the user undoes back to, so there is nothing to
        // undo until they change something.
        self.undo_stack = crate::undo::UndoStack::new();
        true
    }

    /// Capture the buffer as an undo snapshot.
    fn snapshot(&self) -> FileSnapshot {
        FileSnapshot {
            lines: self.edit_lines.clone(),
            row: self.edit_row,
            col: self.edit_col,
        }
    }

    /// Record the state BEFORE a mutation. Called by every primitive below; the
    /// duplicate suppression in [`crate::undo::UndoStack`] keeps per-keystroke
    /// calls cheap.
    pub(super) fn push_undo(&mut self) {
        self.undo_stack.push(self.snapshot());
    }

    /// Undo the last edit. Returns false when there is nothing to undo, which
    /// is the case the caller must report rather than swallow.
    pub fn undo(&mut self) -> bool {
        let Some(snap) = self.undo_stack.undo(self.snapshot()) else {
            return false;
        };
        self.restore(snap);
        true
    }

    /// Re-apply the most recently undone edit.
    pub fn redo(&mut self) -> bool {
        let Some(snap) = self.undo_stack.redo(self.snapshot()) else {
            return false;
        };
        self.restore(snap);
        true
    }

    fn restore(&mut self, snap: FileSnapshot) {
        self.edit_lines = snap.lines;
        self.edit_row = snap.row;
        self.edit_col = snap.col;
        self.clamp_cursor();
        self.discard_armed = false;
    }

    /// True when the buffer differs from what was opened.
    pub fn is_dirty(&self) -> bool {
        self.editing && self.edit_lines != self.lines
    }

    /// True when the tab holds unsaved edits outside edit mode (Phase 3,
    /// tabs): leaving edit mode keeps the buffer, so closing the tab must
    /// ask even though `is_dirty` (an edit-mode predicate) is false.
    pub fn has_unsaved(&self) -> bool {
        !self.editing && !self.edit_lines.is_empty() && self.edit_lines != self.lines
    }

    /// Open find: the query starts empty, showing no matches until typed.
    pub fn find_open(&mut self) {
        self.find_open = true;
        self.find_query.clear();
        self.find_hits.clear();
        self.find_index = 0;
    }

    /// Close find and drop its state; the viewer stays open.
    pub fn find_close(&mut self) {
        self.find_open = false;
        self.find_query.clear();
        self.find_hits.clear();
        self.find_index = 0;
        self.replace_open = false;
        self.replace_query.clear();
    }

    /// Tab while find is open: move the caret to the replacement input, and back.
    /// No new chord — Tab is already the key that walks a form.
    pub fn replace_toggle(&mut self) {
        self.replace_open = !self.replace_open;
        if !self.replace_open {
            self.replace_query.clear();
        }
    }

    /// Replace every hit, in the buffer being edited.
    ///
    /// The snapshot is taken **before** the mutation and pushed only when the
    /// text will actually change: a snapshot taken afterwards restores the
    /// replaced text and Ctrl+Z appears to do nothing, and a no-op replace pushes
    /// a snapshot that poisons the redo rail.
    pub fn replace_all(&mut self) -> usize {
        if self.find_query.is_empty() || self.find_hits.is_empty() {
            return 0;
        }
        let source = if self.editing {
            &self.edit_lines
        } else {
            &self.lines
        };
        let mut n = 0usize;
        for i in &self.find_hits {
            if let Some(line) = source.get(*i)
                && line.contains(self.find_query.as_str())
            {
                n += line.matches(self.find_query.as_str()).count();
            }
        }
        if n == 0 {
            return 0;
        }
        self.push_undo();
        let query = self.find_query.clone();
        let repl = self.replace_query.clone();
        let lines = if self.editing {
            &mut self.edit_lines
        } else {
            &mut self.lines
        };
        for i in &self.find_hits {
            if let Some(line) = lines.get_mut(*i) {
                *line = line.replace(query.as_str(), repl.as_str());
            }
        }
        self.find_recompute();
        n
    }

    /// Recompute matches for the current query (plain substring, file order).
    /// Selection restarts at the first match: the list it pointed into is gone.
    ///
    /// **It searches the buffer being edited when in edit mode.** Find used to
    /// read `lines` unconditionally, so the keys were wired to a buffer the user
    /// was not typing into — and unreachable besides, because the edit-mode key
    /// arm returns before the find arm ever runs. Searching `edit_lines` here is
    /// what makes the same query answer "where is this in *my* text".
    pub fn find_recompute(&mut self) {
        let source: &[String] = if self.editing {
            &self.edit_lines
        } else {
            &self.lines
        };
        self.find_hits = if self.find_query.is_empty() {
            Vec::new()
        } else {
            source
                .iter()
                .enumerate()
                .filter(|(_, l)| l.contains(self.find_query.as_str()))
                .map(|(i, _)| i)
                .collect()
        };
        self.find_index = 0;
        // Keep the selection visible when there is one to show.
        if let Some(&line) = self.find_hits.first() {
            self.scroll = line.min(self.scroll);
        }
    }

    /// Step the selection through the matches, wrapping at both ends, with
    /// the scroll following minimally so the match stays on screen.
    /// `capacity` is the painted screenful; `max(1)` so a step before the
    /// first paint still moves rather than silently doing nothing.
    ///
    /// In edit mode the step also **moves the cursor** to the match, because a
    /// highlighted row the user cannot reach the caret of is a decoration. It
    /// lands on the first occurrence of the query in that line — the same
    /// substring the hit was computed from.
    pub fn find_step(&mut self, forward: bool, capacity: usize) {
        if self.find_hits.is_empty() {
            return;
        }
        let n = self.find_hits.len();
        if forward {
            self.find_index = (self.find_index + 1) % n;
        } else {
            self.find_index = (self.find_index + n - 1) % n;
        }
        let cap = capacity.max(1);
        let line = self.find_hits[self.find_index];
        if line < self.scroll {
            self.scroll = line;
        } else if line >= self.scroll + cap {
            self.scroll = line + 1 - cap;
        }
        if self.editing {
            self.edit_row = line.min(self.edit_lines.len().saturating_sub(1));
            self.edit_col = self.edit_lines[self.edit_row]
                .find(self.find_query.as_str())
                .map(|b| self.edit_lines[self.edit_row][..b].chars().count())
                .unwrap_or(0);
            self.discard_armed = false;
        }
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
        self.push_undo();
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
        self.push_undo();
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
        self.push_undo();
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

    /// Delete forward: the mirror of [`Self::backspace`]. Removes the character
    /// under the cursor, or joins the next line onto this one at end of line.
    ///
    /// **This key was silently discarded before.** The edit-mode match fell
    /// through to `_ => {}`, so `Delete` did nothing at all — not even a notice
    /// — while `Backspace` worked, which is the shape of a half-implemented
    /// editor: every key a user has used in every other editor, minus one.
    pub fn delete_forward(&mut self) {
        if self.edit_row >= self.edit_lines.len() {
            return;
        }
        let line_len = self.edit_lines[self.edit_row].chars().count();
        if self.edit_col < line_len {
            self.push_undo();
            let line = &mut self.edit_lines[self.edit_row];
            let at = Self::byte_idx(line, self.edit_col);
            let next = line[at..].chars().next().map(|c| c.len_utf8()).unwrap_or(0);
            line.drain(at..at + next);
            self.discard_armed = false;
        } else if self.edit_row + 1 < self.edit_lines.len() {
            self.push_undo();
            let tail = self.edit_lines.remove(self.edit_row + 1);
            self.edit_lines[self.edit_row].push_str(&tail);
            self.discard_armed = false;
        }
        // At the very end of the buffer there is nothing to delete: a no-op, not
        // an error, and the cursor does not move.
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
/// a scroll window over the lines, footer names the keys. **Returns** the
/// visible capacity the page keys step by, the same measured-window rule the
/// pickers use — a page key arrives in the key handler, which knows no
/// geometry, so the paint is what teaches it how far a page is.
///
/// `outer` is the band the panel may occupy — the rows between the header and
/// the footer — and the whole of it is cleared before the frame goes on. A
/// panel narrower than that band would otherwise leave the live screen visible
/// down both sides, sliced in half by the panel's rails: the transcript read
/// `a coding` where it had said `…a coding agent…`, and the editor looked like
/// something drawn *over* a broken screen rather than instead of one.
///
/// `undo_key` is the chord for `tui.editor.undo`, resolved by the caller from
/// the live binding table: a rebinding moves the hint the footer prints, and an
/// id that does not resolve prints nothing rather than a chord the handler
/// would refuse.
pub(crate) fn draw_viewer(
    f: &mut Frame<'_>,
    outer: Rect,
    theme: &crate::ui::theme::Theme,
    undo_key: &str,
    viewer: &mut FileViewer,
    tab_strip: &[(String, bool)],
    active_tab: usize,
) -> usize {
    use crate::ui::overlay::{self, Panel, Width};
    f.render_widget(ratatui::widgets::Clear, outer);
    let width = Width::PANEL.resolve(outer);
    let height = outer.height.saturating_sub(4).max(10);
    let title = if viewer.editing {
        format!(
            "edit {} — {}+s saves, esc cancels{}",
            viewer.rel,
            crate::keybindings::alt_name(),
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
    // **Reserve the rows the input lines will occupy, before the body takes them.**
    //
    // Measured, in edit mode with find and replace both open: the body filled
    // `capacity` rows, the find line was drawn last inside the panel, and *both*
    // the replacement line and the hint row were clipped off the bottom. An earlier
    // version of this reservation was built and then controlled away — correctly,
    // because the control it was tested against exercised only the view-mode state,
    // where one spare row happens to fit the find line. With three input rows there
    // is no spare, and the reservation is what makes the query and the replacement
    // reachable instead of off-screen.
    let input_rows = usize::from(viewer.find_open) + usize::from(viewer.replace_open) + 1; // the hint row, always
    let body_rows = capacity.saturating_sub(input_rows).max(1);
    let end = (start + body_rows).min(body.len());
    let mut lines: Vec<Line<'static>> = Vec::new();
    // Phase 3 tabs: the strip names every open tab when there is more than
    // one — one tab needs no chrome. Active marked, unsaved dotted, so the
    // shape of the session is visible before a single key is pressed.
    if tab_strip.len() > 1 {
        let mut spans: Vec<ratatui::text::Span<'static>> = Vec::new();
        for (i, (rel, dirty)) in tab_strip.iter().enumerate() {
            if i > 0 {
                spans.push(ratatui::text::Span::styled(" | ", theme.dim()));
            }
            let mut name = rel.clone();
            if *dirty {
                name.push_str(" \u{25cf}");
            }
            if i == active_tab {
                spans.push(ratatui::text::Span::styled(name, theme.assistant()));
            } else {
                spans.push(ratatui::text::Span::styled(name, theme.dim()));
            }
        }
        lines.push(Line::from(spans));
    }
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
    while lines.len() < body_rows {
        lines.push(Line::from(""));
    }
    let save = format!("{}+s", crate::keybindings::alt_name());
    // **Ctrl+Z is bound, and now it says so.** The edit path implements undo
    // (`app.undo` on `tui.editor.undo`) and the hint row named only
    // type/save/cancel, so the one key that takes an edit back was a secret.
    // The chord arrives resolved from the live table (see [`draw_viewer`]), so
    // a user who rebinds undo moves this text with it.
    let undo = undo_key;
    // Phase 3 viewer find, drawn inside the panel above the footer: a query
    // that is not on screen while it is typed cannot be confirmed.
    if viewer.find_open {
        let count = if viewer.find_query.is_empty() {
            "type to search".to_string()
        } else if viewer.find_hits.is_empty() {
            "no matches".to_string()
        } else {
            format!("{} of {}", viewer.find_index + 1, viewer.find_hits.len())
        };
        lines.push(Line::from(vec![
            ratatui::text::Span::styled("/", theme.accent()),
            ratatui::text::Span::raw(viewer.find_query.clone()),
            ratatui::text::Span::styled("\u{2588}", theme.accent()),
            // The same vocabulary the transcript search uses (M157's panel),
            // because `p` is bound here too: `Up`, `PageUp` and `p` all step
            // back (`app/keys.rs`), and a hint row that names only the forward
            // step hides half the keys it describes.
            ratatui::text::Span::styled(
                format!("  {count} · enter/n next · p previous · esc close"),
                theme.dim(),
            ),
        ]));
    }

    // **The replacement line, drawn where it can be seen.**
    // A previous attempt withdrew this feature believing the line could not be
    // drawn; that belief was wrong. With the row budget in place above, all three
    // input rows are drawn. The witness asserts the replacement line lands on the
    // screen rather than only in the data.
    if viewer.editing && viewer.replace_open {
        lines.push(Line::from(vec![
            ratatui::text::Span::styled(">", theme.accent()),
            ratatui::text::Span::raw(viewer.replace_query.clone()),
            ratatui::text::Span::styled("\u{2588}", theme.accent()),
            ratatui::text::Span::styled("  tab back · enter replace all", theme.dim()),
        ]));
    }

    let hints: &[(&str, &str)] = if viewer.editing {
        &[
            ("type/enter", "edit"),
            (&save, "save"),
            (undo, "undo"),
            ("esc", "cancel"),
        ]
    } else if viewer.truncated {
        // Invariant 7: `e` does not edit a capped file (`start_edit`
        // refuses), so the footer must not offer it as edit.
        &[
            ("↑↓/pgup/pgdn", "scroll"),
            ("/", "find"),
            ("e", "edit unavailable"),
            ("esc", "close"),
        ]
    } else {
        &[
            ("↑↓/pgup/pgdn", "scroll"),
            ("/", "find"),
            ("e", "edit"),
            ("esc", "close"),
        ]
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
    capacity
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
    fn editing_a_truncated_file_is_refused() {
        // The edit buffer would be the capped head; saving it would cut the
        // tail off the file. A truncated file must never enter edit mode.
        let dir = workspace();
        let big: String = (0..VIEWER_LINE_CAP + 50)
            .map(|i| format!("line {i}\n"))
            .collect();
        std::fs::write(dir.path().join("big.log"), &big).unwrap();
        let mut viewer = open_viewer(dir.path(), "big.log").expect("big opens");
        assert!(viewer.truncated);
        assert!(
            !viewer.start_edit(),
            "a capped head must not become the edit buffer"
        );
        assert!(!viewer.editing, "a refused edit changes nothing");
        assert!(
            viewer.edit_lines.is_empty(),
            "no buffer may exist for a file the viewer cannot hold"
        );
    }

    #[test]
    fn editing_a_small_file_still_enters() {
        // Control: the refusal above is about the cap, not about editing.
        let dir = workspace();
        std::fs::write(dir.path().join("small.txt"), "one\ntwo\n").unwrap();
        let mut viewer = open_viewer(dir.path(), "small.txt").expect("small opens");
        assert!(!viewer.truncated);
        assert!(viewer.start_edit(), "small files still enter edit mode");
        assert!(viewer.editing, "small files still enter edit mode");
        assert_eq!(viewer.edit_lines, viewer.lines);
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

    #[test]
    fn find_matches_plain_substrings_in_file_order() {
        let dir = workspace();
        std::fs::write(dir.path().join("f.txt"), "apple\nbanana\nApple pie\n").unwrap();
        let mut v = open_viewer(dir.path(), "f.txt").expect("opens");
        v.find_open();
        assert!(v.find_hits.is_empty(), "empty query matches nothing");
        v.find_query = "apple".into();
        v.find_recompute();
        // Plain substring, case-sensitive like the transcript search: the
        // capitalised line does not match.
        assert_eq!(
            v.find_hits,
            vec![0],
            "only the exact-case line: {:?}",
            v.find_hits
        );
        assert_eq!(v.find_index, 0);
    }

    #[test]
    fn find_steps_wrap_and_drag_the_scroll() {
        let dir = workspace();
        let body: String = (0..50).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.path().join("f.txt"), body).unwrap();
        let mut v = open_viewer(dir.path(), "f.txt").expect("opens");
        v.find_query = "line 4".into();
        v.find_recompute();
        // "line 4", "line 40".."line 49": eleven hits in file order.
        assert_eq!(v.find_hits.len(), 11, "{:?}", v.find_hits);
        assert_eq!(v.find_hits[0], 4);
        v.scroll = 0;
        v.find_step(true, 10);
        assert_eq!(v.find_index, 1);
        // Stepping to the last hit then once more wraps to the first.
        for _ in 0..10 {
            v.find_step(true, 10);
        }
        assert_eq!(v.find_index, 0, "forward wraps past the last hit");
        v.find_step(false, 10);
        assert_eq!(v.find_index, 10, "backward wraps past the first hit");
        // The scroll followed the far hit and stays put for a visible one.
        assert!(
            v.scroll <= 49 && 49 < v.scroll + 10,
            "far hit on screen: scroll={}",
            v.scroll
        );
        v.find_close();
        assert!(!v.find_open && v.find_hits.is_empty() && v.find_query.is_empty());
    }

    // ---- the file editor's own undo, and the Delete key that never worked ----
    //
    // Both blocks below are proof of one thing: the file editor had *no* undo at
    // all, and its `Delete` key was silently discarded. The composer had undo
    // since M16; the viewer's edit mode had `Backspace`, `Enter`, the arrows and
    // `Home`/`End`, and the `_ => {}` arm swallowed everything else — including
    // the one delete direction every other editor on the machine has.

    fn edited() -> FileViewer {
        let mut v = FileViewer {
            rel: "t.txt".into(),
            lines: vec!["alpha".into(), "beta".into(), "gamma".into()],
            scroll: 0,
            truncated: false,
            opened_bytes: Vec::new(),
            editing: false,
            edit_lines: Vec::new(),
            edit_row: 0,
            edit_col: 0,
            discard_armed: false,
            undo_stack: crate::undo::UndoStack::new(),
            find_open: false,
            find_query: String::new(),
            replace_open: false,
            replace_query: String::new(),
            find_hits: Vec::new(),
            find_index: 0,
        };
        assert!(v.start_edit());
        v
    }

    /// The half of deletion that never worked. `Delete` fell through to
    /// `_ => {}` in the edit-mode key match, so it did nothing at all while
    /// `Backspace` worked — a user pressing it saw their cursor sit still and
    /// had no way to tell a broken key from a busy frame.
    #[test]
    fn delete_forward_removes_what_is_under_the_cursor() {
        let mut v = edited();
        v.edit_col = 0; // front of "alpha"
        v.delete_forward();
        assert_eq!(v.edit_lines[0], "lpha", "the first character goes");

        v.edit_col = 2; // middle: "lpha" -> "lpa" (the 'h' goes)
        v.delete_forward();
        assert_eq!(v.edit_lines[0], "lpa", "a middle character goes");

        // At end of line it joins the next line, exactly as Backspace joins the
        // previous one — the two are mirrors over the same buffer.
        v.edit_col = v.edit_lines[0].chars().count();
        v.delete_forward();
        assert_eq!(v.edit_lines, vec!["lpabeta", "gamma"]);
        assert_eq!(v.edit_row, 0);
        assert_eq!(v.edit_col, 3);

        // At the very end of the buffer there is nothing to delete: a no-op, and
        // the cursor must not move off the end.
        v.edit_row = v.edit_lines.len() - 1;
        v.edit_col = v.edit_lines[v.edit_row].chars().count();
        let before = v.edit_lines.clone();
        v.delete_forward();
        assert_eq!(v.edit_lines, before, "no-op at the end, not a panic");
    }

    /// And undo brings it back — which is the pair the editor was missing.
    #[test]
    fn delete_forward_is_undoable() {
        let mut v = edited();
        v.edit_col = 0;
        v.delete_forward();
        assert_eq!(v.edit_lines[0], "lpha");
        assert!(v.undo(), "one deletion is one undo");
        assert_eq!(v.edit_lines, vec!["alpha", "beta", "gamma"]);
        assert_eq!(
            (v.edit_row, v.edit_col),
            (0, 0),
            "the cursor comes back too"
        );
        assert!(!v.undo(), "and there is nothing further to undo");
    }

    /// The missing feature: no undo at all in the file editor. Every primitive
    /// pushes a snapshot first, so any sequence of them is one undo step each.
    #[test]
    fn every_edit_is_undoable_and_redo_reapplies_it() {
        let mut v = edited();
        // Type "!" at the end of line 1.
        v.edit_col = v.edit_lines[0].chars().count();
        v.insert_char('!');
        // Split the line (Enter) then type "x".
        v.split_line();
        v.insert_char('x');
        assert_eq!(v.edit_lines, vec!["alpha!", "x", "beta", "gamma"]);

        assert!(v.undo(), "x");
        assert_eq!(v.edit_lines, vec!["alpha!", "", "beta", "gamma"]);
        assert!(v.undo(), "the split");
        assert_eq!(v.edit_lines, vec!["alpha!", "beta", "gamma"]);
        assert!(v.undo(), "!");
        assert_eq!(v.edit_lines, vec!["alpha", "beta", "gamma"]);

        // Redo walks the same rail in the other direction.
        assert!(v.redo());
        assert_eq!(v.edit_lines, vec!["alpha!", "beta", "gamma"]);
        assert!(v.redo());
        assert_eq!(v.edit_lines, vec!["alpha!", "", "beta", "gamma"]);
        assert!(v.redo());
        assert_eq!(v.edit_lines, vec!["alpha!", "x", "beta", "gamma"]);
        assert!(!v.redo(), "rail drained");
    }

    /// A fresh edit after an undo drops the redo rail — the divergence rule the
    /// composer's stack already had, and the reason generalizing it beat writing
    /// a second one.
    #[test]
    fn a_fresh_edit_after_undo_drops_the_redo_rail() {
        let mut v = edited();
        v.edit_col = 0;
        v.delete_forward();
        assert!(v.undo());
        assert_eq!(v.edit_lines[0], "alpha");
        assert!(v.redo(), "the rail still held the deletion");

        // Diverge: a different edit now.
        v.edit_col = 5;
        v.insert_char('?');
        assert!(
            !v.redo(),
            "a divergent edit clears the rail — redo cannot resurrect the deleted branch"
        );
    }

    /// The cursor is part of the snapshot, not an afterthought: undoing a
    /// movement that changed nothing must not move the cursor either, and
    /// undoing an edit must land where the edit began.
    #[test]
    fn undo_restores_the_cursor_alongside_the_text() {
        let mut v = edited();
        v.edit_row = 2;
        v.edit_col = 3;
        // One edit at row 2 / col 3.
        v.insert_char('Z');
        assert_eq!((v.edit_row, v.edit_col), (2, 4));
        // Move the cursor somewhere else, then undo: it returns to the edit.
        v.edit_row = 0;
        v.edit_col = 0;
        assert!(v.undo());
        assert_eq!(
            (v.edit_row, v.edit_col),
            (2, 3),
            "the snapshot holds the cursor, so undo is not just a text restore"
        );
        assert_eq!(v.edit_lines[2], "gamma");
    }

    /// Entering edit mode starts with an empty history: the file as opened is
    /// the baseline, so there is nothing to undo until the user changes
    /// something — and `Ctrl+Z` on a fresh buffer says so rather than dying.
    #[test]
    fn a_fresh_edit_session_has_nothing_to_undo() {
        let mut v = edited();
        assert!(!v.undo());
        assert!(!v.redo());
        assert_eq!(v.edit_lines, vec!["alpha", "beta", "gamma"]);
    }

    // ---- find and replace, which the file editor never had ---------------------
    //
    // Find existed only in view mode, and it was unreachable in edit mode for two
    // reasons that compounded: `find_recompute` read `lines` (the file as opened)
    // rather than the buffer, and the edit-mode key arm `return false`d before
    // the find arm could ever run. So a user editing a file who pressed `/` typed
    // `/needle` into their own text and nothing else happened.

    /// **Find searches the buffer being edited, and stepping moves the caret.**
    ///
    /// A highlighted line the caret cannot reach is a decoration. The step lands
    /// on the first occurrence of the query in the line it moves to.
    #[test]
    fn find_searches_the_edit_buffer_and_stepping_moves_the_caret() {
        let mut v = edited();
        v.edit_lines = vec![
            "need a needle".to_string(),
            "middle".to_string(),
            "and a needle again".to_string(),
        ];
        v.find_open();
        for c in "needle".chars() {
            v.find_query.push(c);
        }
        v.find_recompute();
        assert_eq!(
            v.find_hits,
            vec![0, 2],
            "both buffer lines, not the opened file's"
        );

        v.find_step(true, 10);
        assert_eq!(v.edit_row, 2, "the caret follows the hit");
        assert_eq!(v.edit_col, 6, "and lands on the match, not the line start");
        // Wrapping back to the first: "need a " is seven characters, so the
        // needle on that line starts one column later than the one above.
        v.find_step(true, 10);
        assert_eq!(v.edit_row, 0);
        assert_eq!(v.edit_col, 7);
    }
}
