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
        lines.push(Line::from(format!("  … capped at {TREE_ROW_BUDGET} rows")));
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
