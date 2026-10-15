//! Frame tests for the eleven verified screen defects of 2026-10-14.
//!
//! # Why this file exists
//!
//! Each of these defects was found by **reading a captured frame**, not by a failing
//! test — which is `AGENTS.md`'s own warning in its own words: *a test that passes is
//! not a claim that is true*. Several of them were invisible to the suite for a reason
//! worth naming: the assertion that looked like it covered the search panel
//! (`screen.contains("needle")`) passed against the broken frame, because the
//! *transcript behind the panel* also contained the word. Every test below therefore
//! asserts on the row that owns the claim — the panel's own query row, the hint row's
//! own text, the header's own model name — and drives the real key path where a key is
//! what a user would press.
//!
//! The theme is pinned the way `golden.rs` pins it: `Theme::detect()` reads the
//! environment, so a test that names a glyph has to name the palette that draws it.
//! Most of these use the bandless `Ansi16` mode, because that is where a gutter glyph
//! is the only thing carrying the role.

use super::tests_support::{render_buffer, render_rows, test_app};
use super::*;
use crate::ui::theme::{ColorMode as ThemeColorMode, Theme, UNICODE};
use crossterm::event::KeyEventState;

/// Pin the look so a frame test can assert exact glyphs.
fn pin_theme(app: &mut App, mode: ThemeColorMode) {
    app.theme = Theme::new(mode, UNICODE);
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// The workspace with one file in it, and an app pointed at it.
fn app_with_a_file() -> (App, Arc<Agent>, tempfile::TempDir) {
    let (mut app, agent, _rx) = test_app();
    let dir = tempfile::tempdir().expect("a temporary workspace");
    std::fs::write(
        dir.path().join("notes.txt"),
        "a needle here\nnothing at all\nand a needle again\n",
    )
    .expect("the fixture file");
    app.workspace_root = dir.path().to_path_buf();
    (app, agent, dir)
}

/// Open the file viewer on `notes.txt` through the tree's own key path.
fn open_the_editor(app: &mut App, agent: &Arc<Agent>) {
    super::keys::handle_key(app, agent, key(KeyCode::F(6)));
    assert!(app.tree_open, "F6 opens the tree");
    super::keys::handle_key(app, agent, key(KeyCode::Enter));
    assert!(app.viewer_open(), "Enter opens the first file");
}

/// One assistant answer, committed the way the loop commits one.
fn push_answer(app: &mut App, id: &str, text: &str) {
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    app.ui
        .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(AssistantMessage {
                id: id.into(),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: "faux-1".into(),
                content: vec![epsilon_contracts::content::ContentBlock::text(text)],
                stop_reason: StopReason::EndTurn,
                error_message: None,
                usage: Usage::default(),
            })),
        });
}

/// **Defect 1: the search panel drew its hint row over its query row.**
///
/// The frame audit's `search-marks.80x24` showed `n next · p previous · esc close`
/// and `1 of 2` and no `needle` anywhere: the panel is five rows tall (three inner
/// rows plus two borders) and the hint was placed at `inner.height - 3`, which is
/// row 0 — on top of `find <query>`.
///
/// **Why the old assertion passed anyway.** `an_open_search_reports_itself_...` in
/// `golden.rs` asserts `screen.contains("needle")`, and the transcript *behind* the
/// panel also contains "needle" — so the row that mattered was never the row that was
/// asserted on. This one asserts on the panel's own query row, and that the query is
/// not sharing it with the hint.
#[test]
fn the_search_panel_shows_the_query_it_is_searching_for() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    for text in ["alpha", "the needle is here", "beta", "another needle"] {
        app.ui.push_notice(text);
    }
    app.search_own_transcript("needle");
    assert_eq!(
        app.search.as_ref().map(|s| s.matches.len()),
        Some(2),
        "precondition: the query matches something"
    );

    let rows = render_rows(&mut app, 80, 24);
    let query_row = rows
        .iter()
        .position(|r| r.contains("find"))
        .unwrap_or_else(|| panic!("the panel's query row is nowhere on screen: {rows:#?}"));
    assert!(
        rows[query_row].contains("needle"),
        "the query row must name what is being searched for: {:?}",
        rows[query_row]
    );
    // The hint keeps its own row, so the two cannot be the same row again.
    let hint_row = rows
        .iter()
        .position(|r| r.contains("n next"))
        .unwrap_or_else(|| panic!("the key hint is nowhere on screen: {rows:#?}"));
    assert_ne!(
        query_row, hint_row,
        "the hint is drawn over the query again"
    );
    // And the position row is its own too: three rows, three statements.
    let position_row = rows
        .iter()
        .position(|r| r.contains("1 of 2"))
        .expect("the match position is on screen");
    assert_ne!(query_row, position_row);
}

/// **Defect 2: the file tree was bound to F6 and named nowhere.**
///
/// Not the first screen, not the palette, not the footer, not `/help` — a key
/// nobody can find is a key nobody presses. This asserts both halves of invariant 7:
/// the palette names it (with the chord the table resolves, not a typed `f6`), and
/// choosing the entry runs it rather than being a silent no-op.
#[tokio::test]
async fn the_palette_names_the_file_tree_and_choosing_it_opens_the_tree() {
    let (mut app, agent, _rx) = app_with_a_file();
    // Ctrl+P is the palette's binding; drive it through the table rather than
    // poking the field, so a rebinding cannot quietly orphan this test.
    let _palette = app
        .keybindings
        .key_text("app.palette")
        .expect("app.palette is bound");
    super::keys::handle_key(
        &mut app,
        &agent,
        KeyEvent {
            code: KeyCode::Char('p'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
    assert!(app.palette_open, "ctrl+p opens the palette");
    // "workspace" matches this entry and no other: the fuzzy filter is not the
    // thing under test, and a query that also matched another row would make the
    // selection below a coin toss.
    for c in "workspace".chars() {
        super::keys::handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    let rows = render_rows(&mut app, 80, 24);
    let row = rows
        .iter()
        .find(|r| r.contains("workspace tree"))
        .unwrap_or_else(|| panic!("the palette does not name the file tree: {rows:#?}"));
    let chord = app
        .keybindings
        .key_text("app.tree.toggle")
        .expect("app.tree.toggle is bound");
    assert!(
        row.contains(&format!("[{chord}]")),
        "the entry must show the chord the table resolves ({chord}): {row:?}"
    );

    // And it is not a no-op: choosing it opens the tree, exactly as F6 does.
    super::keys::handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(app.tree_open, "choosing the entry opens the tree");
    assert!(!app.palette_open, "the palette closed");

    // **And `/help` names it too.** The brief's defect was that F6 was named
    // nowhere — not the first screen, not the palette, not the footer, not
    // `/help`. A palette entry fixes half of that; the help guide is the other
    // half, and the chord is resolved from the same table so the two cannot
    // drift apart.
    app.run_command(
        &agent,
        crate::picker::parse_command("/help").expect("/help parses"),
    );
    let notices = app.ui.lines().to_vec();
    let promised = format!("{chord} opens the file tree");
    assert!(
        notices.iter().any(|n| n.text.contains(&promised)),
        "/help must name the chord ({promised}) — a key nobody is told about is a \
         key nobody presses: {:?}",
        notices.iter().map(|n| n.text.clone()).collect::<Vec<_>>()
    );
}

/// **Defect 3: a long tool phase ate the header's right half.**
///
/// At 80 columns the header read `⠋ running 0s · bash: Compiling epsilon-tools v0.1.0 ed)`
/// — the model name cut to its last three letters, because the phase was emitted
/// whole and `side_by_side` trims the half that yields (Priority::Left) from its far
/// end, and the model is one span. The phase is the fragment with no length of its
/// own, so it is the fragment that gets a budget.
#[test]
fn a_long_tool_phase_does_not_eat_the_model_name() {
    use epsilon_contracts::agent_event::AgentEvent;
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let (mut app, _agent, _rx) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m-phase".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                id: "tc-phase".into(),
                name: "bash".into(),
                arguments: serde_json::json!({ "command": "cargo build" }),
            }],
            stop_reason: StopReason::ToolUse,
            error_message: None,
            usage: Usage::default(),
        })),
    });
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "tc-phase".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({ "command": "cargo build" }),
        },
    );
    // A tool status line in the tool's own words — the M63 partial, which is what
    // `UiState::status` and therefore the header's phase are made of.
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::ToolExecutionUpdate {
            tool_call_id: "tc-phase".into(),
            tool_name: "bash".into(),
            partial: epsilon_contracts::tool::ToolPartial {
                content: vec![epsilon_contracts::content::ContentBlock::Text {
                    text: "bash: Compiling epsilon-tools v0.1.0".into(),
                }],
                details: serde_json::Value::Null,
            },
        },
    );
    // The run is in flight: `run_state` reads that from the UiState flag, which
    // the event that starts a turn sets in a real run.
    app.ui.set_running_for_test(true);

    let rows = render_rows(&mut app, 80, 24);
    let label = app.metrics().model_label;
    assert!(
        rows[0].contains(&label),
        "the model name must survive a long phase at 80 columns: {:?}",
        rows[0]
    );
    assert!(
        rows[0].contains("running"),
        "the run state must survive too: {:?}",
        rows[0]
    );
    // The phase is what pays: it may be cut, but it is still there saying something.
    assert!(
        rows[0].contains("bash"),
        "the phase is the fragment that yields, so it is cut and not deleted: {:?}",
        rows[0]
    );
    assert_eq!(
        crate::ui::layout::display_width(&rows[0]),
        80,
        "the row is exactly the terminal's width: {:?}",
        rows[0]
    );
}

/// **Defect 4: the file editor left the live screen sliced in half behind it.**
///
/// The panel is centred at four fifths of the screen, so at 80 columns it occupies
/// columns 8..72 and the transcript behind it was visible on both sides — cut
/// mid-word by the panel's rails, which is what `file-editor-find.80x24` shows as
/// `a coding│ a needle here`. The panel now owns the band it is drawn in.
#[test]
fn the_file_editor_clears_the_screen_behind_it() {
    let (mut app, agent, _dir) = app_with_a_file();
    // A transcript long enough that the rows behind the panel are not empty: an
    // empty one would make this pass against the broken build.
    for i in 0..40 {
        app.ui
            .push_notice(format!("transcript row {i} behind the open editor"));
    }
    open_the_editor(&mut app, &agent);
    super::keys::handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    assert!(
        app.active_viewer().is_some_and(|v| v.editing),
        "precondition: the editor, not the viewer"
    );

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    let top = rows
        .iter()
        .position(|r| r.contains("edit notes.txt"))
        .unwrap_or_else(|| panic!("the editor panel is not on screen: {rows:#?}"));
    // **The rails, by their own glyphs.** The first non-blank cell of the title
    // row is NOT the panel's left rail when the screen behind it is still
    // visible: the sliced transcript's own gutter starts in column 0, and
    // measuring from there makes the loop below check no columns at all — which
    // is how this test passed against the broken build the first time.
    let left = rows[top]
        .find(app.theme.glyphs.top_left)
        .unwrap_or_else(|| panic!("the panel's left rail is not on screen: {rows:#?}"));
    let right = rows[top]
        .rfind(app.theme.glyphs.top_right)
        .expect("the panel's right rail");
    let bottom = rows
        .iter()
        .rposition(|r| r.contains("type/enter"))
        .expect("the editor's own footer row");
    let mut leaked = Vec::new();
    for y in top + 1..bottom {
        for x in 0..left as u16 {
            if buffer[(x, y as u16)].symbol() != " " {
                leaked.push(format!("row {y} column {x}: {:?}", rows[y]));
                break;
            }
        }
        for x in right as u16 + 1..80u16 {
            if buffer[(x, y as u16)].symbol() != " " {
                leaked.push(format!("row {y} column {x}: {:?}", rows[y]));
                break;
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "the live screen must not be visible beside the editor, sliced in half: {leaked:#?}\n{rows:#?}"
    );
}

/// **Defect 5: the editor's find bar advertised only the step it supports forward.**
///
/// `Up`, `PageUp` and `p` all step back through the open file's matches
/// (`app/keys.rs`), and the row said `enter next · esc close` — half the keys it
/// describes, in a different vocabulary from the transcript search's own `n next ·
/// p previous`. The two now spell the verb the same way.
#[test]
fn the_editor_find_bar_names_the_backward_step() {
    let (mut app, agent, _dir) = app_with_a_file();
    open_the_editor(&mut app, &agent);
    super::keys::handle_key(&mut app, &agent, key(KeyCode::Char('/')));
    // "eedle", not "needle": `n` is the forward step while find owns the
    // keyboard (`app/keys.rs`), so it types nothing — which is the find bar's
    // own story, not this one's.
    for c in "eedle".chars() {
        super::keys::handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    let rows = render_rows(&mut app, 80, 24);
    let find_row = rows
        .iter()
        .position(|r| r.contains("/eedle"))
        .unwrap_or_else(|| panic!("the find row is not on screen: {rows:#?}"));
    assert!(
        rows[find_row].contains("enter/n next · p previous · esc close"),
        "the find row must name both steps the way the transcript search does: {:?}",
        rows[find_row]
    );
}

/// **Defect 6: a wrapped answer repeated its gutter glyph on every row.**
///
/// One assistant paragraph drew as three `▪` rows — a bulleted list nobody wrote.
/// A card's body already nests under its title with the vertical rule; that rule now
/// covers plain rows too, so the role glyph belongs to the line's first row and the
/// continuation rows draw the rule.
///
/// **While it streams and after it is committed.** The tail is drawn at draw time
/// (it is not in the render cache yet) and the committed line comes back out of the
/// cache, so the two are two pieces of code and they have to agree: an answer that
/// changes shape when the turn ends is the round-72 defect in a new place, and the
/// marker is the part a reader counts.
#[test]
fn a_wrapped_answer_draws_one_marker_and_continuation_rules() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    let text = "The answer begins here with the first row of a paragraph that is deliberately \
                long enough to wrap onto three rows at eighty columns, and the middle of it \
                carries a distinctive phrase, while the tail of the same paragraph ends with a \
                closing phrase.";
    // The tail, exactly as a turn in flight draws it: a `MessageUpdate` delta is
    // what a streaming turn sends.
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    app.ui.apply(&AgentEvent::MessageUpdate {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "stream".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::text(text)],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Usage::default(),
        })),
    });
    let tail = render_rows(&mut app, 80, 24);
    let tail_first = tail
        .iter()
        .position(|r| r.contains("The answer begins here"))
        .expect("the streaming answer is on screen");
    assert!(
        tail[tail_first].starts_with(UNICODE.assistant),
        "streaming: the first row carries the marker: {:?}",
        tail[tail_first]
    );
    for word in ["distinctive phrase", "closing phrase"] {
        let at = tail
            .iter()
            .position(|r| r.contains(word))
            .unwrap_or_else(|| panic!("{word} is on screen: {tail:#?}"));
        assert!(
            tail[at].starts_with(UNICODE.vertical),
            "streaming: the wrapped row for {word} draws the rule: {:?}",
            tail[at]
        );
    }

    // The same text, committed the way the loop commits it.
    let mut committed = bench_app(UiState::new());
    pin_theme(&mut committed, ThemeColorMode::Ansi16);
    push_answer(&mut committed, "m-committed", text);
    let rows = render_rows(&mut committed, 80, 24);
    let first = rows
        .iter()
        .position(|r| r.contains("The answer begins here"))
        .unwrap_or_else(|| panic!("the answer never reached the screen: {rows:#?}"));
    let middle = rows
        .iter()
        .position(|r| r.contains("distinctive phrase"))
        .expect("the wrapped middle row");
    let tail_row = rows
        .iter()
        .position(|r| r.contains("closing phrase"))
        .expect("the wrapped tail row");
    assert!(
        rows[first].starts_with(UNICODE.assistant),
        "the line's FIRST row carries the role marker: {:?}",
        rows[first]
    );
    for (name, row) in [("middle", middle), ("tail", tail_row)] {
        assert!(
            rows[row].starts_with(UNICODE.vertical),
            "the {name} continuation row must draw the rule, not repeat the marker: {:?}",
            rows[row]
        );
    }
    // And the marker is not on any of them: a paragraph is not a list.
    let markers = rows
        .iter()
        .filter(|r| r.trim_start().starts_with(UNICODE.assistant))
        .count();
    assert_eq!(
        markers, 1,
        "one answer, one marker — not one per wrapped row: {rows:#?}"
    );
}

/// **Defect 7: the editor never named the undo key it implements.**
///
/// Ctrl+Z takes an edit back and the footer said `type/enter edit · alt+s save ·
/// esc cancel`. Resolved through the table, so a rebinding moves the text.
#[test]
fn the_editor_footer_names_the_undo_key() {
    let (mut app, agent, _dir) = app_with_a_file();
    open_the_editor(&mut app, &agent);
    super::keys::handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    let undo = app
        .keybindings
        .key_text("tui.editor.undo")
        .expect("tui.editor.undo is bound");

    let rows = render_rows(&mut app, 80, 24);
    let footer = rows
        .iter()
        .find(|r| r.contains("type/enter"))
        .unwrap_or_else(|| panic!("the editor's footer is not on screen: {rows:#?}"));
    assert!(
        footer.contains(&format!("{undo} undo")),
        "the editor must name the key that undoes ({undo}): {footer:?}"
    );
}

/// **Defect 8: the footer's last hint collapsed to a bare key at 80 columns.**
///
/// `idle-with-content.80x24` read `… · pageup scroll back · pagedown …\Epsilon\…`:
/// the last hint lost its action and named a key with no words. The location was
/// pinned at 28 columns — 35% of an 80-column row — so it now takes its share of
/// the frame instead, and the hint fits whole.
///
/// **The bare-key arm in `draw` is deliberately left in place**, and this test is
/// the reason it can be: with the location scaled, the row a reader gets at 80
/// columns has no bare key on it. `the_approval_footer_never_prints_half_a_hint_either`
/// pins the other half of that decision (a bare `e` on the approval row is a real,
// bound, handled key with a missing label — degraded, not a lie).
#[test]
fn the_footer_at_80_columns_never_drops_a_hint_to_a_bare_key() {
    let (mut app, _agent, _rx) = test_app();
    app.ui.push_local_input("ask anything");

    let rows = render_rows(&mut app, 80, 24);
    let footer = rows.last().expect("the footer row");
    let hints = app.hints();
    let location = crate::ui::footer::location_span(
        hints.location.as_deref().unwrap_or_default(),
        80,
        &app.theme,
    );
    let head = footer.split(&location).next().unwrap_or(footer);
    let mut legal: Vec<String> = Vec::new();
    if let Some(mode) = &hints.mode {
        legal.push(mode.clone());
    }
    if let Some(goal) = &hints.goal {
        legal.push(goal.clone());
    }
    if let Some(badge) = hints.view.badge(&app.theme) {
        legal.push(badge);
    }
    for (key_text, action) in &hints.keys {
        legal.push(format!("{key_text} {action}"));
    }
    for part in head.split(" · ").skip(1) {
        let part = part.trim_end();
        if part.is_empty() {
            continue;
        }
        assert!(
            legal.iter().any(|l| l == part),
            "`{part}` is a key with no action — the hint was dropped to its key: {footer:?}"
        );
    }
    // The symptom that started this: `scroll back` fits and keeps its words.
    assert!(
        footer.contains("scroll back"),
        "the scroll hint must survive with its action at 80 columns: {footer:?}"
    );
}

/// **Defect 9: the `@` file popup had no empty state.**
///
/// An unmatched fragment closed the popup and the screen said nothing — the reader
/// typed a path, saw nothing change, and could not tell "no such file" from "the
/// popup is broken". Every sibling popup names its empty case; this one now does.
#[tokio::test]
async fn an_unmatched_at_fragment_says_so_on_the_frame() {
    let (mut app, agent, _rx) = app_with_a_file();
    for c in "@zzz".chars() {
        super::keys::handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.at_open, "the popup stays open on an unmatched fragment");

    let rows = render_rows(&mut app, 80, 24);
    assert!(
        rows.iter().any(|r| r.contains("no matching files")),
        "an unmatched fragment must say so, not go silent: {rows:#?}"
    );
    assert!(
        rows.iter().any(|r| r.contains("zzz")),
        "and it must name the fragment that matched nothing: {rows:#?}"
    );
}

/// **Defect 10: the command palette touched the frame edge at 60 columns.**
///
/// `Width::Fixed(64)` was clamped to the frame's width, so a 60-column terminal
/// drew a 60-column box flush against both edges while every other overlay kept a
/// margin. The height already reserved two rows; the width now reserves two columns.
#[tokio::test]
async fn the_palette_keeps_a_margin_at_60_columns() {
    let (mut app, agent, _rx) = test_app();
    super::keys::handle_key(
        &mut app,
        &agent,
        KeyEvent {
            code: KeyCode::Char('p'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
    assert!(app.palette_open, "precondition: the palette is open");

    let rows = render_rows(&mut app, 60, 20);
    let title = rows
        .iter()
        .position(|r| r.contains("commands"))
        .unwrap_or_else(|| panic!("the palette is not on screen: {rows:#?}"));
    // The panel's own left rail, not the first non-blank cell on the row: the
    // welcome screen's `E P S I L O N` is still drawn behind the palette at
    // this width, and it starts in column 0.
    let left = rows[title]
        .find(app.theme.glyphs.top_left)
        .unwrap_or_else(|| panic!("the palette's left rail is not on screen: {rows:#?}"));
    assert_eq!(
        left, 1,
        "the palette must keep a column of frame either side, like every other overlay: {:?}",
        rows[title]
    );
    // And the same box at 80 columns, where nothing about the width changes.
    let rows = render_rows(&mut app, 80, 24);
    let title = rows
        .iter()
        .position(|r| r.contains("commands"))
        .expect("the palette at 80");
    assert_eq!(
        rows[title].find(app.theme.glyphs.top_left),
        Some(8),
        "80 columns: {:?}",
        rows[title]
    );
    // **And nothing live shows through the band.** The margin fix cleared the
    // box's own area only, so at 60 columns the transcript row sharing the
    // bottom rail's row still printed through the margin — `▎╰────╯`, a rail
    // that reads as broken. The palette now clears its row band across the
    // full width before it draws, and this is the assertion that keeps it:
    // every column either side of the box's rails is blank on every row the
    // box occupies.
    let band_top = title;
    let band_bottom = rows
        .iter()
        .enumerate()
        .skip(title)
        .find(|(_, r)| r.contains(app.theme.glyphs.bottom_left))
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("the palette's bottom rail is not on screen: {rows:#?}"));
    for (i, row) in rows.iter().enumerate().take(band_bottom + 1).skip(band_top) {
        let l = row
            .find(app.theme.glyphs.top_left)
            .or_else(|| row.find(app.theme.glyphs.vertical))
            .unwrap_or(0);
        let r = row
            .rfind(app.theme.glyphs.top_right)
            .or_else(|| row.rfind(app.theme.glyphs.vertical))
            .unwrap_or(row.len());
        for (c, ch) in row.chars().enumerate() {
            if c < l || c > r {
                assert_eq!(
                    ch, ' ',
                    "row {i} shows live page content ({ch:?}) beside the palette: {row:?}"
                );
            }
        }
    }
}

/// **Defect 11: a card that was streaming output read `pending`.**
///
/// The frame showed a header saying `running bash…` and a card saying `▸ bash $
/// cargo build · pending` for the same tool — three rows, three words, one call.
/// `Pending` is "asked for, not yet running" (which is what a permission prompt
/// leaves a call in); `Running` is the one whose body is filling. The glyph is
/// unchanged, so the gutter still looks the same at a glance.
#[test]
fn a_running_tool_card_says_running_and_not_pending() {
    use epsilon_contracts::agent_event::AgentEvent;
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let (mut app, _agent, _rx) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    let call = AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m-card".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                id: "tc-card".into(),
                name: "bash".into(),
                arguments: serde_json::json!({ "command": "cargo build" }),
            }],
            stop_reason: StopReason::ToolUse,
            error_message: None,
            usage: Usage::default(),
        })),
    };
    super::apply_with_scroll(&mut app, &call);
    // **The frame in between is the point.** Pending first: asked for, not started.
    let pending = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        pending.contains("cargo build · pending"),
        "a call that has not started still says pending: {pending}"
    );

    super::apply_with_scroll(
        &mut app,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "tc-card".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({ "command": "cargo build" }),
        },
    );
    let running = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        running.contains("cargo build · running"),
        "the same card, now running, must say running: {running}"
    );
    assert!(
        !running.contains("cargo build · pending"),
        "the card must not contradict the header's own `running`: {running}"
    );
}
// ---------------------------------------------------------------------------
// The 2026-10-14 (second) set: truncation marker, rows wording, hidden counts,
// the sanitise hole, a cancelled card, the footer's session name, the retry
// counter, and the CSI residue's reach into a card.
//
// Each test names the row that owns its claim and drives the real reducer, for
// the reason this file's header gives: an assertion that would pass against the
// broken frame is an assertion that has not been made yet.
// ---------------------------------------------------------------------------

/// One committed tool result, through the same reducer the event loop uses.
fn push_tool_result(
    app: &mut App,
    tool_call_id: &str,
    tool_name: &str,
    body: &str,
    is_error: bool,
) {
    use epsilon_contracts::message::{Message, ToolResultMessage};
    super::apply_with_scroll(
        app,
        &AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: "msg-result".into(),
                timestamp_ms: 0,
                tool_call_id: tool_call_id.into(),
                tool_name: tool_name.into(),
                content: vec![epsilon_contracts::content::ContentBlock::text(body)],
                details: serde_json::json!({}),
                usage: None,
                is_error,
            })),
        },
    );
}

/// **C1: the card's cap ate the tool's own omission marker.**
///
/// `epsilon_tools::util::truncate_output` cuts a long output in half and writes
/// its own marker between the halves — exact line and byte counts plus the spill
/// locator. The card keeps the **tail** of a text body, so the marker sat in the
/// dropped half and `cap` replaced it with its own plainer `… N earlier rows`.
/// A reader therefore saw how many rows were hidden and nothing about how much
/// *output* was dropped or where the rest of it lives.
///
/// **The body is the tool's own output, not a fixture.** The test calls
/// `truncate_output` and feeds its result through the reducer, so the marker's
/// wording, its position between the halves and the spill path are all the real
/// ones — a hand-written `[... ...]` row would keep passing if the tool's
/// wording ever changed.
#[test]
fn a_capped_body_keeps_the_tools_own_omission_marker() {
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let (mut app, _agent, _rx) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    // The shape `truncate_output` exists for: the first error at the top, the
    // verdict at the bottom, thousands of noise lines between them.
    let mut lines = vec!["error[E0308]: mismatched types at src/first.rs:12".to_string()];
    for i in 0..4000 {
        lines.push(format!("note: intermediate line {i}"));
    }
    lines.push("aborting due to 1 previous error".to_string());
    let spill = tempfile::tempdir().expect("a spill directory");
    let t = epsilon_tools::util::truncate_output(&lines.join("\n"), spill.path(), "build");
    assert!(
        t.truncated,
        "precondition: the tool truncated its own output"
    );
    assert!(
        t.text.contains("[... "),
        "the tool wrote its own marker: {}",
        &t.text[..t.text.len().min(200)]
    );

    super::apply_with_scroll(
        &mut app,
        &AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(AssistantMessage {
                id: "m-build".into(),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: "faux-1".into(),
                content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                    id: "tc-build".into(),
                    name: "bash".into(),
                    arguments: serde_json::json!({ "command": "cargo build" }),
                }],
                stop_reason: StopReason::ToolUse,
                error_message: None,
                usage: Usage::default(),
            })),
        },
    );
    push_tool_result(&mut app, "tc-build", "bash", &t.text, false);

    let rows = render_rows(&mut app, 80, 24);
    // The marker row, by its own opening: it starts at the body's column 0, so
    // it is the row that begins with the tool's own `[... `.
    let marker = rows
        .iter()
        .find(|r| r.contains("[..."))
        .unwrap_or_else(|| panic!("the tool's omission marker is nowhere on screen: {rows:#?}"));
    assert!(
        marker.contains("lines /") && marker.contains("bytes omitted"),
        "the marker carries the tool's exact counts: {marker:?}"
    );
    // And the tail the cap was designed to keep is still there.
    assert!(
        rows.iter()
            .any(|r| r.contains("aborting due to 1 previous error")),
        "the tail survives the cap: {rows:#?}"
    );
    // The card's own affordance is still there and still honest — it now counts
    // one row fewer, because the marker it used to replace is drawn.
    assert!(
        rows.iter().any(|r| r.contains("earlier rows")),
        "the card still names how much is hidden: {rows:#?}"
    );
}

/// **C2: `… N earlier lines` was counting wrapped rows.**
///
/// The affordance's number comes from the cap, and the cap counts **rows** —
/// that is what [`crate::ui::card::BODY_ROWS`] budgets, because a long line
/// costs its wrapped rows. A body of twenty 70-column lines at a 60-column frame
/// is forty rows, so the row read `… 28 earlier lines` beside a body whose
/// twenty-eighth *line* was on screen: arithmetic a reader cannot reconcile with
/// what they are looking at. The word now says what was counted.
#[test]
fn a_capped_body_says_rows_because_it_counts_wrapped_rows() {
    let (mut app, _agent, _rx) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    // 20 lines of 70 columns: two rows each at a 60-column frame, so the body is
    // 40 rows against a 12-row cap — and 20 logical lines, a number the row must
    // not print.
    let body = (0..20)
        .map(|i| format!("log line {i:02} {}", "x".repeat(58)))
        .collect::<Vec<_>>()
        .join("\n");
    push_tool_result(&mut app, "tc-wrap", "bash", &body, false);

    for (width, height) in [(60u16, 20u16), (80, 24)] {
        let rows = render_rows(&mut app, width, height);
        let row = rows
            .iter()
            .find(|r| r.contains("earlier"))
            .unwrap_or_else(|| panic!("width {width}: no affordance on screen: {rows:#?}"));
        assert!(
            row.contains("earlier rows"),
            "width {width} must count rows, because rows are what the cap counts: {row:?}"
        );
        assert!(
            !row.contains("earlier lines"),
            "width {width} must not call a wrapped row a line: {row:?}"
        );
    }
}

/// **C3: `TranscriptLine::hidden` was written and never read.**
///
/// Four sites folded the strip count into the field and its doc promised "the
/// reader is told"; no render path consulted it, so a line with a bidi override
/// in it was silently rewritten on screen. The count now rides the line's own
/// first row, in the faintest style on it — which is the disposition chosen over
/// deleting the field, because a silent strip is the failure the counter exists
/// to prevent (`epsilon-runtime`'s `text_safety` header says so in as many
/// words).
#[test]
fn a_line_that_had_characters_stripped_says_so_on_the_frame() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    // `push_external_text` is the public door for text the harness did not
    // write — the seam `Card::sanitize`'s own doc names as the one a hostile
    // character has to arrive through.
    app.ui.push_external_text("ls\u{202E} -R /tmp");
    assert_eq!(
        app.ui.lines()[0].hidden.bidi,
        1,
        "precondition: the line recorded what the strip removed"
    );

    let rows = render_rows(&mut app, 80, 24);
    let row = rows
        .iter()
        .find(|r| r.contains("-R /tmp"))
        .unwrap_or_else(|| panic!("the stripped line is not on screen: {rows:#?}"));
    assert!(
        row.contains("1 hidden character"),
        "the reader must be told what left the row: {row:?}"
    );
    // The override itself is gone: what is drawn is what the command does.
    assert!(
        !row.contains('\u{202e}'),
        "the override reached the screen: {row:?}"
    );
}

/// **C4: a hostile tool name reached the card title.**
///
/// `Card::sanitize` stripped the body, the headline and the arguments and never
/// looked at `name` — the one field drawn from the model's own spelling of the
/// tool. A name carrying a whole CSI sequence or a bidi override was therefore
/// drawn verbatim in the title while the line's `text`, which carries the same
/// name, was stripped beside it.
#[test]
fn a_hostile_tool_name_never_reaches_the_card_title() {
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let hostile = "ba\u{1b}[0msh\u{202e}";
    let (mut app, _agent, _rx) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(AssistantMessage {
                id: "m-hostile".into(),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: "faux-1".into(),
                content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                    id: "tc-hostile".into(),
                    name: hostile.into(),
                    arguments: serde_json::json!({ "command": "cargo build" }),
                }],
                stop_reason: StopReason::ToolUse,
                error_message: None,
                usage: Usage::default(),
            })),
        },
    );

    let rows = render_rows(&mut app, 80, 24);
    let title = rows
        .iter()
        .find(|r| r.contains("cargo build"))
        .unwrap_or_else(|| panic!("the card is not on screen: {rows:#?}"));
    for c in title.chars() {
        assert!(
            !('\u{0}'..='\u{1f}').contains(&c) && c != '\u{202e}',
            "a display-steering character survived into the card title: {title:?}"
        );
    }
    assert!(
        title.contains("bash"),
        "the readable part of the name survives: {title:?}"
    );
    // And the data the renderer reads is clean, not only the pixels: a second
    // renderer (or a resumed session) reads `Card::name`.
    let card = app.ui.lines()[0].card.as_ref().expect("the card");
    assert_eq!(card.name, "bash", "the stored name is stripped too");
}

/// **C4's other half: the orphaned result card.** The name test above drives a
/// settled card (call committed, then result), which sanitizes at construction.
/// The orphaned path — a result whose call card never arrived (a replay that
/// lost it, a result carried from another branch) — used to build its card
/// inline without `Card::sanitize`, so its name, body and headline were the one
/// card in the transcript drawn verbatim while its siblings were stripped. The
/// control that proves this test sees that path: replacing `card.sanitize()`
/// at the orphan construction with `Stripped::default()` fails it.
#[test]
fn an_orphaned_tool_result_card_is_sanitized_like_every_other_card() {
    let hostile = "ba\u{1b}[0msh\u{202e}";
    let (mut app, _agent, _rx) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    // No ToolCall for this id: the reducer takes the orphan branch.
    push_tool_result(&mut app, "tc-never-called", hostile, "the body", false);

    let rows = render_rows(&mut app, 80, 24);
    let title = rows
        .iter()
        .find(|r| r.contains("the body"))
        .unwrap_or_else(|| panic!("the orphaned card is not on screen: {rows:#?}"));
    for c in title.chars() {
        assert!(
            !('\u{0}'..='\u{1f}').contains(&c) && c != '\u{202e}',
            "a display-steering character survived into the orphaned card's title: {title:?}"
        );
    }
    assert!(
        title.contains("bash"),
        "the readable part of the name survives on the orphan path too: {title:?}"
    );
}

/// **C8: does the CSI fix (f9fa4b7) reach a card? It does — and this is the
/// frame that proves it.**
///
/// `display_safe` now removes a whole CSI sequence, so `--color=always` output
/// no longer arrives as `[0;32m` printable ASCII on a card. The card path
/// reaches it through `Card::sanitize` on the settled body, which is what this
/// drives: a coloured compiler line, committed as a real tool result, drawn at
/// 80 and 100 columns. The assertion is on the **absence** of the residue,
/// because the failure mode is extra text rather than missing text.
#[test]
fn a_coloured_tool_result_reaches_the_card_without_its_escape_residue() {
    let (mut app, _agent, _rx) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    let body = "\u{1b}[0;32m   Compiling epsilon-tui v0.1.0\u{1b}[0m\n\
                \u{1b}[1;31mmatch\u{1b}[0m:12\u{1b}[K\n\
                error[E0308]: mismatched types at src/first.rs:12";
    push_tool_result(&mut app, "tc-colour", "bash", body, false);

    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let screen = render_rows(&mut app, width, height).join("\n");
        assert!(
            screen.contains("Compiling epsilon-tui"),
            "width {width}: the line itself must survive: {screen}"
        );
        for residue in ["[0;32m", "[0m", "[1;31m", "[K"] {
            assert!(
                !screen.contains(residue),
                "width {width}: escape residue {residue:?} reached the card: {screen}"
            );
        }
    }
}

/// **C7: a cancelled command said `failed`.**
///
/// The shell tool reports a cancellation as an ordinary failure
/// (`epsilon-tools/src/shell.rs:451`, `:570`) and epsilon-core turns a tool
/// failure into an error result carrying that text verbatim, so the card read
/// `✗ bash $ cargo build · failed` for a command the reader stopped themselves.
/// `CardStatus::Cancelled` now names it, and pi spells the same distinction
/// `(cancelled)` — never beside an exit number
/// (`references/pi-1.0.0/packages/coding-agent/src/modes/interactive/components/bash-execution.ts:189`,
/// `:192`).
#[test]
fn a_cancelled_shell_card_says_cancelled_and_not_failed() {
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let (mut app, _agent, _rx) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(AssistantMessage {
                id: "m-cancel".into(),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: "faux-1".into(),
                content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                    id: "tc-cancel".into(),
                    name: "bash".into(),
                    arguments: serde_json::json!({ "command": "cargo build" }),
                }],
                stop_reason: StopReason::ToolUse,
                error_message: None,
                usage: Usage::default(),
            })),
        },
    );
    push_tool_result(
        &mut app,
        "tc-cancel",
        "bash",
        "[aborted] command cancelled by user before completion: cargo build",
        true,
    );

    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let screen = render_rows(&mut app, width, height).join("\n");
        assert!(
            screen.contains("cancelled"),
            "width {width}: the card must say the user cancelled it: {screen}"
        );
        assert!(
            !screen.contains("failed"),
            "width {width}: a decision must not be reported as a defect: {screen}"
        );
    }
    // The control on the whole feature: the same command failing on its own
    // merit must still say `failed`. One test that only ever saw the cancelled
    // case would pass against a renderer that printed the same word for both.
    let (mut app, _agent, _rx) = test_app();
    push_tool_result(
        &mut app,
        "tc-other",
        "bash",
        "cargo build\nerror: could not compile `epsilon-tui`",
        true,
    );
    let screen = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        screen.contains("failed") && !screen.contains("cancelled"),
        "an ordinary failure keeps its own word: {screen}"
    );
}

/// **C9: the footer names the session beside the location (P-200).**
///
/// The `~` half was already implemented — `App::new` shortens the workspace root
/// with `format_path` — but neither half was pinned by a test, and the session
/// name was drawn nowhere. This asserts both on one real row: the location, and
/// the `•`-separated session name beside it.
#[test]
fn the_footer_names_the_session_beside_the_location() {
    let (mut app, _agent, _rx) = test_app();
    app.session_name = Some("work".into());
    let rows = render_rows(&mut app, 80, 24);
    let footer = rows.last().expect("the footer row");
    assert!(
        footer.contains("• work"),
        "the session name must ride the footer: {footer:?}"
    );
    // The location is on the same row, and it is the shortened one the app
    // prepared once at construction rather than a per-frame recompute: the row
    // shows what `location_span` makes of that label at this width, so the test
    // asserts the same transformation the draw applies.
    let label = app.location_labels[0].clone();
    let shown = crate::ui::footer::location_span(&label, 80, &app.theme);
    assert!(
        footer.contains(&shown),
        "the workspace stays on the row beside it ({shown:?}): {footer:?}"
    );
    // And an unnamed session draws no bullet.
    let (mut app, _agent, _rx) = test_app();
    app.session_name = None;
    let footer = render_rows(&mut app, 80, 24).pop().expect("the footer row");
    assert!(
        !footer.contains('•'),
        "an unnamed session must not draw a separator: {footer:?}"
    );
}

/// **C10: the retry notice carries a counter and a countdown (P-189).**
///
/// The notice used to reach the status line as one sentence —
/// `HTTP 503: overloaded — retrying gpt-x in 2.0s (attempt 2) (server error)` —
/// with no counter a reader can follow across attempts and no countdown. Pi
/// draws `Retrying (attempt/max) in Ns... (KEY to cancel)` and re-renders it each
/// whole second
/// (`references/pi-1.0.0/packages/coding-agent/src/modes/interactive/components/status-indicator.ts:56,64`,
/// `components/countdown-timer.ts:20-32`); the `esc` ours names is the
/// `app.interrupt` the footer already advertises while a run is live.
#[test]
fn the_header_counts_a_retry_down_in_whole_seconds() {
    let (mut app, _agent, _rx) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::Notice {
            text: "HTTP 503: overloaded — retrying faux-1 in 30.0s (attempt 3) (server error)"
                .into(),
        },
    );
    app.ui.set_running_for_test(true);

    // 140, not 80 or 100: the countdown is emitted whole or not at all, and the
    // width that decides it is the row's pinned model tail. Measured here: an
    // offline session's `Faux (offline scripted) • thinking off · offline` takes
    // ~46 columns of pinned tail, so the ~37-column countdown fits at 140 and is
    // dropped whole below that — the rule the header's own unit test pins with
    // controlled measurements (`a_retry_countdown_is_drawn_whole_or_not_at_all`).
    // A real (non-offline) model label is a third of that size and fits from 100.
    let screen_rows = render_rows(&mut app, 140, 30);
    assert!(
        screen_rows[0].contains("Retrying (3/5) in 30s... (esc to cancel)"),
        "the header must state the counter, the countdown and the way out: {:?}",
        screen_rows[0]
    );
    // The transcript keeps the notice's own sentence, so a pause is still
    // explainable afterwards — the counter does not replace the record.
    let rows = render_rows(&mut app, 80, 24);
    assert!(
        rows.iter().any(|r| r.contains("retrying faux-1")),
        "the notice itself stays in the transcript: {rows:#?}"
    );
    // And a notice that is not a retry leaves the header alone: a stall
    // watchdog's "still streaming" is not a countdown.
    let (mut app, _agent, _rx) = test_app();
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::Notice {
            text: "no output for 60s — still streaming".into(),
        },
    );
    app.ui.set_running_for_test(true);
    let rows = render_rows(&mut app, 80, 24);
    assert!(
        !rows[0].contains("Retrying ("),
        "a non-retry notice must not be counted down: {:?}",
        rows[0]
    );
    // At 140 the phase fits whole, and the watchdog's own words are on screen.
    // At 80 the phase budget legitimately cuts it (`no output for 6…`) — the
    // budget exists to protect the model name, and a notice is the thing it
    // spends — so the assertion names the width that answers the question.
    let wide = render_rows(&mut app, 140, 24);
    assert!(
        wide[0].contains("still streaming"),
        "and it is the phase, as it was: {:?}",
        wide[0]
    );
}

// ==========================================================================================
// The permission dialog shows the change it is approving
//
// A dialog that carries only `cap` and one line of `what` asks a reader to authorise a write
// having seen neither the old nor the new bytes: `summarize_request` answers `what` from
// `detail["path"]`, so the screen named the file and nothing else. The preview seam closes
// that — `Tool::preview` (epsilon-contracts) produces the diff, `exec::ask_detail`
// (epsilon-core) merges it into the ask's `detail`, and `draw_dialog` renders it in the shape
// a tool result's diff already uses.
//
// **Every frame below is produced by the real ask path.** The fixtures park an ask through
// `epsilon_core::exec::run_local`, which runs the tool's real `preview` and the real merge,
// rather than hand-writing a `detail` object. That is what makes the controls real: deleting
// the merge in `exec.rs` makes the diff vanish from the frame, and these tests fail.
// ==========================================================================================

/// An app whose agent registers the real `write`, `edit` and `bash` tools under a policy that
/// **asks** about them, plus the approval channel that ask goes to and a workspace on disk to
/// write into.
///
/// The ask-every-write rule is the point: the defaults allow `write`/`edit` inside the
/// workspace, so the gate would decide silently and no ask would ever be parked. Layering one
/// rule over the defaults is a policy a user can hold, and it makes the ask happen on the
/// product's own path — `exec::run_local` → the gate → `detail: ask_detail(...)` — rather than
/// a hand-built request.
///
/// The agent is constructed here rather than in `tests_support` because it is the only fixture
/// that needs file-mutating tools: every other test either wants an empty registry or wants
/// `bash`.
fn app_with_file_tools() -> (App, Arc<Agent>, ChannelApprovalUi, tempfile::TempDir) {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let (ftx, _frx) = tokio::sync::mpsc::unbounded_channel();
    let (ntx, _nrx) = tokio::sync::mpsc::unbounded_channel();
    let (otx, _orx) = tokio::sync::mpsc::unbounded_channel();
    let approvals = ChannelApprovalUi::new();
    let ui: Arc<dyn epsilon_safety::ApprovalUi> = Arc::new(approvals.clone());
    let engine = epsilon_safety::PermissionEngine::layered(vec![epsilon_safety::Ruleset {
        name: "ask-every-write".into(),
        rules: vec![
            epsilon_safety::Rule::new("write", "*", epsilon_safety::Action::Ask),
            epsilon_safety::Rule::new("edit", "*", epsilon_safety::Action::Ask),
        ],
    }]);
    let gate = Arc::new(epsilon_safety::RulesetGate::layered(engine, ui));
    let mut registry = epsilon_core::ToolRegistry::new();
    registry.register(Arc::new(epsilon_tools::WriteTool::new()));
    registry.register(Arc::new(epsilon_tools::EditTool::new()));
    registry.register(Arc::new(epsilon_tools::BashTool::new()));
    let agent = Agent::new(
        epsilon_core::LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(epsilon_llm::FauxProvider::new(
            vec![],
        ))),
        Arc::new(registry),
        gate,
        workspace.path().to_path_buf(),
    );
    (
        App::new("faux-write".into(), ftx, ntx, otx),
        agent,
        approvals,
        workspace,
    )
}

/// Park the ask a gated tool call produces, through the core's real ask path.
///
/// The call itself never completes: the gate holds it until somebody answers, which is the
/// product's behaviour and exactly the state the dialog has to cover. The task is left parked;
/// when the ask is dropped without a reply the channel resolves to deny and it ends.
///
/// Bounded, so a gate that answers silently fails the test rather than the suite.
async fn park_a_real_ask(
    app: &mut App,
    agent: &Arc<Agent>,
    approvals: &ChannelApprovalUi,
    name: &str,
    args: serde_json::Value,
) {
    let agent = Arc::clone(agent);
    let name = name.to_string();
    let call =
        tokio::spawn(async move { epsilon_core::exec::run_local(&agent, &name, args).await });
    let ask = tokio::time::timeout(std::time::Duration::from_secs(20), approvals.next_ask())
        .await
        .expect("the gated call parks an ask — a silent gate would mean no dialog to draw")
        .expect("a gated call asks");
    app.pending_ask = Some(ask);
    // The task must not be joined: it is blocked on the answer nobody gives, which is the
    // point. Keeping the handle alive is not needed for the frame.
    std::mem::forget(call);
}

/// The row a needle first appears on, or a panic naming what the frame did show.
fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|r| r.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} never reached the screen: {rows:#?}"))
}

#[tokio::test]
async fn a_write_approval_shows_the_new_content_as_added_lines() {
    let (mut app, agent, approvals, _workspace) = app_with_file_tools();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    // A file that does not exist yet: every line of the write is an addition, so the frame
    // must show the bytes the reader is authorising and nothing else.
    park_a_real_ask(
        &mut app,
        &agent,
        &approvals,
        "write",
        serde_json::json!({
            "path": "created.txt",
            "content": "first new line\nsecond new line\n"
        }),
    )
    .await;

    let rows = render_rows(&mut app, 80, 24);
    let screen = rows.join("\n");
    // The one-line answer still names the file, unchanged — a reader skimming gets the path.
    assert!(
        screen.contains("created.txt"),
        "the `what` row still names the file: {screen}"
    );
    // The diff row names the size of the change: two additions, no removals.
    assert!(
        screen.contains("2 added, 0 removed"),
        "the summary row counts the change: {screen}"
    );
    // The added lines themselves, each on its own row with its `+` sign beside its number —
    // the sign is in its own cell, so colour is never the only thing saying which is which.
    let first = row_of(&rows, "first new line");
    let second = row_of(&rows, "second new line");
    assert!(
        rows[first].contains('+') && rows[second].contains('+'),
        "each added row carries its sign: {:?} / {:?}",
        rows[first],
        rows[second]
    );
    assert!(
        first < second,
        "the added lines read in file order: {first} then {second}\n{screen}"
    );
    // **Nothing is claimed to be removed.** A write to a new file removes nothing, and a
    // frame that drew a `-` row would be describing a change that is not the change.
    assert!(
        !rows.iter().any(|r| r.trim_start().starts_with('-')),
        "a new file cannot remove a line: {screen}"
    );
    // And the decision keys are still on the screen, under the diff.
    assert!(
        second < row_of(&rows, "y allow once"),
        "the diff is read before the keys that act on it\n{screen}"
    );
}

#[tokio::test]
async fn an_edit_approval_shows_the_removed_line_then_the_added_one() {
    let (mut app, agent, approvals, workspace) = app_with_file_tools();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    std::fs::write(workspace.path().join("notes.txt"), "alpha\nbeta\ngamma\n")
        .expect("the fixture file");
    park_a_real_ask(
        &mut app,
        &agent,
        &approvals,
        "edit",
        serde_json::json!({
            "path": "notes.txt",
            "old_string": "beta",
            "new_string": "BETA"
        }),
    )
    .await;

    let rows = render_rows(&mut app, 80, 24);
    let screen = rows.join("\n");
    // The old line, the new line, and the removed one **first** — that is the shape a unified
    // diff reads in and the order the renderer owes the reader.
    let removed = row_of(&rows, "beta");
    let added = row_of(&rows, "BETA");
    assert!(
        removed < added,
        "the removed line precedes the added one: {removed} then {added}\n{screen}"
    );
    // Both carry their sign, so colour is never the only thing saying which is which.
    assert!(
        rows[removed].contains('-') && rows[added].contains('+'),
        "each row carries its own sign: {:?} / {:?}",
        rows[removed],
        rows[added]
    );
    // Context either side is drawn too: a diff with no context cannot be placed in the file.
    assert!(
        screen.contains("alpha") && screen.contains("gamma"),
        "the change is shown with its context: {screen}"
    );
    // ...and the two decision rows still sit under it.
    let keys = row_of(&rows, "y allow once");
    assert!(
        added < keys,
        "the diff is read before the keys that act on it: {added} then {keys}\n{screen}"
    );
}

#[tokio::test]
async fn an_approval_whose_tool_previews_nothing_is_the_six_row_dialog_it_was() {
    // The control on the whole feature: `bash` implements no `preview`, so its ask carries no
    // `preview` key and the dialog must be the one the golden frame pins — six rows, `cap`,
    // `what`, and the decision keys, with no `diff` row and no label. A tool that previews
    // nothing must not move the ask it was already making.
    let (mut app, agent, approvals, _workspace) = app_with_file_tools();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    park_a_real_ask(
        &mut app,
        &agent,
        &approvals,
        "bash",
        serde_json::json!({ "command": "echo hello" }),
    )
    .await;

    let rows = render_rows(&mut app, 80, 24);
    let screen = rows.join("\n");
    assert!(
        screen.contains("cap") && screen.contains("bash") && screen.contains("echo hello"),
        "the fields the dialog has always shown are still there: {screen}"
    );
    assert!(
        !screen.contains("diff"),
        "an ask with no preview grows no diff row: {screen}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("added,")),
        "and no summary row either: {screen}"
    );
    for key in [
        "y allow once",
        "a always (session)",
        "n deny",
        "e deny + guidance",
    ] {
        assert!(
            screen.contains(key),
            "the dialog must offer `{key}`: {screen}"
        );
    }
}
