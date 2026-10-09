//! The workspace file tree and a read-only editor.
//!
//! # Why a tree and not a list
//!
//! A flat list of every path in a repository is unusable past a few hundred files: the
//! shared prefix repeats on every row, and the thing a reader is looking for — "the files
//! in `crates/epsilon-core`" — has no shape. So the flat walk result is folded into a
//! tree, and the tree is **flattened to rows with a depth** so it can be drawn by a scroll
//! area rather than by a recursive widget walk.
//!
//! # Why virtualization is not optional
//!
//! `ScrollArea::show_rows` draws only the visible span. A recursive `Tree` over a few
//! thousand files builds a widget per node **every frame**, and that cost lands exactly
//! where it hurts: on the window the agent is streaming into.
//!
//! # What is deliberately missing
//!
//! **Editing.** This pane shows what a file contains and refuses to pretend it can change
//! it: the header says `read-only` while it is. A read-only view that lies about being
//! read-only is worse than no view. The agent edits files here, and the pane that shows
//! the result is the diff view, not this one.

use std::path::{Path, PathBuf};

use crate::theme::IdeTheme;

/// Rows a workspace listing will collect before it stops looking.
///
/// Higher than `@`'s 200 because a tree is browsed, not typed at: a truncated tree looks
/// like a repository that has no more files, which is a lie. The pane's footer says
/// `partial` when the budget runs out.
pub const TREE_ROW_BUDGET: usize = 5_000;

/// How many `.gitignore` files the tree will read.
const TREE_IGNORE_FILES: usize = 64;

/// One row in the flattened tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Workspace-relative path with forward slashes, exactly what the model sees.
    pub path: String,
    /// The name to draw, without the parents.
    pub name: String,
    /// Nesting level, 0 for a top-level entry.
    pub depth: usize,
    /// `true` for a directory.
    pub is_dir: bool,
    /// `true` while the directory's children are shown.
    pub expanded: bool,
}

/// What the reader clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeClick {
    /// Open this file.
    File(String),
    /// Open or close this directory.
    Directory(String),
}

/// A flattened, expandable workspace tree.
///
/// Built once per workspace and then filtered by a search string; rebuilding on every
/// keystroke is what makes a file tree feel slow, and the walk is the expensive half.
#[derive(Debug, Clone, Default)]
pub struct FileTree {
    /// Every row, in display order, including collapsed directories' children.
    rows: Vec<Row>,
    /// True when the walk hit its budget and the list is **partial**.
    truncated: bool,
}

impl FileTree {
    /// Walk `workspace` and fold the result into a tree.
    ///
    /// A workspace that cannot be read produces an **empty tree, not an error**: the
    /// window opens on a directory the user chose and must open.
    pub fn load(workspace: &Path) -> Self {
        let ignore = epsilon_core::ignore::IgnoreSet::load(workspace, TREE_IGNORE_FILES);
        let mut budget = TREE_ROW_BUDGET;
        let files = epsilon_core::walk::walk_files(workspace, &ignore, &mut budget);
        let mut rows = build_rows(&files);
        // Top level open on first sight. Anything deeper is a decision the reader makes,
        // and a tree that arrives entirely collapsed looks empty.
        set_all_expanded(&mut rows, true);
        Self {
            rows,
            truncated: budget == 0,
        }
    }

    /// An empty tree, for a window with no workspace.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Whether the walk stopped early, so the pane must say the list is partial.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// How many rows the tree holds, open or not.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The rows to draw, after collapsing and filtering.
    ///
    /// Both are applied here rather than in the draw path, so a test can ask the tree the
    /// same question the pane asks it.
    pub fn visible(&self, query: &str) -> Vec<&Row> {
        let needle = query.trim().to_lowercase();
        self.rows
            .iter()
            .filter(|r| needle.is_empty() || r.path.to_lowercase().contains(&needle))
            .filter(|r| is_shown(&self.rows, r))
            .collect()
    }

    /// Open or close one directory.
    ///
    /// Matching on the *path*, not on an index: the visible list is a filtered view, and
    /// an index into it means something different every time the filter changes.
    pub fn toggle(&mut self, path: &str) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.is_dir && r.path == path) {
            row.expanded = !row.expanded;
        }
    }

    /// Whether `path` names a directory row.
    ///
    /// For the keyboard path, which has to tell "open this file" from "expand this folder"
    /// **the same way a click does** — a second rule for it would be two editors.
    pub fn is_dir(&self, path: &str) -> bool {
        self.rows.iter().any(|r| r.is_dir && r.path == path)
    }
}

/// Open or close every directory in `rows`.
fn set_all_expanded(rows: &mut [Row], on: bool) {
    for row in rows.iter_mut() {
        if row.is_dir {
            row.expanded = on;
        }
    }
}

/// Whether a row's parent chain is open.
fn is_shown(rows: &[Row], row: &Row) -> bool {
    if row.depth == 0 {
        return true;
    }
    // Walk the parents: for `a/b/c.rs`, both `a` and `a/b` must be expanded.
    let mut prefix = String::new();
    let segments: Vec<&str> = row.path.split('/').collect();
    for segment in segments.iter().take(segments.len().saturating_sub(1)) {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        match rows.iter().find(|r| r.is_dir && r.path == prefix) {
            Some(dir) if dir.expanded => {}
            _ => return false,
        }
    }
    true
}

/// Fold flat `a/b/c.rs` paths into display rows.
///
/// Directories are synthesised from the paths of the files beneath them, because the walk
/// returns files only — a directory with no files is not content and does not belong in a
/// list of things the agent can act on.
fn build_rows(files: &[String]) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for file in files {
        let segments: Vec<&str> = file.split('/').collect();
        let mut prefix = String::new();
        for (i, segment) in segments.iter().enumerate() {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            let is_last = i + 1 == segments.len();
            if is_last {
                rows.push(Row {
                    path: prefix.clone(),
                    name: (*segment).to_string(),
                    depth: i,
                    is_dir: false,
                    expanded: false,
                });
            } else if !rows.iter().any(|r| r.is_dir && r.path == prefix) {
                rows.push(Row {
                    path: prefix.clone(),
                    name: (*segment).to_string(),
                    depth: i,
                    is_dir: true,
                    expanded: false,
                });
            }
        }
    }
    // The walk emits a parent before its contents, but siblings are not ordered against
    // each other; one stable sort makes the tree stable frame to frame.
    rows.sort_by(|a, b| a.path.cmp(&b.path).then(a.is_dir.cmp(&b.is_dir)));
    rows
}

/// A file opened in the editor pane.
///
/// Read-only, and says so. The whole file is held: truncating it would make the line
/// numbers lie, and a megabyte of text is nothing next to a code editor that will one day
/// hold more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFile {
    /// Workspace-relative path.
    pub path: String,
    /// Its lines, without terminators.
    pub lines: Vec<String>,
}

impl OpenFile {
    /// Read `path` under `workspace`.
    ///
    /// Returns the reason rather than panicking: a file deleted between the click and the
    /// read is an ordinary thing for a reader to do, and the pane should say so.
    pub fn read(workspace: &Path, path: &str) -> Result<Self, String> {
        let full = workspace.join(path);
        let raw = std::fs::read_to_string(&full).map_err(|e| format!("{path}: {e}"))?;
        let lines = raw.lines().map(str::to_string).collect();
        Ok(Self {
            path: path.to_string(),
            lines,
        })
    }

    /// How many lines it has.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether the file has no lines at all.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// The absolute path for a workspace-relative one.
pub fn absolute(workspace: &Path, path: &str) -> PathBuf {
    workspace.join(path)
}

/// The id a tree row's hotspot is recorded under.
///
/// The path, not an index: a tree is filtered and collapsed, so an index means
/// something different on every frame, and a caller that stored one would press the wrong
/// row after typing one character.
pub fn hotspot_id(path: &str) -> String {
    format!("tree:{path}")
}

/// Which row the tree is marking, and as what.
///
/// One argument rather than two because they are one answer: "this row, in this
/// state". Split, a caller has to keep the two in step, and a tree drawing two
/// marks with no rule about which wins is a rule nobody can state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TreeMark<'a> {
    /// The file open in the editor.
    pub selected: Option<&'a str>,
    /// The row the keyboard is on, when the tree has pane focus.
    pub cursor: Option<&'a str>,
}

/// Draw the tree into `rect`, reporting what was clicked.
///
/// Virtualized: `show_rows` draws only the visible span, which is the difference between a
/// tree that opens instantly and one that spends its first second building widgets for
/// files nobody asked for. Every row's rectangle is pushed into `hotspots` as it is drawn.
///
/// `mark.cursor` is painted the way the selected row is painted. A cursor that moved with
/// the arrow keys and left nothing behind would be a cursor nobody could see, and "which
/// row will Enter open" is a question a reader answers by looking. Its rectangle is
/// recorded as `tree-cursor:<path>` for the same reason the tab strip records its own —
/// an assertion that cannot find the thing has measured nothing.
///
/// `summary` rather than a bare `&mut Vec<(String, Rect)>` of hotspots, because a painter
/// text size has to be recorded from the same place the text is painted — see
/// [`crate::app::paint_text`] — and the frame already carries that record.
///
/// **One argument over the clippy budget.** Seven were here before text scaling; the
/// eighth is `scale`, and reading it off the `Ui` instead would give the row height, the
/// indent and the font three chances to answer differently.
#[allow(clippy::too_many_arguments)]
pub fn draw_tree(
    ui: &mut egui::Ui,
    theme: &IdeTheme,
    rect: egui::Rect,
    tree: &FileTree,
    query: &str,
    mark: TreeMark<'_>,
    summary: &mut crate::app::FrameSummary,
    scale: f32,
) -> Option<TreeClick> {
    let visible = tree.visible(query);
    let mut clicked: Option<TreeClick> = None;
    // **One scroll area, in a scope pinned to `rect`.** Both halves matter, and the first
    // version had neither right in the way that matters: a `show` nested inside a
    // `show_rows` built its `Ui` from the scroll area's own origin rather than the pane's,
    // and nothing pinned the scroll area to `rect` at all — so it started wherever the
    // root `Ui`'s cursor happened to be, which is `(0, 0)`. Every row was recorded at the
    // left edge of the *window* and a click on the recorded rectangle hit nothing. A tree
    // that draws correctly and cannot be clicked is a screenshot.
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
        |ui| {
            let out = egui::ScrollArea::vertical()
                .id_salt("epsilon-file-tree")
                .auto_shrink([false, true])
                .show_rows(ui, 18.0 * scale, visible.len(), |ui, range| {
                    let range = range.clone();
                    for row in visible[range.clone()].iter() {
                        let indent = (10.0 + row.depth as f32 * 12.0) * scale;
                        let marker = match (row.is_dir, row.expanded) {
                            (true, true) => "▾ ",
                            (true, false) => "▸ ",
                            _ => "  ",
                        };
                        let is_selected = mark.selected == Some(row.path.as_str());
                        let color = if row.is_dir {
                            theme.title
                        } else if is_selected {
                            theme.accent
                        } else {
                            theme.text
                        };
                        // Allocate rather than position: an allocated response carries
                        // both the screen rectangle and the click handling, and the two
                        // cannot drift apart.
                        let (row_rect, response) = ui.allocate_exact_size(
                            egui::vec2(rect.width(), 17.0 * scale),
                            egui::Sense::click(),
                        );
                        let painter = ui.painter();
                        if is_selected {
                            painter.rect_filled(row_rect, 0.0, theme.selection_focus);
                        }
                        // The keyboard's row. `selection_focus` is the role for exactly
                        // this state and the theme already holds it 3:1 from `selection`
                        // (`STATE_FLOOR`), so the marker is legible without a new colour.
                        if mark.cursor == Some(row.path.as_str()) {
                            painter.rect_filled(row_rect, 0.0, theme.selection_focus);
                            summary
                                .hotspots
                                .push((format!("tree-cursor:{}", row.path), row_rect));
                        }
                        crate::app::paint_text(
                            painter,
                            summary,
                            "tree",
                            row_rect.left_top() + egui::vec2(indent, 13.0 * scale),
                            egui::Align2::LEFT_CENTER,
                            format!("{marker}{}", row.name),
                            egui::FontId::proportional(13.0 * scale),
                            color,
                        );
                        // **Name the node, because `Sense::click()` already created one.**
                        //
                        // egui allocates an AccessKit node for every focusable widget and fills
                        // it without a role (`egui-0.36.2/src/context.rs:1292-1297`), so each
                        // drawn row was a focusable, clickable, **unnamed** node — a screen
                        // reader could focus a file and be told nothing about it. Caught by
                        // reading the dumped tree on 2026-10-04, which carried exactly one
                        // `Role::Unknown` node in a window with one file.
                        //
                        // The name says what the row **is** and what activating it does, because
                        // a bare file name is ambiguous in a tree: a reader has to know whether
                        // they are on a folder, and that Enter opens the file.
                        crate::app::name_node(
                            &response,
                            &format!(
                                // **The workspace-relative path, not the bare name (M312).**
                                // Two files called `README.md` — one at the top, one under
                                // `agents/` — were both named `File · README.md`, and the
                                // platform tree proves it: the UIA dump taken on 2026-10-04
                                // carries two identically-named `TreeItem`s
                                // (`audit/evidence/desktop-uia-tree-2026-10-04.txt`). A
                                // reader arriving by rotor or by search gets two identical
                                // entries and no way to tell them apart; the *drawn* row is
                                // indented by depth, so the eye resolves this and a reader
                                // cannot. The tree already keeps the path — `Row::path` is
                                // "exactly what the model sees", forward slashes and all —
                                // and a top-level file's path is its name, so the common case
                                // is unchanged.
                                "File · {}{}",
                                row.path,
                                if row.is_dir { " · folder" } else { "" }
                            ),
                        );
                        // **And its role**, which naming it does not supply. The node was
                        // `Role::Unknown` even once named: egui builds a node for a focusable
                        // widget and fills it with no role
                        // (`egui-0.36.2/src/context.rs:1292-1297`, `response.rs:908-924`), and
                        // a reader that trusts the role first has nothing to announce.
                        //
                        // `Role::TreeItem` is the role for a row inside a tree, and it is what
                        // lets a reader announce the row as an item rather than as an
                        // anonymous focusable box.
                        //
                        // **The pane is a `Region`, not a `Tree`, and that is not a nesting
                        // problem.** Two earlier versions of this comment said it was: one
                        // promised the pane "would become" a `Tree` and was never reconciled
                        // with the role M302 gave it; the next claimed "a `treeitem` under a
                        // `region` is not a valid ARIA structure". Both were wrong, and an
                        // independent read-only pass on 2026-10-04 is what settled it.
                        //
                        // **There is no `treeitem`-under-`region` here to invalidate.**
                        // egui attaches each new node to the nearest ancestor that already has
                        // one, and the pane `Ui`s built by `pane_ui` carry no node at all — so
                        // `find_accesskit_parent` (`egui-0.36.2/src/context.rs:613-621`) skips
                        // every one of them and lands on the root. **The `Region` node and every
                        // `TreeItem` node are siblings directly under the AccessKit root**, which
                        // is what ADR-022 already records ("the nodes attach flat to the window
                        // root … Truthful, but not a pane hierarchy").
                        //
                        // So flipping this pane's role to `Tree` would change one label and
                        // create no structure — the change "that would have looked like it
                        // worked on the frame it was written". Making the hierarchy real needs
                        // a node on the pane `Ui` first, which moves *every* region's
                        // parentage and is behaviour-level. Not done, and not a role flip.
                        crate::app::name_role(&response, egui::accesskit::Role::TreeItem);
                        if response.clicked() {
                            let path = row.path.clone();
                            clicked = Some(if row.is_dir {
                                TreeClick::Directory(path)
                            } else {
                                TreeClick::File(path)
                            });
                        }
                        summary.hotspots.push((hotspot_id(&row.path), row_rect));
                    }
                });
            // **The scrollbar, named when there is one (M312).** This is the pane the probe
            // found unnamed: a 1000-file workspace scrolls, egui draws the bar and labels it
            // nothing, and a reader met a `ScrollBar` with no name. The predicate is egui's own
            // threshold reproduced from what the output publishes; see `name_scrollbar`.
            crate::app::name_scrollbar(ui.ctx(), &out, "the file tree");
        },
    );
    clicked
}

/// Draw what the agent changed to `change`, in place of the file.
///
/// **This is the review, and the pane had none.** The transcript could report a change in
/// prose — a summary line and a capped number of diff rows — but a report is not a review:
/// the reader cannot see the whole change, cannot tell a removal from an addition at a
/// glance, and has to scroll back through the conversation to find it.
///
/// Three things it does that a coloured text row does not:
///
/// * **Added and removed are different colours, not just different characters.** A `+` and
///   a `-` are the whole signal, and they are two greys rendered next to each other.
/// * **A coarse change says so.** A block replacement past eight changed lines is not a
///   line-accurate diff, and a diff that quietly presents itself as one is worse than the
///   prose it replaced.
/// * **The clipping is the pane's**, so a long line cannot paint across the transcript.
pub fn draw_diff(
    ui: &mut egui::Ui,
    theme: &IdeTheme,
    rect: egui::Rect,
    change: &crate::diff::Change,
    mut summary: Option<&mut crate::app::FrameSummary>,
    scale: f32,
    expanded: &mut bool,
) {
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 4.0, theme.panel);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme.border),
        egui::StrokeKind::Inside,
    );
    let heading = format!(
        "{}  ·  +{} −{}  ·  {}  ·  open the file to read it",
        change.path,
        change.added(),
        change.removed(),
        if change.created {
            "new file"
        } else if change.is_coarse() {
            "middle replaced as a block"
        } else {
            "changed"
        }
    );
    // The heading's own line and the body's top offset are text geometry: at 200% the
    // header is a taller header, and the body starts below it rather than through it.
    let mut sink = crate::app::FrameSummary::default();
    crate::app::paint_text(
        &painter,
        if summary.is_some() {
            summary.as_deref_mut().expect("checked")
        } else {
            &mut sink
        },
        "editor",
        rect.left_top() + egui::vec2(12.0, 16.0 * scale),
        egui::Align2::LEFT_CENTER,
        heading.clone(),
        egui::FontId::proportional(12.0 * scale),
        theme.dim,
    );
    if let Some(s) = summary.as_deref_mut() {
        s.rows.push(("editor".into(), heading));
    }
    let body = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + 30.0 * scale),
        egui::pos2(rect.right(), rect.bottom()),
    );
    if body.height() <= 8.0 {
        return;
    }
    // A "+" and a "-" as two greys is the failure this whole pane exists to fix, so the
    // colours are the palette's success and danger roles rather than two more greys: they
    // are already separated by the contrast floor, and `theme.rs` already asserts they are
    // not the same colour.
    let (add, remove, context) = (theme.success, theme.danger, theme.dim);
    // The row height is the line height of a 12-point monospace face, so it is the text's
    // geometry and moves with it: `show_rows` wants one height for every row, and that
    // height has to be the height the text actually occupies.
    let line_h = 16.0 * scale;
    let gutter = 52.0 * scale;
    // C-135: nine rows, then a fold. Longer diffs open folded with a real
    // button (keyboard-focusable, named for the reader); the heading already
    // carries +added/−removed, so nothing is lost by folding.
    const FOLD_AT: usize = 9;
    let folded = !*expanded && change.lines.len() > FOLD_AT;
    let rows = if folded {
        FOLD_AT + 1
    } else {
        change.lines.len()
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        let out = egui::ScrollArea::vertical()
            .id_salt("epsilon-diff-body")
            .auto_shrink([false, false])
            .show_rows(ui, line_h, rows, |ui, range| {
                for i in range {
                    if folded && i == FOLD_AT {
                        let hidden = change.lines.len() - FOLD_AT;
                        let label = format!("Show {hidden} more lines");
                        let response = ui.button(label.clone());
                        if response.clicked() {
                            *expanded = true;
                        }
                        if let Some(s) = summary.as_deref_mut() {
                            s.rows.push(("editor".into(), label));
                            s.row_rects.push(("editor".into(), response.rect));
                        }
                        continue;
                    }
                    let line = &change.lines[i];
                    // **Allocate the row, or every context line lands on the same y (M312).**
                    // The same defect the editor had, found by rendering this pane and looking at
                    // it: a four-line diff drew its context lines as one band of overlapping text.
                    // `show_rows` decides which rows to call for; the closure has to give each
                    // one space. `allocate_exact_size` advances by exactly the height asked for,
                    // with no `item_spacing` after it (`egui-0.36.2/src/ui.rs:1146-1157`), so the
                    // stride stays `line_h`.
                    let (row, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), line_h),
                        egui::Sense::hover(),
                    );
                    let (marker, text, color) = match line {
                        crate::diff::DiffLine::Added(t) => ('+', t.as_str(), add),
                        crate::diff::DiffLine::Removed(t) => ('-', t.as_str(), remove),
                        crate::diff::DiffLine::Context(t) => (' ', t.as_str(), context),
                    };
                    let painted = format!("{marker}{text}");
                    crate::app::paint_text(
                        ui.painter(),
                        if summary.is_some() {
                            summary.as_deref_mut().expect("checked")
                        } else {
                            &mut sink
                        },
                        "editor",
                        egui::pos2(row.left() + gutter, row.center().y),
                        egui::Align2::LEFT_CENTER,
                        &painted,
                        egui::FontId::monospace(12.0 * scale),
                        color,
                    );
                    if let Some(s) = summary.as_deref_mut() {
                        s.rows.push(("editor".into(), painted));
                        s.row_rects.push(("editor".into(), row));
                    }
                }
            });
        // **The scrollbar, named when there is one (M312).** A diff longer than the pane is
        // the ordinary case here, which makes this the pane where the unnamed node was easiest
        // for a reader to meet.
        crate::app::name_scrollbar(ui.ctx(), &out, "the change under review");
    });
}

/// Draw the open file, read-only, with line numbers.
pub fn draw_editor(
    ui: &mut egui::Ui,
    theme: &IdeTheme,
    rect: egui::Rect,
    file: &OpenFile,
    mut summary: Option<&mut crate::app::FrameSummary>,
    scale: f32,
) {
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 4.0, theme.panel);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme.border),
        egui::StrokeKind::Inside,
    );
    let mut sink = crate::app::FrameSummary::default();
    crate::app::paint_text(
        &painter,
        if summary.is_some() {
            summary.as_deref_mut().expect("checked")
        } else {
            &mut sink
        },
        "editor",
        rect.left_top() + egui::vec2(12.0, 16.0 * scale),
        egui::Align2::LEFT_CENTER,
        format!("{}  ·  {} lines  ·  read-only", file.path, file.len()),
        egui::FontId::proportional(12.0 * scale),
        theme.dim,
    );
    let body = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + 30.0 * scale),
        egui::pos2(rect.right(), rect.bottom()),
    );
    if body.height() <= 8.0 * scale || file.lines.is_empty() {
        return;
    }
    let gutter = 52.0 * scale;
    let line_h = 16.0 * scale;
    // **From the top, and scrollable.** This showed `lines[len - visible..]` — the tail of
    // the file, with no scrollbar and no way to reach line 1, so opening a 900-line source
    // file showed its last forty lines and called it an editor. Every file begins at line
    // 1 now, and the reader scrolls down like every other editor they have used.
    //
    // The scope is pinned to `body` because the pane's own `Ui` starts at the top of the
    // pane, which is where the header was painted; without it the scroll area would open
    // underneath the header line.
    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        let out = egui::ScrollArea::vertical()
            .id_salt("epsilon-file-body")
            .auto_shrink([false, false])
            .show_rows(ui, line_h, file.len(), |ui, range| {
                for i in range {
                    let line = &file.lines[i];
                    // **Allocate the row, or every line lands on the same y (M312).**
                    //
                    // `show_rows` is virtualised: it decides *which* lines to call this closure
                    // for, and the closure is expected to give each one space. egui's own example
                    // does it with `ui.label(text)`. This loop painted with the painter and
                    // allocated **nothing**, so the cursor never advanced and every visible line
                    // was drawn at the same `row.center().y` — **a file opened as one squashed
                    // band of overlapping glyphs**, which is what the rendered frame showed for a
                    // five-line file.
                    //
                    // `allocate_exact_size` is the right call because it advances by exactly the
                    // size given (`egui-0.36.2/src/ui.rs:1146-1157` → `allocate_response`), with
                    // **no** `item_spacing` after it — so the stride stays `line_h` and the
                    // virtualisation's arithmetic keeps matching what is drawn.
                    let (row, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), line_h),
                        egui::Sense::hover(),
                    );
                    let number = format!("{:>5}", i + 1);
                    crate::app::paint_text(
                        ui.painter(),
                        if summary.is_some() {
                            summary.as_deref_mut().expect("checked")
                        } else {
                            &mut sink
                        },
                        "editor",
                        egui::pos2(row.left() + 8.0 * scale, row.center().y),
                        egui::Align2::LEFT_CENTER,
                        &number,
                        egui::FontId::monospace(12.0 * scale),
                        theme.gutter(),
                    );
                    crate::app::paint_text(
                        ui.painter(),
                        if summary.is_some() {
                            summary.as_deref_mut().expect("checked")
                        } else {
                            &mut sink
                        },
                        "editor",
                        egui::pos2(row.left() + gutter, row.center().y),
                        egui::Align2::LEFT_CENTER,
                        line,
                        egui::FontId::monospace(12.0 * scale),
                        theme.text,
                    );
                    if let Some(s) = summary.as_deref_mut() {
                        s.rows.push(("editor".into(), format!("{number} {line}")));
                        s.row_rects.push(("editor".into(), row));
                    }
                }
            });
        // **The scrollbar, named when there is one (M312).** Any file longer than the pane needs
        // it, so this is the most reachable unnamed node in the window.
        crate::app::name_scrollbar(ui.ctx(), &out, "the open file");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "one\ntwo\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src").join("deep")).unwrap();
        std::fs::write(dir.path().join("src/deep/inner.rs"), "x\n").unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target").join("junk.bin"), "").unwrap();
        dir
    }

    #[test]
    fn the_tree_shows_the_workspaces_own_files_and_not_its_build_output() {
        let dir = workspace();
        let tree = FileTree::load(dir.path());
        let paths: Vec<&str> = tree.visible("").iter().map(|r| r.path.as_str()).collect();
        assert!(paths.contains(&"README.md"), "{paths:?}");
        assert!(paths.contains(&"src/main.rs"), "{paths:?}");
        assert!(paths.contains(&"src/deep/inner.rs"), "{paths:?}");
        assert!(
            !paths.iter().any(|p| p.contains("target")),
            "build output is not workspace content: {paths:?}"
        );
    }

    #[test]
    fn a_directory_is_in_the_tree_only_when_it_has_files() {
        // An empty directory is not something the agent can act on, and listing it would
        // be the tree claiming a workspace has content it does not have.
        let dir = workspace();
        std::fs::create_dir_all(dir.path().join("empty")).unwrap();
        let tree = FileTree::load(dir.path());
        let paths: Vec<&str> = tree.visible("").iter().map(|r| r.path.as_str()).collect();
        assert!(!paths.contains(&"empty"), "{paths:?}");
    }

    #[test]
    fn nesting_is_recorded_so_the_tree_indents_rather_than_guesses() {
        let dir = workspace();
        let tree = FileTree::load(dir.path());
        let row = tree
            .visible("")
            .into_iter()
            .find(|r| r.path == "src/deep/inner.rs")
            .expect("the nested file");
        assert_eq!(row.depth, 2, "{row:?}");
    }

    /// **`Row::path` is the identifier, `Row::name` is only the label (M312).**
    ///
    /// The accessible name a file row publishes is its **path**, because two files can share a
    /// name and a reader cannot tell them apart — which the platform tree proves happened
    /// (`audit/evidence/desktop-uia-tree-2026-10-04.txt` carries two `File · README.md`). This
    /// test pins the property that change depends on: the path is the whole workspace-relative
    /// path with forward slashes, and the name is its last segment alone.
    ///
    /// **If this fails, the naming fix is wrong rather than incomplete** — a path that lost its
    /// parents, or a name that grew them, puts `agents/README.md` on screen in a tree whose
    /// indentation already says where it is.
    #[test]
    fn a_row_carries_its_whole_path_and_keeps_the_bare_name_for_drawing() {
        let dir = workspace();
        let tree = FileTree::load(dir.path());
        let rows = tree.visible("");
        let nested = rows
            .iter()
            .find(|r| r.path == "src/deep/inner.rs")
            .expect("the nested file");
        assert_eq!(
            nested.name, "inner.rs",
            "the drawn row shows the last segment only"
        );
        assert!(
            !nested.name.contains('/'),
            "a name that carried its parents would print `deep/inner.rs` under an indented row: {:?}",
            nested.name
        );
        let top = rows
            .iter()
            .find(|r| r.path == "README.md")
            .expect("the fixture's top-level file");
        assert_eq!(
            top.path, top.name,
            "a top-level row's path and name agree, which is why the common case is unchanged"
        );
    }

    #[test]
    fn collapsing_a_directory_hides_its_children_and_toggling_brings_them_back() {
        let dir = workspace();
        let mut tree = FileTree::load(dir.path());
        assert!(
            tree.visible("")
                .iter()
                .any(|r| r.path == "src/deep/inner.rs"),
            "the fixture is nested"
        );
        tree.toggle("src");
        let collapsed: Vec<&str> = tree.visible("").iter().map(|r| r.path.as_str()).collect();
        assert!(!collapsed.contains(&"src/main.rs"), "{collapsed:?}");
        assert!(
            !collapsed.contains(&"src/deep/inner.rs"),
            "a child of a collapsed child is hidden too: {collapsed:?}"
        );
        tree.toggle("src");
        assert!(tree.visible("").iter().any(|r| r.path == "src/main.rs"));
    }

    #[test]
    fn a_filter_narrows_the_tree_by_path() {
        let dir = workspace();
        let tree = FileTree::load(dir.path());
        let hits: Vec<&str> = tree
            .visible("main")
            .iter()
            .map(|r| r.path.as_str())
            .collect();
        assert_eq!(hits, vec!["src/main.rs"], "{hits:?}");
    }

    #[test]
    fn a_missing_workspace_is_an_empty_tree_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("not-here");
        assert!(FileTree::load(&gone).visible("").is_empty());
    }

    #[test]
    fn opening_a_file_reads_its_lines_and_says_so_when_it_cannot() {
        let dir = workspace();
        let file = OpenFile::read(dir.path(), "src/main.rs").expect("the file is there");
        assert_eq!(file.lines, vec!["fn main() {}"]);
        assert_eq!(file.len(), 1);

        let err = OpenFile::read(dir.path(), "gone.rs").expect_err("must say why");
        assert!(err.contains("gone.rs"), "the refusal names the file: {err}");
    }

    #[test]
    fn a_trailing_newline_does_not_invent_a_line() {
        // `str::lines()` on "a\nb\n" gives ["a", "b"]; the bug this guards is the
        // opposite one, a phantom empty last line the editor's count would report.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a\nb\n").unwrap();
        let file = OpenFile::read(dir.path(), "a.txt").unwrap();
        assert_eq!(file.lines, vec!["a", "b"], "{:?}", file.lines);
    }

    #[test]
    fn a_long_line_is_one_line_and_does_not_become_a_page_of_widgets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("big.txt"), "x".repeat(80)).unwrap();
        let file = OpenFile::read(dir.path(), "big.txt").unwrap();
        assert_eq!(file.len(), 1, "one long line is one line");
    }
}
