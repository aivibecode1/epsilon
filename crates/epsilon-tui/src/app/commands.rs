//! Slash-command execution, split by what the command is *about*.
//!
//! ## Why this file exists
//!
//! `run_command` had grown to **1,147 lines**: a single `match` with 34 arms, no one of them
//! larger than 97 lines. Nothing about it was hard to *do* — that is exactly the problem. A
//! function that long cannot be reviewed, cannot be tested one concern at a time, and gives a
//! new command nowhere obvious to go.
//!
//! So it is now a dispatcher over three themed executors. The 34 arms were moved by line
//! range, not retyped, and each executor's arm bodies are byte-identical to what `app.rs`
//! held (verified by the checker in `docs/milestones/M162-slash-commands-by-subject.md`).
//! Because the arms sat at the same indent depth inside the same `match`-inside-`fn` shape,
//! the move required no re-indentation at all — which is also why it is hard to get wrong.
//!
//! ## Why the catch-all is deliberate
//!
//! `run_command` routes two groups by name and sends **everything else** to
//! `run_workspace_command`. A new `Command` variant therefore cannot be silently unrouted:
//! it lands in the third executor, where the compiler will insist its arm exist, and the
//! `Command::Unknown` fallback stays honest. An exhaustive-by-name router would instead make
//! a forgotten command a runtime no-op, which is the kind of bug that ships.
//!
//! ## The grouping
//!
//! - **provider** — who answers, and how hard they think: `/models`, `/model`, `/think`,
//!   `/scoped-models`, `/login`, `/logout`.
//! - **session** — what the session *is*: fork, new, resume, browse, inspect the tree,
//!   clone, reload, rename.
//! - **workspace** — everything the frame and the workspace do, from `/theme` to `/export`.
//!
//! It is a judgement about cohesion, not a claim that these three are the only right groups.
//! What matters is that the boundaries are legible to the next reader.

use super::*;

/// The one-line preview the timeline picker shows for a session entry.
///
/// Long text is cut at 100 characters and marked with an ellipsis, because a picker's row
/// width is the terminal's, not the entry's: a 40,000-character assistant message would
/// push every other row off screen. Whitespace-only text reads `(empty)` rather than
/// nothing, so "this entry has no text" stays distinguishable from "this row is cut off".
pub(super) fn timeline_snip(e: &epsilon_contracts::SessionEntry) -> String {
    /// The preview budget, in **screen columns**.
    ///
    /// It was 100 `char`s, which is two budgets for CJK and a half for emoji — the preview
    /// overflowed the very column it existed to fit.
    const PREVIEW_COLUMNS: usize = 100;
    match &e.kind {
        epsilon_contracts::session_entry::EntryKind::Message { message, .. } => {
            let txt: String = match message {
                epsilon_contracts::message::Message::User(u) => u
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        epsilon_contracts::content::UserContent::Text { text } => {
                            Some(text.as_str())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                epsilon_contracts::message::Message::Assistant(a) => {
                    // The calls ride the row, not only the prose: a turn that only
                    // called tools previewed as `(empty)` — the least informative row
                    // in a picker whose job is finding "that bash call". Each call is
                    // named through the shared `call_head` (C-120), so the tree, the
                    // live card and the window title all describe one call one way.
                    let calls: Vec<String> = a
                        .tool_calls()
                        .map(|(_, name, args)| {
                            let head = crate::renderers::call_head(name, args, "…");
                            if head.is_empty() {
                                format!("[{name}]")
                            } else {
                                format!("[{name}]: {head}")
                            }
                        })
                        .collect();
                    let txt = a.text();
                    match (txt.trim().is_empty(), calls.is_empty()) {
                        (true, true) => String::new(),
                        (true, false) => calls.join(" · "),
                        (false, true) => txt,
                        (false, false) => format!("{} · {txt}", calls.join(" · ")),
                    }
                }
                epsilon_contracts::message::Message::ToolResult(t) => t
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        epsilon_contracts::content::ContentBlock::Text { text } => {
                            Some(text.as_str())
                        }
                        _ => None,
                    })
                    .collect::<String>(),
                _ => String::new(),
            };
            let s = crate::wrap::fit_width(&txt, PREVIEW_COLUMNS);
            if s.trim().is_empty() {
                "(empty)".into()
            } else {
                s
            }
        }
        epsilon_contracts::session_entry::EntryKind::Compaction { summary, .. } => {
            let s = crate::wrap::fit_width(summary, PREVIEW_COLUMNS);
            format!("[compaction] {s}")
        }
        epsilon_contracts::session_entry::EntryKind::TurnEnd { reason, .. } => {
            format!("[turn_end {reason:?}]")
        }
        _ => "[other]".into(),
    }
}

/// The narrow column naming what kind of entry a row is: `user`, `assistant`, `tool`,
/// `compact`, `turn`, or `other` for any entry kind added after this was written.
pub(super) fn timeline_kind_tag(e: &epsilon_contracts::SessionEntry) -> &'static str {
    match &e.kind {
        epsilon_contracts::session_entry::EntryKind::Message { message, .. } => match message {
            epsilon_contracts::message::Message::User(_) => "user",
            epsilon_contracts::message::Message::Assistant(_) => "assistant",
            epsilon_contracts::message::Message::ToolResult(_) => "tool",
            _ => "msg",
        },
        epsilon_contracts::session_entry::EntryKind::Compaction { .. } => "compact",
        epsilon_contracts::session_entry::EntryKind::TurnEnd { .. } => "turn",
        _ => "other",
    }
}

/// Flatten a session forest into picker rows, depth-first.
///
/// The order is the forest's own order, which `build_forest_from_entries` already
/// sorted by timestamp at every level — so a parent always precedes its children
/// and a branch reads in the order it happened. Re-sorting here would be a
/// second answer to a question the store has already answered.
pub(crate) fn flatten_tree(
    nodes: &[epsilon_contracts::session_entry::SessionTreeNode],
    depth: usize,
    active: &std::collections::HashSet<String>,
    labels: &std::collections::HashMap<String, String>,
    leaf_id: Option<&str>,
    out: &mut Vec<crate::app::TimelineRow>,
) {
    // P-303: the subtree holding the leaf sorts first at every level, so the
    // live context meets the eye before the abandoned lanes — pi's rule
    // (`tree-selector.ts:245,274,217-240`), applied at display time so the
    // stored order (timestamp ascending) never moves. Stable within each half.
    let mut ordered: Vec<&epsilon_contracts::session_entry::SessionTreeNode> =
        nodes.iter().collect();
    if leaf_id.is_some() {
        ordered.sort_by_key(|n| !subtree_contains(n, leaf_id));
    }
    for n in ordered {
        // P-305: pi's drop rule (`tree-selector.ts:346-356`) — a tool-only turn
        // is navigation noise. The recursion continues past a dropped row so
        // its children stay listed, exactly as pi's flat filter does; their
        // depth is recomputed downstream (`visible_depth` promotes orphans).
        if dropped_from_tree(&n.entry, leaf_id) {
            flatten_tree(&n.children, depth + 1, active, labels, leaf_id, out);
            continue;
        }
        out.push(crate::app::TimelineRow {
            seq: n.entry.seq,
            id: n.entry.id.clone(),
            kind: timeline_kind_tag(&n.entry).to_string(),
            snippet: timeline_snip(&n.entry),
            depth,
            on_active: active.contains(&n.entry.id),
            parent: n.entry.parent_id.clone(),
            label: labels.get(&n.entry.id).cloned(),
        });
        flatten_tree(&n.children, depth + 1, active, labels, leaf_id, out);
    }
}

/// Whether the subtree rooted at `node` holds `leaf_id`: the node itself or
/// any descendant. `None` never matches, so an unknown leaf keeps stored order.
fn subtree_contains(
    node: &epsilon_contracts::session_entry::SessionTreeNode,
    leaf_id: Option<&str>,
) -> bool {
    let Some(leaf) = leaf_id else {
        return false;
    };
    node.entry.id == leaf || node.children.iter().any(|c| subtree_contains(c, leaf_id))
}

/// Pi's tree drop rule (`tree-selector.ts:346-356`): an assistant message with
/// no text content and a normal stop reason is dropped — unless it is the leaf
/// the reader is on, or it carries an abnormal stop (truncated, failed,
/// aborted), which is exactly what a reader hunting "where did it break"
/// needs to find. Tool calls do not count as text; pi checks content blocks,
/// and the prose half of the snip is this crate's same question.
///
/// An unknown leaf drops nothing: hiding turns when the reader's position is
/// unknown could bury the only context they have.
fn dropped_from_tree(e: &epsilon_contracts::SessionEntry, leaf_id: Option<&str>) -> bool {
    let Some(leaf) = leaf_id else {
        return false;
    };
    if e.id == leaf {
        return false;
    }
    let epsilon_contracts::session_entry::EntryKind::Message { message, .. } = &e.kind else {
        return false;
    };
    let epsilon_contracts::message::Message::Assistant(a) = message else {
        return false;
    };
    if !a.text().trim().is_empty() {
        return false;
    }
    matches!(
        a.stop_reason,
        epsilon_contracts::message::StopReason::EndTurn
            | epsilon_contracts::message::StopReason::ToolUse
    )
}

/// Apply a filter to the flattened rows, **promoting orphans to depth 0**.
///
/// **The subtlety, and the reason this is not a `filter().collect()`.** pi
/// filters a flat list and then calls `recalculateVisualStructure()` on what
/// survived (`tree-selector.ts:408`). So when a filter hides a node but keeps
/// its child, the child's parent is gone: the surviving row is re-parented onto
/// its nearest visible ancestor and its depth recomputed. A plain filter would
/// either drop the child (losing an entry the user asked to see) or leave it
/// indented under a row that is not on screen — a dangling indent that reads as
/// a rendering bug.
///
/// The selection is returned too, so the caller can keep the highlighted entry
/// selected when it survives and fall back sensibly when it does not.
pub(crate) fn apply_filter(
    rows: &[crate::app::TimelineRow],
    f: crate::app::TreeFilter,
    labels: &std::collections::HashMap<String, String>,
) -> Vec<crate::app::TimelineRow> {
    let visible: std::collections::HashSet<&str> = rows
        .iter()
        .filter(|r| match f {
            crate::app::TreeFilter::LabeledOnly => labels.contains_key(&r.id),
            _ => passes_filter_entry(r, f),
        })
        .map(|r| r.id.as_str())
        .collect();
    if visible.is_empty() {
        return Vec::new();
    }
    let by_id: std::collections::HashMap<&str, &crate::app::TimelineRow> =
        rows.iter().map(|r| (r.id.as_str(), r)).collect();

    rows.iter()
        .filter(|r| visible.contains(r.id.as_str()))
        .map(|r| {
            let mut row = r.clone();
            row.depth = visible_depth(r, &visible, &by_id);
            row
        })
        .collect()
}

/// A surviving row's depth in the **visible** tree.
///
/// Walks up the parent chain and counts only the ancestors that are still on
/// screen. A row whose parent was filtered out therefore becomes shallower --
/// which is what "recalculate the indent based on the visible tree" means in
/// pi (`tree-selector.ts:408`), and what stops a row from being indented under
/// something the reader cannot see.
///
/// The walk is bounded by the row count, which is enough even if a hand-edited
/// file has a parent cycle: a cycle would otherwise loop forever, and the bound
/// makes the answer "as deep as the chain is" rather than a hang.
fn visible_depth(
    row: &crate::app::TimelineRow,
    visible: &std::collections::HashSet<&str>,
    by_id: &std::collections::HashMap<&str, &crate::app::TimelineRow>,
) -> usize {
    let mut depth = 0usize;
    let mut cursor = row.parent.as_deref();
    for _ in 0..by_id.len() {
        let Some(id) = cursor else { break };
        if !visible.contains(id) {
            // The parent is hidden: stop counting, this row is promoted. The
            // chain is NOT followed past a hidden node, because everything under
            // a hidden node is itself not on screen -- except this row, which is
            // here precisely because it passed on its own merits.
            break;
        }
        depth += 1;
        cursor = by_id.get(id).and_then(|r| r.parent.as_deref());
    }
    depth
}

/// Drop every row whose ancestor is folded shut.
///
/// pi does this as a second pass after filtering (`tree-selector.ts:400-407`):
/// it walks the flat list and marks any row whose `parentId` is folded **or is
/// itself marked**, which is what makes a fold hide a whole subtree rather than
/// one level. Reproducing that two-pass shape matters — doing it only one level
/// deep is the bug this function exists to avoid.
pub(crate) fn apply_folds(
    rows: &[crate::app::TimelineRow],
    folded: &std::collections::HashSet<String>,
) -> Vec<crate::app::TimelineRow> {
    if folded.is_empty() {
        return rows.to_vec();
    }
    let mut hidden: std::collections::HashSet<String> = Default::default();
    for r in rows {
        // A row is hidden when its parent is folded, or when its parent is
        // already hidden -- the second clause is what makes this transitive.
        if let Some(p) = r.parent.as_deref()
            && (folded.contains(p) || hidden.contains(p))
        {
            hidden.insert(r.id.clone());
        }
    }
    rows.iter()
        .filter(|r| !hidden.contains(&r.id))
        .cloned()
        .collect()
}

/// Does this row have children in the UNFILTERED tree?
///
/// pi's `isFoldable` (`tree-selector.ts:1004`) refuses to fold a leaf, because
/// "fold" on a leaf is a key press that appears to do nothing — the silent no-op
/// this repo treats as worse than a missing feature.
pub(crate) fn has_children(row: &crate::app::TimelineRow, all: &[crate::app::TimelineRow]) -> bool {
    all.iter()
        .any(|r| r.parent.as_deref() == Some(row.id.as_str()))
}

/// Does a row's underlying entry pass a filter (everything except LabeledOnly)?
fn passes_filter_entry(r: &crate::app::TimelineRow, f: crate::app::TreeFilter) -> bool {
    // The row already resolved kind to a tag; the tag is the same question the
    // entry answered, and keeping one representation is what stops the two from
    // disagreeing about what "user" means.
    match f {
        crate::app::TreeFilter::UserOnly => r.kind == "user",
        crate::app::TreeFilter::NoTools => !matches!(
            r.kind.as_str(),
            "label" | "model" | "thinking" | "info" | "tool"
        ),
        crate::app::TreeFilter::All => true,
        crate::app::TreeFilter::Default => {
            !matches!(r.kind.as_str(), "label" | "model" | "thinking" | "info")
        }
        crate::app::TreeFilter::LabeledOnly => false,
    }
}

/// Read the session's resolved labels, taking the store lock once.
fn store_labels(agent: &Arc<Agent>) -> Result<std::collections::HashMap<String, String>, String> {
    let Some(guard) = agent.session_store() else {
        return Err("no session store attached".to_string());
    };
    let st = guard.lock().map_err(|_| "store unavailable".to_string())?;
    Ok(resolved_labels(&st.read_all().map_err(|e| e.to_string())?))
}

/// Resolve every entry's label, in file order, last write wins.
///
/// Same rule as the wire projection in `rpc_mode.rs` and pi's loader: scan
/// entries, apply each `Label`, and an empty label deletes. Three readers of the
/// same rule is one too many, so the derivation is one function each side
/// calls rather than a third answer to compute.
fn resolved_labels(
    entries: &[epsilon_contracts::session_entry::SessionEntry],
) -> std::collections::HashMap<String, String> {
    let mut out: std::collections::HashMap<String, String> = Default::default();
    for e in entries {
        if let epsilon_contracts::session_entry::EntryKind::Label { target_id, label } = &e.kind {
            match label {
                Some(text) => {
                    out.insert(target_id.clone(), text.clone());
                }
                None => {
                    out.remove(target_id);
                }
            }
        }
    }
    out
}

/// One display line per skill: name, description, and whether the model may
/// invoke it on its own.
///
/// **A function, not inline formatting,** for the same reason `/templates` has
/// one: the property worth testing is that a description REACHES THE SCREEN, and
/// that is awkward to observe through a notice that must be rendered and scraped.
/// pi's resource panel states enablement per row (`config-selector.ts`), so a
/// name-only list hides half of what a reader needs.
pub(crate) fn skill_lines(
    skills: &[epsilon_core::skills::Skill],
    disabled: &[String],
) -> Vec<String> {
    skills
        .iter()
        .map(|s| {
            let mut line = format!("{} — {}", s.name, s.description);
            if s.disable_model_invocation {
                line.push_str(" (manual only)");
            }
            // **Off is stated, not implied by absence.** A skill that has been
            // switched off still gets discovered, so a listing that simply omitted
            // it would look identical to "it was never installed" -- and the
            // user would go looking for a directory that is right there.
            if disabled.iter().any(|d| d == &s.name) {
                line.push_str(" [off]");
            }
            line
        })
        .collect()
}

/// Rows that no filter and no search may hide.
///
/// Two rules, both from pi's own filter (`tree-selector.ts:339-355`):
///
/// 1. **The current leaf is always shown.** A view that can hide where you are
///    cannot answer "where am I?" -- the first question a session picker is asked.
/// 2. **An assistant turn with no text is hidden**, because a run of tool-call-only
///    turns makes the list unreadable. The exception that matters is an ERROR: an
///    error is text the reader is looking for, so those stay.
///
/// Applied here once, rather than in each filter: three filters each re-deciding
/// "is this row mandatory" is three chances to disagree.
fn apply_mandatory(
    rows: &[crate::app::TimelineRow],
    mut out: Vec<crate::app::TimelineRow>,
) -> Vec<crate::app::TimelineRow> {
    for r in rows {
        if out.iter().any(|o| o.id == r.id) {
            continue;
        }
        let is_leaf = r.on_active;
        let is_textless_assistant = r.kind == "assistant" && r.snippet.trim().is_empty();
        if is_leaf || is_textless_assistant {
            out.push(r.clone());
        }
    }
    // The rescued row keeps the TREE's position, not the end of the list. A
    // rescue that appends is a rescue that reorders, which is its own confusion.
    let order: std::collections::HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.as_str(), i))
        .collect();
    out.sort_by_key(|r| order.get(r.id.as_str()).copied().unwrap_or(usize::MAX));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// The text a row is searched against: its label, its kind, and its snippet.
///
/// pi builds the same three in `getSearchableText` (`tree-selector.ts:560-600`).
/// The message *role* pi also pushes is the `kind` tag here, so searching "user"
/// finds user messages in both.
fn searchable_text(r: &crate::app::TimelineRow) -> String {
    let mut s = String::new();
    if let Some(l) = r.label.as_deref() {
        s.push_str(l);
        s.push(' ');
    }
    s.push_str(&r.kind);
    s.push(' ');
    s.push_str(&r.snippet);
    s.to_lowercase()
}

/// Keep the rows matching **every** whitespace-separated token of `query`.
///
/// **AND, not OR** (`tree-selector.ts:393`: `searchTokens.every(...)`). Typing
/// "refactor auth" finds entries containing *both*, which is what a reader means
/// by two words; an OR shows everything matching either and buries the answer.
///
/// An empty query keeps everything, so the caller applies this unconditionally
/// rather than branching on whether a search is active.
pub(crate) fn apply_search(
    rows: &[crate::app::TimelineRow],
    query: &str,
) -> Vec<crate::app::TimelineRow> {
    let tokens: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if tokens.is_empty() {
        return rows.to_vec();
    }
    rows.iter()
        .filter(|r| {
            let hay = searchable_text(r);
            tokens.iter().all(|t| hay.contains(t.as_str()))
        })
        .cloned()
        .collect()
}

/// The whole visibility pipeline: filter -> search -> mandatory -> folds.
///
/// **One function, because there are two callers** (opening the picker, and
/// `refilter` when a chord changes something). Two copies would drift in a way
/// nothing catches: the picker opens showing rows the chord handler believes are
/// hidden, or the highlight lands on a row that is not drawn.
pub(crate) fn visible_rows(
    all: &[crate::app::TimelineRow],
    filter: crate::app::TreeFilter,
    labels: &std::collections::HashMap<String, String>,
    query: &str,
    folded: &std::collections::HashSet<String>,
) -> Vec<crate::app::TimelineRow> {
    let filtered = apply_filter(all, filter, labels);
    let searched = apply_search(&filtered, query);
    let with_mandatory = apply_mandatory(all, searched);
    apply_folds(&with_mandatory, folded)
}

/// The goal as a few lines of transcript.
///
/// One line per fact rather than a paragraph: the transcript wraps, and a wrapped sentence whose
/// continuation starts in the gutter reads as a second notice. The objective gets a line of its
/// own for the same reason — it is the part a reader came for.
fn goal_lines(view: &epsilon_contracts::goal::GoalView) -> Vec<String> {
    let g = &view.goal;
    let mut lines = vec![format!(
        "goal {} · rounds {}/{} · {}",
        g.status.as_str(),
        g.rounds_started,
        g.max_rounds,
        if view.armed {
            "continuation armed"
        } else {
            "continuation disarmed"
        }
    )];
    lines.push(g.objective.clone());
    if let Some(blocker) = &g.blocker {
        lines.push(format!("blocked by {}: {}", blocker.code, blocker.reason));
    }
    lines.push(format!("id {} revision {}", g.id, g.revision));
    lines
}

impl App {
    /// Route a parsed slash command to the executor that owns it.
    ///
    /// Replies are pushed synchronously into the transcript so they render on the very next
    /// frame; the notices channel stays reserved for async events. Returns `true` to quit.
    pub(super) fn run_command(
        &mut self,
        agent: &Arc<Agent>,
        command: crate::picker::Command,
    ) -> bool {
        match &command {
            // Who answers, and how hard they think.
            crate::picker::Command::Models
            | crate::picker::Command::Model(_)
            | crate::picker::Command::Think(_)
            | crate::picker::Command::ScopedModels
            | crate::picker::Command::Login { .. }
            | crate::picker::Command::Logout(_) => self.run_provider_command(agent, command),
            // What the session is.
            crate::picker::Command::Fork
            | crate::picker::Command::New
            | crate::picker::Command::Resume(_)
            | crate::picker::Command::Sessions
            | crate::picker::Command::Session
            | crate::picker::Command::Tree
            | crate::picker::Command::Clone
            | crate::picker::Command::Reload
            | crate::picker::Command::Name(_)
            // M196: mutates, so it does NOT belong with the view commands. A
            // `/skill` that only printed would leave a setting nothing could
            // change -- the "advertised but read by nothing" shape.
            | crate::picker::Command::Skill(_)
            // M203: mutates the session's goal, and the bare form reads it. It lives here
            // rather than with the view commands because half its verbs write.
            | crate::picker::Command::Goal(_)
            // ADR-032: the session's permission posture — bare reads it,
            // a name switches it (full-access confirms first).
            | crate::picker::Command::Preset(_) => self.run_session_command(agent, command),
            // Show the user something.
            crate::picker::Command::Help
            | crate::picker::Command::Skills
            | crate::picker::Command::Templates
            | crate::picker::Command::Tasks
            | crate::picker::Command::Timeline
            | crate::picker::Command::Hotkeys
            | crate::picker::Command::Settings
            | crate::picker::Command::Changelog => self.run_view_command(agent, command),
            // Move bytes or permissions across the trust boundary.
            crate::picker::Command::Trust(_)
            | crate::picker::Command::Export(_)
            | crate::picker::Command::Import(_)
            | crate::picker::Command::Share
            | crate::picker::Command::Copy => self.run_transfer_command(agent, command),
            // The catch-all: the conversation and the frame.
            _ => self.run_workspace_command(agent, command),
        }
    }

    /// Provider and model commands.
    fn run_provider_command(
        &mut self,
        agent: &Arc<Agent>,
        command: crate::picker::Command,
    ) -> bool {
        match command {
            crate::picker::Command::Models => {
                self.picker_open = true;
                self.picker_index = 0;
                // Reuse the editor line as the picker's filter box.
                self.input.clear();
                self.cursor_chars = 0;
                // M20 auto-fetch (opencode/Hermes convention): the highlighted
                // provider's live list starts refreshing immediately, so by the
                // time the user descends into it the rows are current.
                self.request_live(false);
            }
            crate::picker::Command::Model(id) => {
                // Hybrid (P-247): one candidate applies instantly; zero or
                // many open the picker with the text pre-filled (pi's
                // selector) instead of switching to — and saving — a typo or
                // a guess. Candidates are the same resolution a run uses, so
                // "applies" and "runs" can never disagree on what names a
                // model — and "deepseek" (a family, not a model) was the
                // case that used to pick a winner silently.
                if epsilon_llm::catalog::resolve_model_candidates(id.trim()).len() == 1 {
                    self.apply_model(agent, id)
                } else {
                    self.picker_open = true;
                    self.picker_provider = None;
                    self.picker_index = 0;
                    self.input = id.trim().to_string();
                    self.cursor_chars = self.input.chars().count();
                    self.invalidate_picker();
                    self.ui.push_notice(format!(
                        "no single model '{id}' — picker opened; refine or pick",
                    ));
                }
            }
            crate::picker::Command::Think(arg) => {
                match arg.as_deref().map(str::trim) {
                    None => {
                        // Direct push: slash replies render this frame.
                        let cur = agent.thinking_level();
                        self.ui
                            .push_notice(format!("thinking level: {}", cur.as_str()));
                    }
                    Some(word) => match epsilon_contracts::ThinkingLevel::parse(word) {
                        Some(level) => self.set_thinking(agent, level),
                        None => {
                            // Direct push: slash replies render this frame.
                            self.ui.push_notice(format!(
                                "unknown level '{word}' - use: {}",
                                levels_help()
                            ));
                        }
                    },
                }
            }
            crate::picker::Command::ScopedModels => {
                if self.favorites.is_empty() {
                    self.ui
                        .push_notice("scoped-models: no favorites yet — open Ctrl+P and press Ctrl+F on a model to star it, then Ctrl+P cycles favorites");
                } else {
                    // Ponytail: route to the model picker seeded with favorites (pi scopedModels → model picker).
                    self.picker_open = true;
                    self.picker_index = 0;
                    self.invalidate_picker();
                    self.ui.push_notice(format!(
                        "scoped-models: {} favorite(s) — cycling favorites · open Ctrl+P to pick, Ctrl+F to star/unstar",
                        self.favorites.len()
                    ));
                }
            }
            crate::picker::Command::Login { provider, key } => {
                let provider = provider.trim().to_string();
                // M52: `/login <provider> oauth` (or `--oauth`) starts the
                // subscription ceremony instead of storing a pasted key.
                if !provider.is_empty() && epsilon_llm::oauth_login::is_oauth_ceremony_arg(&key) {
                    self.start_oauth_login(provider);
                    return false;
                }
                if provider.is_empty() {
                    // Bare `/login` — open the provider picker so the user
                    // picks the target instead of having to type the id.
                    // Mirrors opencode's DialogProvider select step.
                    self.auth_pick_open = true;
                    self.auth_pick_index = 0;
                    self.input.clear();
                    self.cursor_chars = 0;
                    self.ui.push_notice(
                        "connect a provider — pick one (↑/↓ Enter), or type to filter",
                    );
                    return false;
                }
                if key.trim().is_empty() {
                    // Masked prompt overlay like opencode's DialogPrompt placeholder="API key".
                    self.auth_prompt_open = true;
                    self.auth_provider = provider.clone();
                    self.auth_key.clear();
                    self.auth_key_cursor = 0;
                    let oauth_hint = if epsilon_llm::oauth_login::login_flow(&provider).is_some() {
                        " · Ctrl+O = subscription login (OAuth)"
                    } else {
                        ""
                    };
                    self.ui.push_notice(format!(
                        "enter API key for '{provider}' — paste the key (masked), Enter saves to ~/.config/epsilon/auth.json, Esc cancels{oauth_hint}"
                    ));
                    return false;
                }
                match epsilon_llm::auth::save_credential(&provider, key.trim()) {
                    Ok(path) => {
                        self.ui.push_notice(format!(
                            "saved key for '{provider}' → {}  (next request uses it)",
                            path.display()
                        ));
                        self.invalidate_picker();
                    }
                    Err(e) => self
                        .ui
                        .push_notice(format!("not saved for '{provider}': {e}")),
                }
            }
            crate::picker::Command::Logout(provider) => {
                let provider = provider.trim().to_string();
                if provider.is_empty() {
                    self.ui
                        .push_notice("usage: /logout <provider> — e.g. /logout anthropic");
                    return false;
                }
                match epsilon_llm::auth::remove_credential(&provider) {
                    Ok(true) => {
                        self.ui
                            .push_notice(format!("removed stored key for '{provider}'"));
                        self.invalidate_picker();
                    }
                    Ok(false) => self
                        .ui
                        .push_notice(format!("no stored key for '{provider}' (nothing removed)")),
                    Err(e) => self
                        .ui
                        .push_notice(format!("not removed for '{provider}': {e}")),
                }
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_provider_command case");
                return false;
            }
        }
        false
    }

    /// Session-identity commands.
    fn run_session_command(&mut self, agent: &Arc<Agent>, command: crate::picker::Command) -> bool {
        match command {
            crate::picker::Command::Sessions
            | crate::picker::Command::Session
            | crate::picker::Command::Tree => self.run_navigate_command(agent, command),
            crate::picker::Command::Fork
            | crate::picker::Command::Clone
            | crate::picker::Command::Name(_) => self.run_branch_command(agent, command),
            crate::picker::Command::New
            | crate::picker::Command::Resume(_)
            | crate::picker::Command::Reload => self.run_restore_command(agent, command),
            crate::picker::Command::Preset(arg) => self.run_preset_command(agent, arg),
            crate::picker::Command::Skill(arg) => {
                let arg = arg.trim();
                let (name, want_off) = match arg.rsplit_once(char::is_whitespace) {
                    Some((n, "off")) | Some((n, "disable")) => (n.trim(), true),
                    Some((n, "on")) | Some((n, "enable")) => (n.trim(), false),
                    _ => {
                        self.ui.push_notice(
                            "usage: /skill <name> on|off — `/skills` lists what is available",
                        );
                        return false;
                    }
                };
                if name.is_empty() {
                    self.ui.push_notice("usage: /skill <name> on|off");
                    return true;
                }
                // **The name must be one that EXISTS.** Writing a disable for a
                // skill nothing provides would produce a setting that does
                // nothing, and that reads back as a disabled skill the user
                // cannot find to re-enable -- the worst of both. This is the
                // check that makes the config layer's contract true.
                let known = agent.skills().iter().any(|s| s.name == name);
                if !known {
                    self.ui.push_notice(format!(
                        "no skill named '{name}' — `/skills` lists what is available"
                    ));
                    return true;
                }
                match epsilon_core::config::set_skill_disabled(name, want_off) {
                    Ok(_) => self.ui.push_notice(format!(
                        "{name} {} — takes effect on the next turn",
                        if want_off { "off" } else { "on" }
                    )),
                    Err(e) => self.ui.push_notice(format!("could not change {name}: {e}")),
                }
                // `run_session_command` returns `bool` = "quit". `/skill` changes
                // a setting and leaves the composer alone, so it must answer
                // `false` -- a `true` here ended the session, which the
                // all-commands sweep caught immediately.
                false
            }
            crate::picker::Command::Goal(arg) => {
                self.run_goal_command(agent, arg.as_deref());
                false
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_session case");
                false
            }
        }
    }

    /// `/goal` — the session's one long-running objective.
    ///
    /// The whole tail arrives here unparsed, because an objective is a sentence: splitting it in
    /// the command parser is how a goal ends up reading "make the doc". Only a *leading word*
    /// that names one of the verbs is treated as one, and only when nothing else could be meant
    /// — `edit` and `clear` are the two where a bare objective would otherwise be ambiguous, so
    /// `pause`, `resume` and `complete` are recognised the same way for consistency. Everything
    /// else creates.
    ///
    /// Every path answers in the transcript: the M196 rule that a silently swallowed action is
    /// indistinguishable from a broken program applies to commands at least as much as to keys,
    /// and a `/goal` that printed nothing after a typo would read as a hang.
    fn run_goal_command(&mut self, agent: &Arc<Agent>, arg: Option<&str>) {
        // **Parsing lives in `epsilon-runtime::goal_command`** (M267). It was ninety lines
        // here, which is how the desktop window came to refuse `/goal edit`, `/goal complete`
        // and `/goal blocked` by name: the rules belonged to a frontend, so the other frontend
        // had none. The rules are the verb table, the `rounds <n>` sub-form and the
        // replace-don't-append gate list, and none of them are about pixels.
        //
        // What stays here is the *application*, and the two halves are genuinely different:
        // a created or resumed goal starts a round here, through `continue_goal`.
        let op = match epsilon_runtime::goal_command(arg) {
            epsilon_runtime::GoalCommand::Show => {
                self.notice_goal(agent);
                return;
            }
            epsilon_runtime::GoalCommand::Help(lines) => {
                for line in lines {
                    self.ui.push_notice(*line);
                }
                return;
            }
            epsilon_runtime::GoalCommand::Invalid(why) => {
                self.ui.push_notice(&why);
                return;
            }
            epsilon_runtime::GoalCommand::Apply(op) => op,
        };
        // No compare-and-set ref: the user is looking at the current goal and acting on it, and
        // asking them to type a revision would be asking them to do the harness's bookkeeping.
        // The model's path (`update_goal`) does carry one, which is where staleness is real.
        //
        // Creating or resuming a goal starts the first round immediately when the session is
        // idle: `/goal fix the gates` that parked an objective and then sat there until the user
        // typed something else would be a note, not a goal. `pause` is how a user parks one
        // without starting it, and the round it spends is counted and reported like any other.
        let starts_work = matches!(
            op,
            epsilon_contracts::goal::GoalOp::Create { .. }
                | epsilon_contracts::goal::GoalOp::Resume
        );
        match agent.goal_apply(None, op) {
            Ok(Some(view)) => {
                self.ui.push_notice(goal_lines(&view).join("\n"));
                self.goal_badge = Some(view.badge());
                if starts_work {
                    self.continue_goal(agent);
                }
            }
            Ok(None) => {
                self.ui.push_notice("goal cleared");
                self.goal_badge = None;
            }
            Err(e) => self.ui.push_notice(format!("/goal refused: {e}")),
        }
    }

    /// Print the current goal, or say there is none — and how to start one.
    fn notice_goal(&mut self, agent: &Arc<Agent>) {
        match agent.goal() {
            Some(view) => {
                self.ui.push_notice(goal_lines(&view).join("\n"));
                self.goal_badge = Some(view.badge());
            }
            None => {
                self.goal_badge = None;
                self.ui.push_notice(
                    "no goal is set — `/goal <what to achieve>` starts one, `/goal help` lists \
                     the verbs",
                );
            }
        }
    }

    /// Say that this session already holds history, when this launch is not showing it.
    ///
    /// Ledger A17. `--resume` seeds the runtime history; a plain launch does not, and it still
    /// appends to the same file — so a returning user gets a first-run screen while the header
    /// reports a few percent of the context already used. Both facts are true (that figure is the
    /// system prompt and the tool schemas, per M194's C2 fix) and together they read as a bug.
    /// One sentence closes it, and it names the command that brings the history back rather than
    /// leaving the user to guess that the file still has it.
    ///
    /// Silent when the history *is* restored — the condition is "the store has entries and this
    /// process is not carrying them", which is exactly what makes the screen look wrong.
    ///
    /// **The count is the union `epsilon -c` resolves (M222, A17's open half).** This used to list
    /// the directory the launch's own file sits in, which since M217 is
    /// `sessions/--<cwd>--/` — so it saw this workspace's own sessions and **none of the pre-M217
    /// files in the flat root**, which `-c` opens too (M217 reads both). A notice that stays
    /// silent while `-c` has something to open is the reader/writer asymmetry M217 removed
    /// everywhere else, reached from the other end: the same argument cannot be true of this count
    /// and of that one.
    pub(crate) fn note_prior_history(&mut self, agent: &Agent) {
        if !agent.history().is_empty() {
            return;
        }
        // A20: this used to count the entries of the store *this launch opened*, which was the right
        // question while every frontend opened one fixed file — the file held earlier conversations
        // the model could not see, and saying so was the point. M210 gave each launch a file of its
        // own, so that count is now zero on every plain launch and the notice never fired. The
        // question that survived the change is "does this workspace know about earlier
        // conversations?", and the answer lives in the directory rather than in this file.
        let Some(own) = agent.session_path() else {
            return;
        };
        // M231: a **count**, not a list. `list_sessions_for_cwd` summarizes every file it finds --
        // three opens each (header, an 8 KiB tail, the first 64 lines) -- and at 500 sessions that
        // measured **202 ms at every launch**, which this notice paid in full to print one number.
        // `count_sessions_for_cwd` is the same union by name in the per-cwd directory, where a file
        // is this workspace by construction, and by header in the flat root. A test pins that it
        // agrees with the resolver `-c` itself uses: a notice saying "3 earlier sessions" while `-c`
        // opens a fourth is the defect this avoids.
        let earlier =
            epsilon_session::count_sessions_for_cwd(&self.session_dir(), &self.workspace_root)
                .saturating_sub(usize::from(own.file_name().is_some()));
        if earlier == 0 {
            return;
        }
        self.ui.push_notice(format!(
            "this workspace holds {earlier} earlier session{} — `epsilon -c` opens the most \
             recent, `/resume <file>` picks another",
            if earlier == 1 { "" } else { "s" }
        ));
    }

    /// Start the next round of an armed goal, if this is the moment for one.
    ///
    /// Called at a turn end. Four conditions have to hold, and each one is a way the harness
    /// could otherwise do something the person in front of it did not ask for:
    ///
    /// * **The goal is continuable** — active, armed in this process, and under its cap. All of
    ///   that is re-read from the log here rather than remembered, so a tool that completed or
    ///   paused the goal during the turn is respected immediately.
    /// * **Nothing is in flight**, and no permission ask is parked. Belt-and-braces at a turn
    ///   end, and both would be a disaster if they were ever true.
    /// * **The composer is empty.** A user halfway through typing a follow-up has not asked for
    ///   another round, and starting one would race their Enter key. The goal stays armed, so
    ///   their next turn continues it.
    ///
    /// The caller is responsible for one more condition, which it can see and this cannot: a run
    /// that ended *without producing a message* (a provider error) does not continue, or an
    /// unreachable provider would spend the whole round budget in a hot loop.
    ///
    /// Deliberately **not** `submit_text`: that records the text in the Up-arrow recall ring and
    /// in the global prompt-history file, and a harness-authored continuation is not something a
    /// person typed. What it shares is the transcript echo and the message constructor, so a
    /// round is visible and logged exactly like any other turn.
    ///
    /// Returns whether a round was started, which is what the tests assert on.
    pub(super) fn continue_goal(&mut self, agent: &Arc<Agent>) -> bool {
        if self.ui.running() || self.pending_ask.is_some() {
            return false;
        }
        if !self.input.trim().is_empty() {
            return false;
        }
        let Some(view) = agent.goal() else {
            return false;
        };
        if !view.is_continuable() {
            // A goal that is armed, active and out of rounds is the one state worth a sentence:
            // the loop stopped for a reason the user has to change something to fix, and silence
            // would read as "it forgot".
            if view.armed && view.goal.status.is_continuable() && view.goal.exhausted() {
                self.ui.push_notice(format!(
                    "goal round cap reached ({}/{}): `/goal edit rounds <n>` raises it",
                    view.goal.rounds_started, view.goal.max_rounds
                ));
            }
            return false;
        }
        // **A goal with gates is checked before its round is sent**, because a round that
        // re-reads "continue toward the objective" while the suite is red is the loop
        // Hermes' design exists to prevent. The check runs on its own task because a gate is
        // a shell command — thirty seconds of `cargo test` must not be thirty seconds of
        // dead terminal — and the round starts when it answers, in `start_gate_round`.
        if view.goal.gates.iter().any(|g| g.is_runnable()) && self.gate_runner.is_some() {
            if self.goal_round_pending {
                return false;
            }
            self.goal_round_pending = true;
            let agent = Arc::clone(agent);
            let runner = self.gate_runner.clone();
            let cwd = self.workspace_root.clone();
            let tx = self.goal_round_tx.clone();
            let notices = self.notices_tx.clone();
            tokio::spawn(async move {
                let round = epsilon_runtime::begin_goal_round(&agent, runner.as_deref(), &cwd)
                    .await
                    .unwrap_or(epsilon_runtime::GoalRound::Stop);
                if let epsilon_runtime::GoalRound::GateFailed { reports, .. } = &round {
                    for r in reports {
                        notices.send(r.sentence()).ok();
                    }
                }
                tx.send(round).ok();
            });
            return false;
        }
        let Ok(Some(prompt)) = agent.goal_begin_round() else {
            return false;
        };
        self.ui.push_notice(format!(
            "goal round {}/{} — continuing",
            view.goal.rounds_started + 1,
            view.goal.max_rounds
        ));
        self.note_goal_budget(view.goal.rounds_started + 1, view.goal.max_rounds);
        self.ui.push_local_input(&prompt);
        let msg = epsilon_core::references::user_message(&self.workspace_root, &prompt);
        self.spawn_run(Arc::clone(agent), msg);
        true
    }

    /// Start a round a gate check has just answered for.
    ///
    /// The other half of the spawn in [`Self::continue_goal`]. Kept separate so the gate's
    /// own timing never reaches the frame loop, and so a round still starts from exactly one
    /// place whether or not gates are involved.
    pub(super) fn start_gate_round(
        &mut self,
        agent: &Arc<Agent>,
        round: epsilon_runtime::GoalRound,
    ) -> bool {
        self.goal_round_pending = false;
        let Some(view) = agent.goal() else {
            return false;
        };
        let (prompt, gates_passed) = match round {
            epsilon_runtime::GoalRound::Continue {
                prompt,
                gates_passed,
            } => (prompt, gates_passed),
            epsilon_runtime::GoalRound::GateFailed { prompt, .. } => (prompt, 0),
            epsilon_runtime::GoalRound::Stop => return false,
        };
        let note = if gates_passed > 0 {
            format!(
                "goal round {}/{} — {gates_passed} gate(s) passed",
                view.goal.rounds_started, view.goal.max_rounds
            )
        } else {
            format!(
                "goal round {}/{} — continuing",
                view.goal.rounds_started, view.goal.max_rounds
            )
        };
        self.ui.push_notice(note);
        self.note_goal_budget(view.goal.rounds_started, view.goal.max_rounds);
        self.ui.push_local_input(&prompt);
        let msg = epsilon_core::references::user_message(&self.workspace_root, &prompt);
        self.spawn_run(Arc::clone(agent), msg);
        true
    }

    /// Warn as a goal's round budget runs down.
    ///
    /// Rounds are the only budget a goal has (DSH's design, and deliberate), which means the cap
    /// is what stops a loop that a human has stopped watching — and a user who set a goal and
    /// walked away gets no signal at all until it stops. This is a small ladder rather than a
    /// threshold: each step fires once as it is crossed, so the warning cannot become the noise
    /// that gets ignored, and the last two are the ones that matter.
    ///
    /// The numbers are fractions of the cap (10%, 4%, 2%, and the final round) rather than fixed
    /// counts, because `/goal edit rounds <n>` can set any cap at all. `rounds_started` is the
    /// count *after* the round that is starting, so the two notices a continuing goal prints
    /// agree about which round it is on.
    fn note_goal_budget(&mut self, rounds_started: u32, max_rounds: u32) {
        let remaining = max_rounds.saturating_sub(rounds_started);
        let ladder = [
            (max_rounds / 10).max(1),
            (max_rounds / 25).max(1),
            (max_rounds / 50).max(1),
            1,
        ];
        if !ladder.contains(&remaining) {
            return;
        }
        self.ui.push_notice(format!(
            "goal has {remaining} of {max_rounds} rounds left — `/goal edit rounds <n>` raises \
             the cap, `/goal pause` stops it"
        ));
    }

    fn run_navigate_command(
        &mut self,
        agent: &Arc<Agent>,
        command: crate::picker::Command,
    ) -> bool {
        match command {
            crate::picker::Command::Sessions => {
                self.open_session_browser(agent);
            }
            crate::picker::Command::Session => {
                // No durable mirror is a normal state (a plain `--tui` run, a child agent), not a
                // crash, so every command that needs the store asks for the Option.
                // M168: nine call sites used `Agent::store_handle_for_tests` instead, which
                // panics on exactly this state — `/tree`, `/timeline`, `/fork`, `/clone`,
                // `/name`, `/export` and `/share` all took the TUI down. `session_store` is
                // the accessor for this; the test-only one is `#[doc(hidden)]` for a reason.
                let (path, leaf, count) = match agent.session_store() {
                    Some(store) => match store.lock() {
                        Ok(st) => (
                            st.path().to_path_buf(),
                            st.leaf_id().unwrap_or_else(|| "(none)".into()),
                            st.branch_entries().map(|v| v.len()).unwrap_or(0),
                        ),
                        Err(_) => {
                            self.ui.push_notice("session: store unavailable");
                            return false;
                        }
                    },
                    None => {
                        let (tin, tout) = self.ui.usage();
                        self.ui.push_notice(format!(
                            "session: no durable session file attached \u{b7} tokens {tin}/{tout} \u{b7} model {}",
                            self.model_id
                        ));
                        // Cost, broken down (M143). The header shows one number; this says where
                        // it went. A total alone cannot tell you the cache is working, and the
                        // cache is the main lever a user has on cost.
                        self.ui.push_notice(self.cost_summary());
                        for line in agent.cache_report() {
                            self.ui.push_notice(line);
                        }
                        for line in repair_lines(agent) {
                            self.ui.push_notice(line);
                        }
                        return false;
                    }
                };
                let (tin, tout) = self.ui.usage();
                if let Some(n) = self.session_name.as_deref() {
                    self.ui.push_notice(format!(
                        "session: '{n}' · {} · leaf {} · {} entries · tokens {tin}/{tout} · model {}",
                        path.display(),
                        leaf,
                        count,
                        self.model_id
                    ));
                } else {
                    self.ui.push_notice(format!(
                        "session: {} · leaf {} · {} entries · tokens {tin}/{tout} · model {}",
                        path.display(),
                        leaf,
                        count,
                        self.model_id
                    ));
                }
                // Cache attribution (M69): the footer's CH% says whether the prompt cache is
                // paying off; these lines say what it cost when it did not.
                for line in agent.cache_report() {
                    self.ui.push_notice(line);
                }
                // CONTROL (M143): the cost breakdown line removed — the state before this
                // slice, where the four per-component accessors had no caller at all.
                // Session repairs (M75): a resumed session may have been missing tool results or
                // an end marker. The kinds are worth naming — they are what the user lost.
                for line in repair_lines(agent) {
                    self.ui.push_notice(line);
                }
            }
            crate::picker::Command::Tree => {
                let _ = self.open_timeline_picker(agent, "tree");
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_navigate case");
                return false;
            }
        }
        false
    }

    /// Show or switch the permission preset (ADR-032).
    ///
    /// Bare lists the four with the current marked; a name switches
    /// immediately except `danger-full-access`, which holds the question in
    /// `pending_preset` until the reader types `yes` (answered in
    /// `submit_editor`, next to the trust and import gates). Switching down
    /// never confirms (D-080: only the raising direction gates).
    fn run_preset_command(&mut self, agent: &Arc<Agent>, arg: Option<String>) -> bool {
        let current = agent.current_preset();
        let Some(name) = arg.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            self.ui.push_notice(format!(
                "permission preset: {current} (current) — auto | workspace-write | read-only | danger-full-access"
            ));
            return false;
        };
        let Ok(preset) = epsilon_safety::Preset::parse(name) else {
            self.ui.push_notice(format!(
                "unknown preset '{name}' — one of: auto, workspace-write, read-only, danger-full-access"
            ));
            return false;
        };
        if preset.raises_privilege() {
            self.pending_preset = Some(preset.name().to_string());
            self.ui.push_notice(format!(
                "Switch permission preset to '{}'? This allows everything nothing forbids. (yes/no)",
                preset.name()
            ));
            return false;
        }
        if agent.set_preset(preset.name(), &self.workspace_root) {
            self.ui.push_notice(format!(
                "permission preset: {} (was {current})",
                preset.name()
            ));
        } else {
            self.ui.push_notice(format!(
                "preset not switched — the gate reported no change ({})",
                preset.name()
            ));
        }
        false
    }

    fn run_branch_command(&mut self, agent: &Arc<Agent>, command: crate::picker::Command) -> bool {
        match command {
            crate::picker::Command::Fork => {
                let Some(store) = agent.session_store() else {
                    self.ui.push_notice("no session store attached");
                    return false;
                };
                let (path, leaf) = match store.lock() {
                    Ok(st) => (st.path().to_path_buf(), st.leaf_id()),
                    Err(_) => {
                        self.ui.push_notice("fork: session store unavailable");
                        return false;
                    }
                };
                if leaf.is_none() {
                    self.ui
                        .push_notice("fork: nothing to fork — session is empty");
                    return false;
                }
                // The store's `fork_branch_at`, not `std::fs::copy`.
                //
                // A copy duplicated the whole file — every abandoned branch and every
                // dead path — under the *same* session id in the header, so the two
                // files claimed one identity. `fork_branch_at` writes a fresh header
                // (`store.rs:1531-1536`) and copies only `branch_to(leaf)`, which is
                // what pi's `forkFrom` does (`session-manager.ts:1611-1634`).
                //
                // That correct implementation already existed and was wired **only**
                // from the RPC surface — so the transport a human uses had the worse
                // one. Same defect class as `branch_with_summary`: a faithful port with
                // no production caller on the path anyone actually takes.
                let Some(leaf_id) = leaf else { return false };
                let dest_dir = match path.parent() {
                    Some(d) => d.to_path_buf(),
                    None => {
                        self.ui
                            .push_notice("fork: the session file has no parent directory");
                        return false;
                    }
                };
                let forked = match store.lock() {
                    Ok(st) => st.fork_branch_at(&leaf_id, &dest_dir),
                    Err(_) => {
                        self.ui.push_notice("fork: session store unavailable");
                        return false;
                    }
                };
                match forked {
                    Ok(dest) => self.ui.push_notice(format!(
                        "forked session → {} (open with --session {})",
                        dest.display(),
                        dest.display()
                    )),
                    Err(e) => self.ui.push_error(format!("fork failed: {e}")),
                }
            }
            crate::picker::Command::Clone => {
                let Some(store) = agent.session_store() else {
                    self.ui.push_notice("no session store attached");
                    return false;
                };
                let (path, leaf) = match store.lock() {
                    Ok(st) => (st.path().to_path_buf(), st.leaf_id()),
                    Err(_) => {
                        self.ui.push_notice("clone: session store unavailable");
                        return false;
                    }
                };
                if leaf.is_none() {
                    self.ui
                        .push_notice("clone: nothing to clone — session is empty");
                    return false;
                }
                // Same correction as `/fork`: pi's clone duplicates the *current
                // branch* from the current leaf under a fresh id, and a whole-file
                // `std::fs::copy` duplicated every abandoned branch under the same
                // one. `fork_branch_at(leaf)` is exactly that, and it is what the RPC
                // `clone` already calls — so the two surfaces now agree.
                let Some(leaf_id) = leaf else { return false };
                let dest_dir = match path.parent() {
                    Some(d) => d.to_path_buf(),
                    None => {
                        self.ui
                            .push_notice("clone: the session file has no parent directory");
                        return false;
                    }
                };
                let cloned = match store.lock() {
                    Ok(st) => st.fork_branch_at(&leaf_id, &dest_dir),
                    Err(_) => {
                        self.ui.push_notice("clone: session store unavailable");
                        return false;
                    }
                };
                match cloned {
                    Ok(dest) => self.ui.push_notice(format!(
                        "cloned session → {} (open with --session {})",
                        dest.display(),
                        dest.display()
                    )),
                    Err(e) => self.ui.push_error(format!("clone failed: {e}")),
                }
            }
            crate::picker::Command::Name(arg) => {
                let name = arg.as_deref().unwrap_or("").trim();
                if name.is_empty() {
                    if let Some(n) = self.session_name.clone() {
                        self.ui.push_notice(format!("session name: '{n}'"));
                    } else {
                        // Also check persisted Custom entry (survives resume before in-memory is set).
                        let persisted: Option<String> = (|| {
                            let store = agent.session_store()?;
                            let st = store.lock().ok()?;
                            let entries = st.read_all().ok()?;
                            for e in entries.iter().rev() {
                                if let epsilon_contracts::session_entry::EntryKind::Custom {
                                    custom_type,
                                    data,
                                } = &e.kind
                                    && custom_type == "session_name"
                                    && let Some(v) = data
                                        .as_ref()
                                        .and_then(|d| d.get("name"))
                                        .and_then(|v| v.as_str())
                                {
                                    return Some(v.to_string());
                                }
                            }
                            None
                        })();
                        if let Some(n) = persisted {
                            self.session_name = Some(n.clone());
                            self.refresh_title();
                            self.ui.push_notice(format!("session name: '{n}'"));
                        } else {
                            self.ui.push_notice(
                                "usage: /name <display name> — sets session display name",
                            );
                        }
                    }
                } else {
                    self.session_name = Some(name.to_string());
                    // Persist as non-LLM Custom entry so it survives restarts via
                    // the same session file pi/gap-analysis parity intended (extend
                    // to a dedicated EntryKind when a UI needs to render it).
                    let persisted = (|| -> Result<(), String> {
                        let store = agent.session_store().ok_or("no session store attached")?;
                        let mut st = store.lock().map_err(|_| "store lock".to_string())?;
                        st.append(epsilon_contracts::session_entry::EntryKind::Custom {
                            custom_type: "session_name".into(),
                            data: Some(serde_json::json!({"name": name})),
                        })
                        .map_err(|e| format!("{e}"))?;
                        Ok(())
                    })();
                    // **Say which of the two happened (M312).** The name is set in memory either
                    // way, so the only honest notice names what was persisted — and the refusal
                    // this reports is the one that now exists: a write refused because another
                    // process holds the session. `let _ =` on that is the silent failure M311
                    // closed twice in the core, and it would have been a third instance of the
                    // same shape one layer up.
                    match persisted {
                        Ok(()) => {
                            self.refresh_title();
                            self.ui.push_notice(format!("session name set to '{name}'"))
                        }
                        Err(why) => self.ui.push_notice(format!(
                            "session named '{name}' for this run, but not saved: {why}"
                        )),
                    }
                }
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_branch case");
                return false;
            }
        }
        false
    }

    fn run_restore_command(&mut self, agent: &Arc<Agent>, command: crate::picker::Command) -> bool {
        match command {
            crate::picker::Command::New => {
                // M221: `/new` starts a new session **file**, not a cleared transcript. Since M210
                // a launch owns its file, and since M217 that file lives in a directory per working
                // copy — so "new session" is the same act as "launch": name a file beside the others
                // and attach it through the ONE switch path (`switch_session_file`, shared with
                // `/resume` and the browser). The old behaviour cleared the visible rows and kept
                // appending to the same file, which is the one-file-many-conversations primitive
                // this model removed — and it is why `reset_leaf` had no caller.
                if self.ui.running() || agent.is_running() {
                    self.ui.push_notice(
                        "new: busy — wait for the current run to finish, then /new again",
                    );
                    return true;
                }
                let (tin, tout) = self.ui.usage();
                let dir = epsilon_session::time::session_dir_for(
                    &self.session_dir(),
                    &self.workspace_root,
                );
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    self.ui
                        .push_notice(format!("new session: {}: {e}", dir.display()));
                    return true;
                }
                // pi's file name, from the rule M209 transcribed and M217 put behind a directory.
                let path = dir.join(epsilon_session::time::session_file_name(
                    now_ms(),
                    &epsilon_contracts::new_id("ses"),
                ));
                // Create it now, and drop the handle immediately: the store takes a cross-process
                // write lease, and `switch_session_file` opens the same file again — holding both
                // handles would have this process wait on its own lease.
                if let Err(e) = epsilon_session::SessionStore::open_in(&path, &dir).map(|_| ()) {
                    self.ui
                        .push_notice(format!("new session: {}: {e}", path.display()));
                    return true;
                }
                match self.switch_session_file(agent, &path, None) {
                    Ok(_) => self.ui.push_notice(format!(
                        "new session — {} (the previous one is kept; /sessions lists both, \
                         tokens {tin}/{tout})",
                        path.strip_prefix(&self.workspace_root)
                            .unwrap_or(&path)
                            .display()
                    )),
                    Err(e) => self.ui.push_error(format!("new session failed: {e}")),
                }
            }
            crate::picker::Command::Resume(path) => {
                let p = path.trim();
                if p.is_empty() {
                    // M57: bare `/resume` opens the browser — the path form is
                    // still there for scripts and muscle memory.
                    self.open_session_browser(agent);
                } else if self.ui.running() || agent.is_running() {
                    self.ui.push_notice(
                        "resume: busy — wait for the current run to finish, then /resume again",
                    );
                } else {
                    let raw = std::path::PathBuf::from(p);
                    let abs = if raw.is_absolute() {
                        raw
                    } else {
                        self.workspace_root.join(raw)
                    };
                    if !abs.exists() {
                        self.ui.push_notice(format!(
                            "resume: no file at '{}' — check the path",
                            abs.display()
                        ));
                    } else {
                        match self.switch_session_file(agent, &abs, Some(p)) {
                            Ok(count) => self.ui.push_notice(format!(
                                "resumed {} messages from {} (history restored, store swapped)",
                                count,
                                abs.display()
                            )),
                            Err(e) => self.ui.push_error(format!("resume failed: {e}")),
                        }
                    }
                }
            }
            crate::picker::Command::Reload => {
                let mut roots = Vec::new();
                if let Some(dir) = epsilon_core::config::global_dir() {
                    roots.push(dir.join("skills"));
                }
                roots.push(self.workspace_root.join(".agents").join("skills"));
                roots.push(self.workspace_root.join(".epsilon").join("skills"));
                // `discover_with_notices`, not `discover`: the wrapper throws away everything
                // discovery could not do, and `/reload` is the one command whose whole promise
                // is "read what is on disk now". An unreadable root, a `SKILL.md` that will not
                // parse, a skill shadowed by a later root — each produced a count and no reason,
                // which is the silent failure `AGENTS.md` §2.7 forbids and §8 names again. The
                // composition path already routes the same lines into the startup notices the
                // terminal reads, so this call site was the last place a skill could fail to
                // load with nobody told.
                let found = epsilon_core::skills::discover_with_notices(&roots);
                let n = found.skills.len();
                agent.set_skills(found.skills);
                // The transcript, like every other reply in this file (`run_command`'s own doc:
                // synchronous so they render on the very next frame, leaving the notices channel
                // for async events). Discovery's lines are already formatted as whole sentences
                // naming the root and the reason, so they are pushed verbatim rather than
                // re-worded into a second vocabulary — this is forwarding, not authoring.
                for notice in &found.notices {
                    self.ui.push_notice(notice.clone());
                }
                // Templates reload with the skills: both are markdown in the same two
                // directories, and a user who edits one and types `/reload` expects both.
                let t = self.discover_prompt_templates();
                if t > 0 {
                    self.ui.push_notice(format!("prompt templates: {t}"));
                }
                // M50: agent defs reload alongside skills (same roots pattern —
                // global always, project pair gated by the trust bit pi's
                // agentScope maps onto: trust_review_pending means untrusted).
                //
                // `discover_agents_with_notices`, not the wrapper, for the same reason as the
                // skill call above: the comment that used to sit here said this needed a
                // `Discovered`-shaped return in `epsilon-core`, and that is what landed
                // (`agents::Discovered`, a sibling of `skills::Discovered`). A `.md` that will
                // not parse, or an agents root nobody can read, used to produce a count and no
                // reason here — the same silent failure the skill path had already fixed one
                // line of code above. Forwarded verbatim through `push_notice` for the same
                // reason: `run_command`'s own doc reserves the async notices channel for async
                // events, so the transcript is where a person reads this, above the count.
                let found = epsilon_core::agents::discover_agents_with_notices(
                    &{
                        let mut r = Vec::new();
                        if let Some(dir) = epsilon_core::config::global_dir() {
                            r.push(dir.join("agents"));
                        }
                        r
                    },
                    &[
                        self.workspace_root.join(".agents").join("agents"),
                        self.workspace_root.join(".epsilon").join("agents"),
                    ],
                    !self.trust_review_pending,
                );
                for notice in &found.notices {
                    self.ui.push_notice(notice.clone());
                }
                let an = found.agents.len();
                agent.set_agents(found.agents);
                // G6: re-read keybindings.json so a remap takes effect without restart (pi reload()).
                let kb_before = self.keybindings.effective();
                self.keybindings.reload();
                let kb_note = if self.keybindings.effective() != kb_before {
                    " · keybindings reloaded"
                } else {
                    ""
                };
                self.ui.push_notice(format!(
                    "reload: {n} skill(s) + {an} agent(s) loaded (prompt tier re-read on next turn){kb_note}"
                ));
                self.invalidate_picker();
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_restore case");
                return false;
            }
        }
        false
    }

    /// Everything the frame and the workspace do.
    /// Commands that act on the running conversation or the frame itself.
    ///
    /// This is the dispatcher's **catch-all**: a new `Command` variant lands here, where
    /// the compiler demands its arm, rather than going unrouted. That is why it is allowed
    /// to be the vague one - and why it is also kept small. The two executors it used to
    /// hold are `run_view_command` and `run_transfer_command`.
    fn run_workspace_command(
        &mut self,
        agent: &Arc<Agent>,
        command: crate::picker::Command,
    ) -> bool {
        match command {
            crate::picker::Command::Clear => {
                let (tin, tout) = self.ui.usage();
                self.ui.clear_lines();
                self.ui.push_notice(format!(
                    "transcript cleared — session totals kept (tokens {tin}/{tout})"
                ));
            }
            crate::picker::Command::Expand => {
                // The same action Ctrl+X performs, through the shared method, so the
                // menu and the key cannot drift into behaving differently.
                self.toggle_tool_expansion();
                // A command leaves a line a reader can find afterwards; the key path
                // does not, and must not — `app::golden::ctrl_x_opens_the_newest_card…`
                // asserts the frame is byte-identical, and a notice row there moved the
                // very card the test is about. Read the state back rather than predicting
                // it, so the two paths cannot report different things.
                self.ui.push_notice(if self.prefs.expand_outputs {
                    "all cards expanded"
                } else {
                    "all cards collapsed"
                });
            }
            crate::picker::Command::Search(query) => {
                // M157: this scanned the session *log on disk* and printed its hits as
                // transcript notices — a list of places you could go, printed into the one
                // place you already were. Nothing moved, nothing was highlighted, and with
                // no session log attached it answered "search unavailable this run", which
                // is exactly when a user most wants it.
                //
                // It searches the rendered transcript now, because that is what the reader
                // is looking at (pi searches the screen too, not the log). The trade is
                // honest and recorded in `search.rs`: the log reached further back than the
                // viewport, but it could not show you what it found.
                self.search_own_transcript(&query);
            }
            crate::picker::Command::Quit => return true,
            crate::picker::Command::Theme(arg) => {
                // The palette is a **view** choice, not session state, so it is applied to the
                // live theme and not written to config: a cosmetic toggle should not quietly
                // rewrite a file the user may have under version control. `EPSILON_THEME` and
                // `[tui] theme` remain the way to make a default permanent.
                let next = match arg.as_deref().map(str::trim) {
                    None | Some("") => self.theme.palette().flipped(),
                    Some(name) => match crate::ui::theme::ThemeMode::from_name(name) {
                        Some(mode) => mode,
                        None => {
                            self.ui.push_notice(format!(
                                "unknown palette '{name}' \u{2014} use `light` or `dark`, or bare `/theme` to toggle"
                            ));
                            return false;
                        }
                    },
                };
                let was = self.theme.palette();
                self.theme.set_palette(next);
                // M160: the transcript is drawn through the render cache, and a cache entry
                // carries the escape codes it was drawn with. The palette is part of `CacheKey`
                // now, so the next reconcile rebuilds; this drops the rows already drawn for
                // this frame so none of them survives it. Without both, the chrome would change
                // immediately while cached rows kept the old palette \u{2014} half a theme change,
                // which is worse than none because it looks deliberate.
                self.render_cache.clear();
                self.ui
                    .push_notice(format!("palette: {} \u{2192} {}", was.name(), next.name()));
            }
            crate::picker::Command::Compact => {
                if self
                    .compact_running
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    self.ui
                        .push_notice("compact already running — wait for it to finish");
                    return false;
                }
                if self.ui.running() {
                    self.ui.push_notice(
                        "compact queued — will run after the current turn (loop also auto-compacts on pressure)",
                    );
                } else {
                    use std::sync::atomic::Ordering;
                    self.compact_running.store(true, Ordering::Relaxed);
                    self.ui.push_notice("compacting context…");
                    let a = Arc::clone(agent);
                    let running = Arc::clone(&self.compact_running);
                    let notices = self.notices_tx.clone();
                    // **The task clears the flag when it finishes**, which is the entire
                    // reason this field is an `Arc<AtomicBool>` and not a `bool`. It was set
                    // here and never cleared, so the first `/compact` in a session was the
                    // last one: every later press answered "compact already running — wait
                    // for it to finish" for a task that finished long ago. That is the
                    // worst shape of this defect — a refusal that is plausible, so nobody
                    // suspects it, and it is permanent.
                    //
                    // The error used to go into `let _ =`. `compact_now` reports failure
                    // through a generic notice that cannot be told apart from any other,
                    // so on its own a failed compaction ends the spinner and says nothing
                    // at all about why the context did not shrink. It is named here.
                    tokio::spawn(async move {
                        let mut produced = Vec::new();
                        let outcome = a.compact_now(&mut produced).await;
                        running.store(false, Ordering::Relaxed);
                        if let Err(e) = outcome {
                            notices.send(format!("compaction failed: {e}")).ok();
                        }
                    });
                }
            }
            crate::picker::Command::Unknown(word) => {
                let hint = crate::picker::suggest_slash(&word)
                    .map(|item| format!(" — did you mean '{}'?", item.value.trim()))
                    .unwrap_or_default();
                self.ui.push_notice(format!(
                    "unknown command '/{word}'{hint} — /help lists commands"
                ));
            }
            // Window-only, refused by name (D-046/S-105): the terminal is bound to
            // its process directory, so there is no workspace to switch to here.
            // The sentence names the door rather than leaving a bare unknown —
            // and `/resume <path>` for opening another *session*.
            crate::picker::Command::Open(_) => {
                self.ui.push_notice(
                    "/open switches workspaces in the desktop window — here the workspace \
                     is this shell's directory; /resume <path> opens another session"
                        .to_string(),
                );
            }
            crate::picker::Command::Editor => {
                self.request_external_editor();
            }
            crate::picker::Command::Restore(argument) => {
                self.run_restore_file(agent, argument);
            }
            // A variant that reached the catch-all without an arm of its own.
            //
            // `run_command` sends everything it does not route by name to *this*
            // executor, so adding a `Command` variant puts it here — and this arm
            // is what it lands in. That makes the module's claim ("the compiler
            // will insist its arm exist") false as written: `#[allow]` silences
            // the warning, `debug_assert` fires only in a debug build, and a
            // **release** binary compiled with an unrouted variant returned
            // `false` in silence. A slash command that did nothing, with no
            // notice, in the build users actually run.
            //
            // So the arm stays — it has to, or this executor would have to
            // re-enumerate the 29 variants its six siblings own — and it is made
            // honest instead. A notice naming the command is the difference
            // between a keypress that visibly does nothing and one that looks
            // like a hang, and it is the TUI invariant in `AGENTS.md` applied to
            // the case the invariant did not name: not only must an unbound key
            // say so, an unrouted *command* must.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_workspace_command case");
                self.ui.push_notice(format!(
                    "/{other:?} is not implemented yet — /help lists the commands that are"
                ));
                return false;
            }
        }
        false
    }

    /// `/restore <path>` — put a file back to the bytes captured before the agent's last
    /// write or edit.
    ///
    /// ## Two presses, and the reason for the second one
    ///
    /// The first press names the file, the checkpoint and what will happen, and changes
    /// nothing. A restore overwrites bytes, `AGENTS.md` §6 requires a confirmation
    /// before an overwrite, and "one keystroke replaces a file with an older version of
    /// itself" is the shape this repository treats as a defect rather than a feature.
    /// The confirming press repeats the path and ends in `yes`, which a tab-completion
    /// or a muscle-memory Enter cannot produce — and it lands directly under the preview
    /// it confirms, in the transcript, so the reader decides against the text they just
    /// read rather than against a recollection of it.
    ///
    /// No diff is drawn, and this is a deliberate ceiling: the crate's diff renderer
    /// ([`crate::state::parse_diff`]) renders a unified or split diff of an `edit` tool
    /// result, so it needs both sides as text, and `/restore` has only the "before" side.
    /// Synthesising the other one means inventing a second diff implementation to answer
    /// a question the byte counts and the checkpoint identity already answer honestly.
    /// (`ponytail:` the ceiling is "the preview says what will change, not how"; the
    /// upgrade trigger is a request for the old-vs-new text side by side.)
    ///
    /// ## The refusal has to leave the file alone
    ///
    /// `CheckpointStore::restore` refuses when the bytes on disk are not the bytes that
    /// were captured, and this does not work around it. `epsilon_runtime::restore_checkpoint`
    /// passes that refusal through unchanged — it is the one sentence telling a reader their
    /// own edit is still there — and this pushes it verbatim behind the `restore:` marker.
    /// A reader whose later work would have been discarded is told so instead of being
    /// shown a success; a restore that silently overwrites is worse than no restore at all.
    ///
    /// ## Why it lives here, and why it names no store type
    ///
    /// This executor is the dispatcher's catch-all by design (`run_command`'s own doc: a
    /// new variant lands here so the compiler insists its arm exist), and a workspace revert
    /// belongs to none of the named groups — `/export` and `/import` move the *session*
    /// across a trust boundary, and this moves nothing but the harness's own record of its
    /// own edit.
    ///
    /// The store itself is reached through `epsilon_runtime::describe_checkpoint` and
    /// `epsilon_runtime::restore_checkpoint`, not through `epsilon-tools`: this crate's
    /// manifest states a rule worth keeping — *"the shipped TUI reaches the shell through
    /// `epsilon_core::exec::run_local`, never by naming a tool crate"* — and naming one for
    /// a single command would break it to save an import.
    ///
    /// `ponytail:` the ceiling is "one synchronous executor, guarded on the agent being
    /// idle"; the upgrade trigger is a front end that needs to restore from outside the key
    /// handler, which would want the per-path mutation queue `with_file_mutation_queue`
    /// holds and a sync handler cannot take.
    fn run_restore_file(&mut self, agent: &Arc<Agent>, argument: String) {
        let tail = argument.trim();
        if tail.is_empty() {
            self.ui.push_notice(
                "usage: /restore <path> — preview the checkpoint for a file, then \
                 `/restore <path> yes` to put it back",
            );
            return;
        }
        // A run in flight owns the workspace's files, and this is a synchronous key
        // handler: `with_file_mutation_queue` is a `tokio::Mutex`, so there is no way
        // to take the per-path gate from here. Refusing is the same guard `/import`,
        // `/resume` and `/new` use, and it is an honest answer instead of a race.
        if self.ui.running() || agent.is_running() {
            self.ui.push_notice(
                "restore: busy — wait for the current run to finish, then /restore again",
            );
            return;
        }
        // The trailing `yes` is the confirmation, and it has to be the LAST word: a file
        // called `yes` is a file, and `rsplit_once` on whitespace keeps that true,
        // because a path with no space in it never splits.
        let (path, confirmed) = match tail.rsplit_once(char::is_whitespace) {
            Some((p, word)) if word.eq_ignore_ascii_case("yes") => (p.trim(), true),
            _ => (tail, false),
        };
        // Two functions, not one with a flag: the sentence that lets a reader decide
        // whether to overwrite has to be obtainable **without** overwriting. Asking for
        // the plan must not carry the power to act on it.
        let answer = if confirmed {
            epsilon_runtime::restore_checkpoint(&self.workspace_root, path)
        } else {
            epsilon_runtime::describe_checkpoint(&self.workspace_root, path)
        };
        match answer {
            Ok(sentence) => self.ui.push_notice(format!("restore: {sentence}")),
            Err(why) => self.ui.push_notice(format!("restore: {why}")),
        }
    }

    /// `/timeline` and `/tree` are the same picker, opened by the same code.
    ///
    /// pi parity: `/branch` is an alias for the timeline view, and so is `/tree`.
    /// They were two 96-line copies differing only in the label inside three notice
    /// strings - the exact shape that lets a fix land in one copy and not the other.
    /// `label` is the only difference that survives.
    fn open_timeline_picker(&mut self, agent: &Arc<Agent>, label: &str) -> bool {
        let entries = {
            let Some(guard) = agent.session_store() else {
                self.ui
                    .push_notice(format!("{label}: no session store attached"));
                return false;
            };
            let Ok(st) = guard.lock() else {
                self.ui.push_notice(format!("{label}: store unavailable"));
                return false;
            };
            // The ACTIVE LANE, read only to mark which rows the session is
            // currently on. The rows themselves come from `tree()` below.
            match st.branch_entries() {
                Ok(v) => v,
                Err(e) => {
                    self.ui.push_notice(format!("{label} failed: {e}"));
                    return false;
                }
            }
        };
        let active: std::collections::HashSet<String> =
            entries.iter().map(|e| e.id.clone()).collect();
        // M196: the whole forest, not just the lane. `branch_entries()` walks
        // parent links from the current leaf, so a branch the user rewound away
        // from was never in this list -- which is why `/tree` could not show a
        // branch at all. `tree()` is the same structure pi's `getTree()` returns
        // and the RPC path projects, so all three agree.
        let forest = {
            let Some(guard) = agent.session_store() else {
                self.ui
                    .push_notice(format!("{label}: no session store attached"));
                return false;
            };
            let Ok(st) = guard.lock() else {
                self.ui.push_notice(format!("{label}: store unavailable"));
                return false;
            };
            match st.tree() {
                Ok(v) => v,
                Err(e) => {
                    self.ui.push_notice(format!("{label} failed: {e}"));
                    return false;
                }
            }
        };
        if forest.is_empty() {
            self.ui.push_notice(format!("{label}: session is empty"));
            return false;
        }
        let labels = match store_labels(agent) {
            Ok(l) => l,
            Err(e) => {
                self.ui.push_notice(format!("{label} failed: {e}"));
                return false;
            }
        };
        let mut all: Vec<crate::app::TimelineRow> = Vec::new();
        // The leaf the reader is on: the drop rule never hides it and the sort
        // leads with its branch. Read under the same lock discipline as the
        // forest above — a second implementation of "current" is how the picker
        // and the store disagree about where the conversation is.
        let leaf: Option<String> = {
            let Some(guard) = agent.session_store() else {
                self.ui
                    .push_notice(format!("{label}: no session store attached"));
                return false;
            };
            let Ok(st) = guard.lock() else {
                self.ui.push_notice(format!("{label}: store unavailable"));
                return false;
            };
            st.leaf_id()
        };
        flatten_tree(&forest, 0, &active, &labels, leaf.as_deref(), &mut all);
        // M196: the filter is applied here, once, on the flattened rows -- so the
        // picker, the key handler and the renderer all see the same filtered
        // list rather than each applying the rule and hoping they agree.
        // The same pipeline `refilter` uses, so the picker and the chords cannot
        // disagree about what is visible.
        let rows = visible_rows(
            &all,
            self.tree_filter,
            &labels,
            &self.tree_query,
            &self.folded,
        );
        if rows.is_empty() {
            self.ui.push_notice(format!("{label}: session is empty"));
            return false;
        }
        self.timeline_all = all;
        self.timeline_entries = rows;
        // Land on the leaf, which is where the conversation is -- not on the last
        // row of the forest, which after a rewind is a branch the user left.
        self.timeline_index = self
            .timeline_entries
            .iter()
            .rposition(|r| r.on_active)
            .unwrap_or(self.timeline_entries.len() - 1);
        self.timeline_open = true;
        false
    }

    /// Commands that show the user something and change nothing: `/help`, `/skills`,
    /// `/tasks`, `/timeline`, `/hotkeys`, `/settings`, `/changelog`.
    fn run_view_command(&mut self, agent: &Arc<Agent>, command: crate::picker::Command) -> bool {
        match command {
            crate::picker::Command::Help => {
                for item in crate::picker::SLASH_ITEMS {
                    self.ui.push_notice(format!(
                        "{} — {}",
                        item.value.trim_end(),
                        item.description
                    ));
                }
                let mut guide: Vec<&str> = vec![
                    "auth: /login anthropic sk-...  (paste inside TUI, masked) · /logout anthropic · keys saved to ~/.config/epsilon/auth.json",
                    "subscription login: /login anthropic oauth · /login xai oauth (device code) · /login openrouter oauth — browser round-trip, same store",
                    "also: /connect /auth are aliases for /login — same masked prompt, paste works inside it",
                    "thinking: Ctrl+T cycles · current level in the status bar",
                ];
                // The header is the harness's own report on itself, and a report with
                // no legend is only transparent to the person who wrote it.
                //
                // The legend lives in `ui::metrics` rather than here, and that is the
                // fix for this row: the three lines that used to sit here described
                // `42%/200k` and `cache 87%`, neither of which the header has printed
                // since the `ctx` label and the cache volume landed. The three tokens a
                // reader actually cannot decode — `ctx ?`, `CH n/a`, and the trailing
                // `offline` — appeared in none of them. A legend kept beside the
                // formatters is held to them by a test; a legend kept beside the help
                // text is held to nothing.
                guide.extend(crate::ui::metrics::LEGEND.iter().copied());
                guide.extend([
                    "goal: /goal <objective> parks one long-running objective in this session; the header line shows its phase and rounds",
                    "      /goal shows it · edit · pause · resume · complete · blocked <code> <reason> · clear · the model has the same controls as tools",
                    "keys: Enter submit · Esc aborts a run (quits when idle) · Ctrl+X expands tool output",
                    "      PgUp/PgDn scroll transcript · Ctrl+M model picker · Ctrl+P command palette",
                ]);
                for line in guide {
                    self.ui.push_notice(line);
                }
            }
            crate::picker::Command::Skills => {
                // M196: this printed `names.join(", ")`, discarding a description
                // the `Skill` struct already carries. The repo had solved this
                // exact problem for `/templates` one command away (`/name —
                // description`), so a user could read what a template does and
                // not what a skill does.
                let skills = agent.skills();
                if skills.is_empty() {
                    self.ui.push_notice(
                        "no skills discovered — drop SKILL.md dirs into .epsilon/skills/",
                    );
                } else {
                    let disabled = epsilon_core::config::load_disabled_skills();
                    for line in skill_lines(&skills, &disabled) {
                        self.ui.push_notice(line);
                    }
                    let on = skills.len() - disabled.len();
                    self.ui.push_notice(format!(
                        "{on} of {} enabled · toggle one with `/skill <name> off|on`",
                        skills.len()
                    ));
                }
            }
            crate::picker::Command::Templates => {
                // Descriptions, not just names. A template's whole purpose is that you can
                // forget its body but not want to forget what it does, and a bare list of
                // names is the thing `/skills` would print. A user who cannot remember
                // whether they wrote `/commit` or `/commit-msg` has no way to find out
                // otherwise.
                if self.prompt_templates.is_empty() {
                    self.ui.push_notice(
                        "no prompt templates — drop .md files into .epsilon/prompts/ or .agents/prompts/",
                    );
                } else {
                    let mut out: Vec<String> = Vec::new();
                    for t in &self.prompt_templates {
                        let mut line = format!("/{} — {}", t.name, t.description);
                        if let Some(hint) = &t.argument_hint {
                            line.push_str(&format!(" (takes {hint})"));
                        }
                        out.push(line);
                    }
                    self.ui
                        .push_notice(format!("prompt templates: {}", out.join(" · ")));
                }
            }
            crate::picker::Command::Tasks => {
                // M54: the registry is the only place a delegated child's state
                // is visible; Esc (abort) now reaches running children too, so
                // this doubles as the "what am I still waiting on" view.
                let snapshots = agent.task_snapshots();
                if snapshots.is_empty() {
                    self.ui.push_notice(
                        "no subagent tasks — the model starts them with the `task` tool (background=true runs detached and reports back here)",
                    );
                } else {
                    let mut running = 0usize;
                    for s in &snapshots {
                        let state = match s.state {
                            epsilon_contracts::tool::TaskState::Running => "running",
                            epsilon_contracts::tool::TaskState::Completed => "completed",
                            epsilon_contracts::tool::TaskState::Failed => "failed",
                        };
                        if s.state == epsilon_contracts::tool::TaskState::Running {
                            running += 1;
                        }
                        let head = s.output.lines().next().unwrap_or("").trim();
                        let preview = crate::wrap::fit_width(head, 120);
                        // M56: name the model the child runs on, so a scout on
                        // a cheap model is distinguishable at a glance.
                        let on_model = s
                            .model
                            .as_deref()
                            .map(|m| format!(" · @{m}"))
                            .unwrap_or_default();
                        // M229: the child's own tokens, in the row that already names its model.
                        // They are counted in the turn total and the cost panel now; printing
                        // them here is what makes "why is this session expensive" answerable
                        // without adding up turns by hand.
                        let spend = s
                            .usage
                            .map(|u| {
                                format!(
                                    " · {} in / {} out",
                                    crate::ui::format::format_tokens(u.input),
                                    crate::ui::format::format_tokens(u.output)
                                )
                            })
                            .unwrap_or_default();
                        self.ui.push_notice(format!(
                            "task {} · {state}{on_model}{spend} · {} · {} turn(s) · {}",
                            s.id,
                            s.description,
                            s.turns,
                            if preview.is_empty() {
                                "(no output yet)"
                            } else {
                                preview.as_str()
                            }
                        ));
                    }
                    self.ui.push_notice(format!(
                        "{} task(s) · {running} running — resume one with the task tool (task_id) or Esc a run to stop them",
                        snapshots.len()
                    ));
                }
            }
            crate::picker::Command::Timeline => {
                let _ = self.open_timeline_picker(agent, "timeline");
            }
            crate::picker::Command::Hotkeys => {
                for line in [
                    "hotkeys: Enter submit · Esc aborts run (quits when idle) · Ctrl+X expands tool output",
                    "         PgUp/PgDn scroll · Ctrl+M model picker · Ctrl+P command palette · Ctrl+T think",
                    "         Ctrl+U kill to start · Ctrl+W kill word · Ctrl+Y yank · Ctrl+F favorite",
                    "         /help lists slash commands · /models switches model",
                ] {
                    self.ui.push_notice(line);
                }
                // G6 polish: prove the remap in the transcript — effective
                // bindings from ~/.config/epsilon/keybindings.json (sorted).
                let mut eff: Vec<(String, Vec<String>)> =
                    self.keybindings.effective().into_iter().collect();
                eff.sort_by(|a, b| a.0.cmp(&b.0));
                if !eff.is_empty() {
                    self.ui
                        .push_notice("bindings (effective — ~/.config/epsilon/keybindings.json):");
                    for (id, keys) in eff {
                        self.ui.push_notice(format!("  {id}: {}", keys.join(" | ")));
                    }
                }
            }
            crate::picker::Command::Settings => {
                // Ponytail: no overlay form at v1 (pi SettingsSelectorComponent is ~30 toggles).
                // Instead show actionable guidance: what exists + where it lives + how to change it.
                let global = epsilon_core::config::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "~/.config/epsilon/config.toml".to_string());
                let project = self.workspace_root.join(".epsilon").join("config.toml");
                self.ui.push_notice(format!(
                    "settings: global {global} · project {}",
                    project.display()
                ));
                self.ui.push_notice(format!(
                    "  current model: {} · think: {}",
                    self.model_id,
                    self.thinking.as_str()
                ));
                let rec = if self.recents.is_empty() {
                    "(none yet)".to_string()
                } else {
                    self.recents[..self.recents.len().min(5)].join(", ")
                };
                let fav = if self.favorites.is_empty() {
                    "(none — Ctrl+F to star)".to_string()
                } else {
                    self.favorites[..self.favorites.len().min(5)].join(", ")
                };
                self.ui.push_notice(format!("  recents: {rec}"));
                self.ui.push_notice(format!("  favorites: {fav}"));
                self.ui
                    .push_notice("edit the toml directly (model: [model] id, favorites: [picker] favorite), then /reload to apply without restart");
            }
            crate::picker::Command::Changelog => {
                // Ponytail: read workspace CHANGELOG.md (pi parseChangelog parity, first section)
                // and emit it into the transcript — no new dep, reuses read_to_string.
                let candidates = [
                    self.workspace_root.join("CHANGELOG.md"),
                    self.workspace_root.join("docs/CHANGELOG.md"),
                ];
                let mut found: Option<(std::path::PathBuf, String)> = None;
                for c in candidates {
                    if let Ok(s) = std::fs::read_to_string(&c)
                        && !s.trim().is_empty()
                    {
                        found = Some((c, s));
                        break;
                    }
                }
                if let Some((path, raw)) = found {
                    // Emit header + first ~120 lines (header + Unreleased + last release) to keep
                    // the transcript bounded; the full file stays on disk at `path`.
                    let lines: Vec<&str> = raw.lines().collect();
                    let take = lines.len().min(120);
                    self.ui.push_notice(format!(
                        "changelog: {} ({} lines, showing {} — see file for full history)",
                        path.display(),
                        lines.len(),
                        take
                    ));
                    for line in &lines[..take] {
                        self.ui.push_notice((*line).to_string());
                    }
                    if lines.len() > take {
                        self.ui.push_notice(format!(
                            "… {} more lines in {}",
                            lines.len() - take,
                            path.display()
                        ));
                    }
                } else {
                    self.ui
                        .push_notice("changelog: no CHANGELOG.md in workspace (see docs/milestones/ for milestone logs)");
                }
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_view_command case");
                return false;
            }
        }
        false
    }

    /// Commands that move bytes or permissions across the trust boundary: `/trust`,
    /// `/export`, `/import`, `/share`, `/copy`.
    fn run_transfer_command(
        &mut self,
        agent: &Arc<Agent>,
        command: crate::picker::Command,
    ) -> bool {
        match command {
            crate::picker::Command::Trust(arg) => {
                let a = arg.as_deref().unwrap_or("").trim().to_ascii_lowercase();
                if a.is_empty() {
                    match epsilon_core::config::current_project_config_hash_in(
                        &self.workspace_root,
                    ) {
                        None => self.ui.push_notice(
                            "trust: no project config (.epsilon/config.toml not found) — nothing to trust",
                        ),
                        Some(hash) => {
                            let trust_path =
                                self.workspace_root.join(".epsilon").join("trust.json");
                            match std::fs::read_to_string(&trust_path) {
                                Ok(raw) => {
                                    let prev = serde_json::from_str::<serde_json::Value>(&raw)
                                        .ok()
                                        .and_then(|v| {
                                            v.get("config_sha256")
                                                .and_then(|h| h.as_str())
                                                .map(|s| s.to_string())
                                        });
                                    match prev {
                                        Some(prev) if prev == hash => self.ui.push_notice(
                                            format!("trust: config is trusted (hash {}…)", &hash[..8.min(hash.len())]),
                                        ),
                                        Some(prev) => self.ui.push_notice(format!(
                                            "trust: config changed (prev {}… current {}…) — /trust yes to accept, /trust no to keep untrusted",
                                            &prev[..8.min(prev.len())],
                                            &hash[..8.min(hash.len())]
                                        )),
                                        None => self.ui.push_notice(
                                            "trust: trust file unreadable — /trust yes to accept",
                                        ),
                                    }
                                }
                                Err(_) => self.ui.push_notice(format!(
                                    "trust: config present (hash {}…) but not yet trusted — /trust yes to accept",
                                    &hash[..8.min(hash.len())]
                                )),
                            }
                        }
                    }
                    return false;
                }
                if a == "yes" || a == "y" {
                    if self.trust_review_pending {
                        self.trust_review_pending = false;
                    }
                    let ws = self.workspace_root.clone();
                    // M45: same dual-memory accept as the review gate above.
                    if let Some(store) = crate::trust_store::ProjectTrustStore::open() {
                        match store.set(&ws, Some(true)) {
                            Ok(()) => self.ui.push_notice(
                                "project path trusted (remembered for future runs)".to_string(),
                            ),
                            Err(e) => self.ui.push_notice(format!("trust store note: {e}")),
                        }
                    }
                    match epsilon_core::config::current_project_config_hash_in(&ws) {
                        Some(hash) => {
                            match epsilon_core::config::accept_project_config_hash_in(&ws, &hash) {
                                Ok(path) => self.ui.push_notice(format!(
                                    "trust accepted — hash {}… saved to {}",
                                    &hash[..8.min(hash.len())],
                                    path.display()
                                )),
                                Err(e) => self.ui.push_error(format!("trust accept failed: {e}")),
                            }
                        }
                        None => self.ui.push_notice("trust: no config file to accept"),
                    }
                } else if a == "no" || a == "n" {
                    self.ui
                        .push_notice("trust: not accepted — config stays untrusted (edits still require /trust yes)");
                } else {
                    self.ui.push_notice("usage: /trust [yes|no]");
                }
            }
            crate::picker::Command::Export(arg) => {
                let tail = arg.as_deref().unwrap_or("");
                // Bare `/export` defaults to html into .epsilon/exports/ (pi parity:
                // `/export` without args writes the HTML export automatically).
                // The spelling is shared (`split_export_arg`) so both front ends
                // mean the same thing by it.
                let (fmt, raw_path) = epsilon_contracts::command::split_export_arg(tail);
                let dest: std::path::PathBuf = if raw_path.is_empty() {
                    // default: <workspace>/.epsilon/exports/session-<id>.{html,json}
                    let id = epsilon_contracts::new_id("export");
                    let ext = if fmt == "html" { "html" } else { "json" };
                    self.workspace_root
                        .join(".epsilon")
                        .join("exports")
                        .join(format!("session-{id}.{ext}"))
                } else {
                    let p = std::path::Path::new(&raw_path);
                    if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        self.workspace_root.join(p)
                    }
                };
                if fmt == "html" {
                    let result = (|| -> Result<usize, String> {
                        let store = agent.session_store().ok_or("no session store attached")?;
                        let guard = store.lock().map_err(|_| "store unavailable")?;
                        epsilon_session::export::write_html(&guard, &dest)
                    })();
                    match result {
                        Ok(n) => self.ui.push_notice(format!(
                            "exported html → {} ({} entries)",
                            dest.display(),
                            n
                        )),
                        Err(e) => self.ui.push_error(format!("export failed: {e}")),
                    }
                } else {
                    let result = (|| -> Result<usize, String> {
                        let store = agent.session_store().ok_or("no session store attached")?;
                        let guard = store.lock().map_err(|_| "store unavailable")?;
                        epsilon_session::export::write_json(&guard, &dest)
                    })();
                    match result {
                        Ok(n) => self.ui.push_notice(format!(
                            "exported json → {} ({} entries)",
                            dest.display(),
                            n
                        )),
                        Err(e) => self.ui.push_error(format!("export failed: {e}")),
                    }
                }
            }
            crate::picker::Command::Import(path) => {
                let p = path.trim();
                if p.is_empty() {
                    self.ui.push_notice(
                        "usage: /import <path> — switches session live, no restart needed (same as /resume)",
                    );
                } else if self.ui.running() || agent.is_running() {
                    self.ui.push_notice(
                        "import: busy — wait for the current run to finish, then /import again",
                    );
                } else {
                    let raw = std::path::PathBuf::from(p);
                    let abs = if raw.is_absolute() {
                        raw
                    } else {
                        self.workspace_root.join(raw)
                    };
                    if !abs.exists() {
                        self.ui.push_notice(format!(
                            "import: no file at '{}' — check the path",
                            abs.display()
                        ));
                    } else {
                        // Ask first (P-079): a swap drops the current
                        // context from view. Answered in `submit_editor`,
                        // next to the trust gate.
                        self.pending_import = Some((abs.clone(), p.to_string()));
                        self.ui.push_notice(format!(
                            "Replace current session with '{}'? (yes/no)",
                            abs.display()
                        ));
                    }
                }
            }
            crate::picker::Command::Share => {
                // Pi `handleShareCommand` parity (ponytail: reuse write_export_html,
                // try `gh gist create --public=false` when present, honest guidance otherwise).
                let share_dest = self
                    .workspace_root
                    .join(".epsilon")
                    .join("exports")
                    .join(format!("share-{}.html", epsilon_contracts::new_id("share")));
                match (|| -> Result<usize, String> {
                    let store = agent.session_store().ok_or("no session store attached")?;
                    let guard = store.lock().map_err(|_| "store unavailable")?;
                    epsilon_session::export::write_html(&guard, &share_dest)
                })() {
                    Ok(n) => {
                        let gh_attempt = std::process::Command::new("gh")
                            .args(["gist", "create", "--public=false"])
                            .arg(&share_dest)
                            .output();
                        match gh_attempt {
                            Ok(out) if out.status.success() => {
                                let gist_url =
                                    String::from_utf8_lossy(&out.stdout).trim().to_string();
                                let gist_id = gist_url.rsplit('/').next().unwrap_or("").to_string();
                                let preview = std::env::var("PI_SHARE_VIEWER_URL")
                                    .unwrap_or_else(|_| "https://pi.dev/session/".to_string())
                                    + &gist_id;
                                if gist_id.is_empty() {
                                    self.ui.push_notice(format!(
                                        "share: exported html → {} ({} entries) · gist: {} (preview parse failed — copy the URL above)",
                                        share_dest.display(),
                                        n,
                                        gist_url
                                    ));
                                } else {
                                    self.ui.push_notice(format!(
                                        "share: gist {} · preview {} · file {} ({} entries)",
                                        gist_url,
                                        preview,
                                        share_dest.display(),
                                        n
                                    ));
                                }
                            }
                            Ok(out) => {
                                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                let hint = if err.is_empty() {
                                    "unknown error".to_string()
                                } else {
                                    err
                                };
                                self.ui.push_notice(format!(
                                    "share: exported html → {} ({} entries) · gist not created ({}).\n       Publish the file or run: gh gist create --public=false {}",
                                    share_dest.display(),
                                    n,
                                    hint,
                                    share_dest.display()
                                ));
                            }
                            Err(e) => {
                                let hint = if e.kind() == std::io::ErrorKind::NotFound {
                                    "gh not installed — https://cli.github.com/ (then `gh auth login`)".to_string()
                                } else {
                                    format!("{e}")
                                };
                                self.ui.push_notice(format!(
                                    "share: exported html → {} ({} entries) · gist deferred ({}).\n       When gh is available: gh gist create --public=false {}",
                                    share_dest.display(),
                                    n,
                                    hint,
                                    share_dest.display()
                                ));
                            }
                        }
                    }
                    Err(e) => self.ui.push_error(format!("share failed: {e}")),
                }
            }
            crate::picker::Command::Copy => {
                // Copy the last assistant message to the clipboard (pi
                // app.message.copy parity). Two paths, and the notice names the
                // one that landed: `arboard` reaches this machine's clipboard,
                // and OSC 52 asks the *terminal* to reach the one the user is
                // sitting at — the only path that works over SSH, where arboard
                // either fails or fills a clipboard nobody will paste from.
                let text = self
                    .ui
                    .lines()
                    .iter()
                    .rev()
                    .find(|l| l.kind == crate::state::LineKind::Assistant)
                    .map(|l| l.full.as_deref().unwrap_or(l.text.as_str()).to_string());
                match text {
                    Some(text) => {
                        let chars = text.chars().count();
                        // The sequence is handed to the *terminal*, so it is
                        // parked for the frame boundary rather than written from
                        // a key handler; `drive` writes it through the backend's
                        // own writer once the frame is out. A local rather than a
                        // `self`-capturing closure: the borrow checker is right
                        // that a closure holding `&mut self` across the call
                        // conflicts with the notice pushed after it.
                        let mut osc = None;
                        let copied = crate::clipboard::copy(
                            &text,
                            crate::clipboard::Multiplexer::detect(),
                            crate::clipboard::is_remote(),
                            |sequence| {
                                osc = Some(sequence.to_string());
                                Ok(())
                            },
                        );
                        self.osc_pending = osc;
                        self.ui
                            .push_notice(crate::clipboard::notice(chars, &copied));
                    }
                    None => self.ui.push_notice("copy: no assistant message to copy"),
                }
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_transfer_command case");
                return false;
            }
        }
        false
    }
}

/// Frame-level coverage for `/restore`.
///
/// Every assertion here is against **the drawn frame**, through the production draw path
/// (`render_rows` calls the same `draw` the event loop calls), and not against
/// `app.ui.lines()`. A restore that printed a sentence nobody can see is the same defect
/// this whole change exists to close — a rollback nobody can reach — wearing a different
/// hat. The state assertions that matter (the file is untouched, the file is gone) are
/// alongside, because they are about bytes and the frame is about words.
#[cfg(test)]
mod tests {
    use super::super::keys::handle_key;
    use super::super::tests_support::{render_rows, test_app};
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    /// Type `text` one key at a time and press Enter — the real key path.
    ///
    /// Written out rather than imported: `app::tests` has an identical helper, but it is
    /// private to that module, and duplicating six lines beats making a private test
    /// helper part of the crate's surface.
    fn type_and_enter(app: &mut App, agent: &Arc<Agent>, text: &str) -> bool {
        for c in text.chars() {
            handle_key(app, agent, key(KeyCode::Char(c)));
        }
        handle_key(app, agent, key(KeyCode::Enter))
    }

    /// The whole screen as one string a reader's eye could follow.
    ///
    /// A notice wraps to the terminal's width and every transcript row carries a gutter
    /// glyph, so a test that looks for a sentence otherwise looks for the column it
    /// happened to wrap at — and a needle that matches at 100×30 misses at 80×24. Each
    /// row's leading gutter goes and the whitespace is squashed, which leaves the
    /// assertion about the words rather than about the renderer's arithmetic.
    ///
    /// Only the **leading** gutter goes. `·` is left alone because it separates fields
    /// inside a row (`idle?/1.0M·↑0↓0`), and stripping it everywhere would invent word
    /// boundaries that are not on screen.
    fn frame(app: &mut App, width: u16, height: u16) -> String {
        render_rows(app, width, height)
            .iter()
            // `•` is the unicode repertoire's System-row gutter — the same leading gutter
            // `*` is in the ASCII one. Stripping `*` and not `•` made every wrapped-notice
            // assertion repertoire-dependent: under ASCII the continuation's bullet went and
            // `contains` saw the sentence; under unicode it stayed and the squeezed screen
            // read `wrote•it` where the test looked for `wroteit`. Round 105's battery found
            // this when the harness's own `TERM=xterm-256color` flipped the suite from ASCII
            // to unicode with no source change — the tests had been green against the wrong
            // repertoire. `·` (the in-row field separator) stays: stripping it would invent
            // word boundaries that are not on screen.
            .map(|row| row.trim_start_matches(|c: char| c.is_whitespace() || "*•│┃".contains(c)))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    /// A workspace whose ledger holds one entry for `path`, recorded exactly the way
    /// `files.rs` records one around a real write.
    ///
    /// **Two steps, and the second one is not optional.** The tool records the pre-image,
    /// writes, and *then* calls `amend_with_post_image` so the entry also carries what the
    /// harness left behind. A fixture that stops after `record` produces a capture the
    /// product no longer creates, and the restore guard answers it with its coarser
    /// refusal — "this checkpoint does not record what the harness left behind" — which is
    /// true of the fixture and false of the product. That is exactly the sort of thing a
    /// fixture is supposed to prevent rather than introduce.
    fn checkpointed(root: &std::path::Path, path: &str, content: &str) -> std::path::PathBuf {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().expect("the file has a directory")).unwrap();
        std::fs::write(&file, content).unwrap();
        let store = epsilon_tools::checkpoint::CheckpointStore::open(root);
        // Pre-image: the state a revert returns to. Captured before the tool moves bytes,
        // which is why `files.rs` records first and writes second.
        store.record(root, &file, "write");
        // Post-image: what the harness then left on disk. The restore guard asks whether
        // the file still matches *this*, so without it an untouched edit is
        // indistinguishable from a user-edited one.
        store.amend_with_post_image(root, &file, "write");
        file
    }

    /// The same, for a file the **agent created**: `record` runs first, while the file
    /// does not exist yet, which is the whole difference between the two entries.
    ///
    /// The order is the point and it is easy to get backwards in a fixture. `record`
    /// captures what is there *before* the tool writes, so a helper that writes first
    /// and records after produces an entry that says the file already existed — and the
    /// preview it earns ("would put back N bytes") is then a sentence about an edit
    /// rather than a creation. The first version of this test did exactly that and the
    /// frame caught it.
    ///
    /// **The amendment is here, and an earlier version of this comment said it must not be.**
    ///
    /// It said a creation entry has `blob: None`, "so there is nothing a post-image could
    /// amend" — which was true of the *pre-image* and false of the point. A creation has no
    /// bytes to capture because there was no file to capture them from, but the write that
    /// follows leaves one, and that is exactly what the deletion guard compares against. With
    /// the amendment omitted this fixture built an entry shape no real `write` produces, and
    /// the frame test failed on a correctly-guarded store.
    ///
    /// This is the second time this file's fixtures have lied about the product in the same
    /// way; the first is recorded in M277c. The pattern is worth naming: a fixture is a
    /// claim about what the product writes, and the day the product changes, the fixture is
    /// the thing that is wrong — not the guard it was written to avoid.
    fn created_by_the_agent(
        root: &std::path::Path,
        path: &str,
        content: &str,
    ) -> std::path::PathBuf {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().expect("the file has a directory")).unwrap();
        let store = epsilon_tools::checkpoint::CheckpointStore::open(root);
        store.record(root, &file, "write");
        std::fs::write(&file, content).unwrap();
        // What `files.rs` does once the bytes have landed, on every write and every edit.
        store.amend_with_post_image(root, &file, "write");
        file
    }

    /// A restore that succeeds, asserted on the frame and on the bytes.
    ///
    /// The shape asserted is the one a rollback exists for in this build: the agent
    /// **created** a file, so reverting means removing it. The file did not exist when the
    /// entry was taken, so there is no pre-image to compare — which is why the frame has to
    /// say `DELETE` in capitals *before* the reader is asked to confirm, and why the file is
    /// still there after the first press.
    ///
    /// The post-image is still what makes the deletion safe (see `created_by_the_agent`):
    /// "no pre-image" is a reason to warn loudly, not a reason to act unguarded.
    ///
    /// Both terminal sizes are checked because the preview is a long sentence and a wide
    /// frame wraps it differently: a preview that reads at 100×30 and is cut at 80×24 is
    /// a preview half the people using this will not see.
    #[tokio::test]
    async fn a_confirmed_restore_reverts_the_file_and_the_frame_says_so() {
        let (mut app, agent, (_f, _n)) = test_app();
        let dir = tempfile::tempdir().expect("a temporary workspace");
        app.workspace_root = dir.path().to_path_buf();
        let created = created_by_the_agent(dir.path(), "src/new.rs", "// written by the model\n");

        // First press: the plan. Nothing on disk may change here.
        assert!(!type_and_enter(&mut app, &agent, "/restore src/new.rs"));
        for (w, h) in [(80u16, 24u16), (100u16, 30u16)] {
            let screen = frame(&mut app, w, h);
            assert!(
                screen.contains("nothinghaschangedyet"),
                "the frame must say nothing has changed yet, at {w}x{h}: {screen}"
            );
            assert!(
                screen.contains("DELETE"),
                "the irreversible outcome is named in capitals at {w}x{h}: {screen}"
            );
            assert!(
                screen.contains("/restoresrc/new.rsyes"),
                "and the exact keys that apply it, at {w}x{h}: {screen}"
            );
        }
        assert!(
            created.exists(),
            "the preview must not have touched the file"
        );

        // Second press: the confirmation, and only now the change.
        assert!(!type_and_enter(&mut app, &agent, "/restore src/new.rs yes"));
        assert!(
            !created.exists(),
            "the confirmed revert removes the file the agent created"
        );
        let screen = frame(&mut app, 80, 24);
        assert!(
            screen.contains("restored0bytestosrc/new.rs"),
            "the frame reports what happened: {screen}"
        );
    }

    /// A restore refused because the file moved says so, and the file survives it.
    ///
    /// This is the security rule `AGENTS.md` §6 names and M104 guards: reverting over a
    /// file somebody edited since the capture destroys their work. The refusal sentence is
    /// the only thing telling a reader their edit is still there, so it is asserted on the
    /// frame *and* the file's contents are asserted afterwards — a refusal that printed
    /// the right words after having already overwritten would pass the first test alone.
    ///
    /// **The wording moved, and the property did not.** This asserted `changed since it was
    /// captured`, which was true of M104's original guard and stopped being true once the
    /// entry also carried the harness's own post-image (ADR-023): a capture now knows which
    /// moment the file stopped matching, so it can say "changed after this harness wrote
    /// it" instead of the indistinguishable "changed since it was captured". The property
    /// being pinned is that a moved file refuses and survives — the sentence is only how a
    /// reader learns it.
    #[tokio::test]
    async fn a_restore_refused_because_the_file_changed_leaves_it_and_says_why() {
        let (mut app, agent, (_f, _n)) = test_app();
        let dir = tempfile::tempdir().expect("a temporary workspace");
        app.workspace_root = dir.path().to_path_buf();
        let file = checkpointed(dir.path(), "src/lib.rs", "as the agent saw it\n");
        std::fs::write(&file, "the user's later work\n").unwrap();

        assert!(!type_and_enter(&mut app, &agent, "/restore src/lib.rs yes"));
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "the user's later work\n",
            "a refusal must leave the file exactly as it was"
        );
        let screen = frame(&mut app, 80, 24);
        assert!(
            screen.contains("changedafterthisharnesswroteit"),
            "the frame says why it refused: {screen}"
        );
        assert!(
            screen.contains("src/lib.rs"),
            "and names the file it would have overwritten: {screen}"
        );
    }

    /// The same guard on the **deletion** path, asserted on the frame — the half ADR-026 closed.
    ///
    /// The refusal above covers an edited file whose pre-image is put back. This covers reverting a
    /// **creation**, where the outcome is a deletion and the user's later edit is the whole file.
    /// It is asserted on the frame for the same reason: the sentence is the only thing that tells a
    /// reader their work is still there, and a store that printed it *after* deleting would pass a
    /// test that only checked the words.
    #[tokio::test]
    async fn a_created_file_the_user_edited_is_refused_and_the_frame_says_why() {
        let (mut app, agent, (_f, _n)) = test_app();
        let dir = tempfile::tempdir().expect("a temporary workspace");
        app.workspace_root = dir.path().to_path_buf();
        let created = created_by_the_agent(dir.path(), "src/new.rs", "// written by the model\n");
        std::fs::write(&created, "// mine, and not yours\n").unwrap();

        assert!(!type_and_enter(&mut app, &agent, "/restore src/new.rs yes"));
        assert_eq!(
            std::fs::read_to_string(&created).unwrap(),
            "// mine, and not yours\n",
            "a refused deletion must leave the user's own file exactly as it was"
        );
        let screen = frame(&mut app, 80, 24);
        assert!(
            screen.contains("changedafterthisharnesswroteit"),
            "the frame says why it refused: {screen}"
        );
        assert!(
            screen.contains("src/new.rs"),
            "and names the file it would have deleted: {screen}"
        );
    }

    /// An argument that names nothing says so — it does not print a success.
    ///
    /// The control for the two tests above: with the same store, the same frame and the
    /// same command, a name the ledger does not hold must produce a refusal that names
    /// what was asked for and what *is* recoverable. Without it, "restored 0 bytes" would
    /// pass a test that only ever saw a refusal on the happy path.
    #[tokio::test]
    async fn a_restore_naming_nothing_says_so_on_the_frame() {
        let (mut app, agent, (_f, _n)) = test_app();
        let dir = tempfile::tempdir().expect("a temporary workspace");
        app.workspace_root = dir.path().to_path_buf();
        let file = checkpointed(dir.path(), "src/lib.rs", "as the agent saw it\n");

        assert!(!type_and_enter(
            &mut app,
            &agent,
            "/restore src/parser.rs yes"
        ));
        assert!(
            file.exists(),
            "an argument that names nothing must not touch anything"
        );
        let screen = frame(&mut app, 80, 24);
        assert!(
            screen.contains("nocheckpointfor'src/parser.rs'"),
            "the frame quotes what was asked for: {screen}"
        );
        assert!(
            screen.contains("src/lib.rs"),
            "and names the file that does have a checkpoint: {screen}"
        );
        assert!(
            !screen.contains("restored"),
            "and it must not claim a restore happened: {screen}"
        );
    }

    /// A bare `/restore` prints its grammar and changes nothing.
    ///
    /// The repo's own rule for an argument-taking command: say what the shape is rather
    /// than assuming one. Assuming here would mean restoring *some* file, which is the
    /// silent-no-op shape in its worst form.
    #[tokio::test]
    async fn a_bare_restore_prints_usage_and_changes_nothing() {
        let (mut app, agent, (_f, _n)) = test_app();
        let dir = tempfile::tempdir().expect("a temporary workspace");
        app.workspace_root = dir.path().to_path_buf();
        let file = checkpointed(dir.path(), "src/lib.rs", "as the agent saw it\n");
        let before = std::fs::read(&file).unwrap();

        assert!(!type_and_enter(&mut app, &agent, "/restore"));
        assert_eq!(std::fs::read(&file).unwrap(), before);
        let screen = frame(&mut app, 80, 24);
        assert!(
            screen.contains("usage:/restore<path>"),
            "the frame carries the usage line: {screen}"
        );
    }

    /// `frame` drops every whitespace character, so a needle built from a path that happens
    /// to contain a space could never match. Same rule as [`frame`], applied once here so
    /// the assertions below are about the words and not about the renderer.
    fn squeezed(text: &str) -> String {
        text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Write one well-formed `SKILL.md` under a root — the shape the rules accept.
    fn write_skill(root: &std::path::Path, dir: &str, description: &str) {
        let d = root.join(dir);
        std::fs::create_dir_all(&d).expect("the skill directory");
        std::fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {dir}\ndescription: {description}\n---\nbody of {dir}\n"),
        )
        .expect("the SKILL.md");
    }

    /// **The defect: `/reload` reported a count and never said why it was short.**
    ///
    /// `epsilon_core::skills::discover` threw the diagnostics away, so a skills root that
    /// could not be read produced "reload: 0 skill(s) loaded" and a user who had typed a
    /// skill had nothing to act on. `discover_with_notices` names the root and the reason,
    /// and this asserts that line reaches **the frame** — not a field on `App`.
    ///
    /// The root is made unreadable by **being a file**, which is the portable way to make
    /// `read_dir` fail for a reason other than absence: a permission error needs an ACL and
    /// a dangling link needs a privilege, and a test that needs either is a test that
    /// quietly asserts nothing on the machine that happens to permit it. The same trick is
    /// what `crates/epsilon-core/src/skills.rs` uses for its own version of this.
    ///
    /// The count assertion is the second half of the sentence: the skills that *could* be
    /// read still load, so adding the diagnostics did not turn a reload into a refusal.
    #[tokio::test]
    async fn a_skill_root_reload_cannot_read_is_named_on_the_frame() {
        let (mut app, agent, (_f, _n)) = test_app();
        let dir = tempfile::tempdir().expect("a temporary workspace");
        app.workspace_root = dir.path().to_path_buf();

        // The good root, so "0 skills" cannot be mistaken for "discovery did nothing".
        write_skill(
            &dir.path().join(".epsilon").join("skills"),
            "commit-craft",
            "Craft commits.",
        );
        // The bad one: `.agents/skills` is a regular file, so `read_dir` refuses it.
        let agents_root = dir.path().join(".agents");
        std::fs::create_dir_all(&agents_root).expect("the .agents directory");
        let unreadable = agents_root.join("skills");
        std::fs::write(&unreadable, "a file where a skills root should be\n").unwrap();

        assert!(!type_and_enter(&mut app, &agent, "/reload"));

        // The whole sentence up to the reason, so a notice that named the root but no reason
        // does not satisfy this.
        let lead = squeezed(&format!(
            "skills: cannot read root {}: ",
            unreadable.display()
        ));
        for (w, h) in [(80u16, 24u16), (100u16, 30u16)] {
            let screen = frame(&mut app, w, h);
            assert!(
                screen.contains(&lead),
                "the frame must name the root it could not read AND leave room for the \
                 reason, at {w}x{h}: {screen}"
            );
        }

        // The reason. `/reload` always ends with its own summary line, so whatever sits
        // between the lead and that word is what discovery said caused the failure.
        let screen = frame(&mut app, 100, 30);
        let after = &screen[screen.find(&lead).expect("asserted above") + lead.len()..];
        let reason = after.split("reload:").next().unwrap_or_default();
        assert!(
            reason.len() >= 3,
            "a notice naming a root and no reason is the defect again: {screen}"
        );

        assert!(
            screen.contains("reload:1skill(s)"),
            "the skills that could be read still load: {screen}"
        );
    }

    /// **The sibling of the skill test above, and the arm that was left unwitnessed.**
    ///
    /// `/reload` forwards `discover_agents_with_notices`'s lines through `push_notice`, and the
    /// control that proved it — deleting that loop — left every test in this crate green,
    /// because the skill half had a test and the agent half had none. So this is that test:
    /// same shape, same portable unreadable root, same two-frame assertion, and the second
    /// half the loop could plausibly have broken — the defs that *could* be read still load, so
    /// adding the diagnostics did not turn a reload into a refusal.
    ///
    /// The trust bit is pinned rather than assumed: `/reload` reads the project agent roots
    /// only when `trust_review_pending` is clear, and a root that was never read produces no
    /// notice at all — so a fixture that left it set would make this pass for the wrong reason.
    #[tokio::test]
    async fn an_agents_root_reload_cannot_read_is_named_on_the_frame() {
        let (mut app, agent, (_f, _n)) = test_app();
        let dir = tempfile::tempdir().expect("a temporary workspace");
        app.workspace_root = dir.path().to_path_buf();
        app.trust_review_pending = false;

        // The good root, so "0 agents" cannot be mistaken for "discovery did nothing".
        let good = dir.path().join(".epsilon").join("agents");
        std::fs::create_dir_all(&good).expect("the .epsilon/agents directory");
        std::fs::write(
            good.join("scout.md"),
            "---\nname: scout\ndescription: Reads the tree and reports.\n---\nbody of scout\n",
        )
        .expect("the agent def");
        // The bad one: `.agents/agents` is a regular file, so `read_dir` refuses it — the same
        // portable trick as the skill root above, needing neither an ACL nor a privilege.
        let unreadable = dir.path().join(".agents").join("agents");
        std::fs::create_dir_all(dir.path().join(".agents")).expect("the .agents directory");
        std::fs::write(&unreadable, "a file where an agents root should be\n").unwrap();

        assert!(!type_and_enter(&mut app, &agent, "/reload"));

        // The whole sentence up to the reason, so a notice that named the root but no reason
        // does not satisfy this.
        let lead = squeezed(&format!(
            "agents: cannot read root {}: ",
            unreadable.display()
        ));
        for (w, h) in [(80u16, 24u16), (100u16, 30u16)] {
            let screen = frame(&mut app, w, h);
            assert!(
                screen.contains(&lead),
                "the frame must name the root it could not read AND leave room for the \
                 reason, at {w}x{h}: {screen}"
            );
        }

        // The reason. `/reload` always ends with its own summary line, so whatever sits
        // between the lead and that word is what discovery said caused the failure.
        let screen = frame(&mut app, 100, 30);
        let after = &screen[screen.find(&lead).expect("asserted above") + lead.len()..];
        let reason = after.split("reload:").next().unwrap_or_default();
        assert!(
            reason.len() >= 3,
            "a notice naming a root and no reason is the defect again: {screen}"
        );

        assert!(
            agents_loaded_on_frame(&screen).is_some_and(|n| n >= 1),
            "the agent defs that could be read still load: {screen}"
        );
    }

    /// The `+ N agent(s) loaded` half of `/reload`'s summary, read off the frame.
    ///
    /// **Parsed rather than written as a literal.** `/reload` also scans the *global* agents
    /// root (`~/.config/epsilon/agents`), so a person with a personal def would make a literal
    /// `1` fail — a test about the machine rather than about the product, which is why the
    /// sibling skill test reads its count back off the agent. There is no public
    /// `Agent::agents` here, so this reads the number the reader is actually shown.
    fn agents_loaded_on_frame(screen: &str) -> Option<usize> {
        let after = screen.split_once("skill(s)+")?.1;
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if !after[digits.len()..].starts_with("agent(s)") {
            return None;
        }
        digits.parse().ok()
    }

    /// **The regression guard: a healthy workspace must pay nothing for the new channel.**
    ///
    /// Fixing the silence is only worth it if the listing still works *and* stays quiet —
    /// a notice per launch is a channel people learn to stop reading, which is the failure
    /// `discover_with_notices`' own doc spends a paragraph avoiding. So this drives the two
    /// commands a user drives and asserts both halves on the frame: `/reload` still counts
    /// what it loaded, and `/skills` still lists each one by name **and description**.
    ///
    /// The count is read back off the agent rather than written as a literal, because
    /// `/reload` also scans a *global* skills root (`~/.config/epsilon/skills`). Hard-coding
    /// `2` would make this test fail on any machine whose user has a personal skill, which
    /// is a test about the machine and not about the product.
    #[tokio::test]
    async fn a_clean_reload_still_lists_the_skills_and_says_nothing_about_them() {
        let (mut app, agent, (_f, _n)) = test_app();
        let dir = tempfile::tempdir().expect("a temporary workspace");
        app.workspace_root = dir.path().to_path_buf();
        let root = dir.path().join(".epsilon").join("skills");
        write_skill(&root, "commit-craft", "Craft commits.");
        write_skill(&root, "review-diff", "Review a diff.");

        assert!(!type_and_enter(&mut app, &agent, "/reload"));
        assert!(!type_and_enter(&mut app, &agent, "/skills"));

        let held = agent.skills();
        for (name, description) in [
            ("commit-craft", "Craft commits."),
            ("review-diff", "Review a diff."),
        ] {
            let row = squeezed(&format!("{name} — {description}"));
            let found = held
                .iter()
                .any(|s| s.name == name && s.description == description);
            assert!(
                found,
                "{name} must reach the agent, or the listing this guards was broken: {held:?}"
            );
            let screen = frame(&mut app, 100, 30);
            assert!(
                screen.contains(&row),
                "the frame must still list {name} with its description: {screen}"
            );
        }

        let screen = frame(&mut app, 100, 30);
        assert!(
            screen.contains(&squeezed(&format!("reload:{}skill(s)", held.len()))),
            "the summary count is the number of skills actually held: {screen}"
        );
        assert!(
            !screen.contains("skills:cannotreadroot"),
            "two well-formed skills and nothing to report — a notice here is a tax: {screen}"
        );
        assert!(
            !screen.contains("skills:ignoring"),
            "nothing was rejected, so nothing should say one was: {screen}"
        );
    }
}
