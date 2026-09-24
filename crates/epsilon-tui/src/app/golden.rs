//! Golden frames: the whole screen, drawn by the real draw path.
//!
//! # Why this file exists
//!
//! Every other TUI test here checks one widget in isolation, which is the right
//! unit for most of them and wrong for exactly one question: **what does the
//! screen actually look like**. A composer that renders perfectly on its own and
//! a footer that renders perfectly on its own can still be assembled into a frame
//! where the two overlap, where the composer has no room, or where the transcript
//! is squeezed to nothing — and every unit test would stay green.
//!
//! So these tests drive [`draw`] itself, through the same `TestBackend` a unit
//! test uses, and assert on the assembled screen. They are deliberately about
//! *structure* rather than pixel-exact text: a golden test that pins every
//! character fails on every deliberate copy change and gets rewritten without
//! being read, which is worse than no test. What is pinned here is the set of
//! properties a frame must have at every size — and the one property this module
//! exists for, which is that a row is never wider than the terminal.
//!
//! `render_rows` calls `draw(app)`, the production entry point. A test that
//! hand-assembled the shell would be testing the test.

use super::tests_support::{park_ask, render_buffer, render_rows, test_app};
use super::*;
use crate::ui::theme::{ColorMode as ThemeColorMode, Theme};

/// Pin the look so a golden frame can assert exact colours and glyphs.
///
/// `Theme::detect()` reads `NO_COLOR`, `COLORTERM` and `EPSILON_ASCII` from
/// whatever environment the test runner happens to have, which is right for the
/// product and wrong for a test that names a colour. The theme is a field of
/// `App`, so this sets it rather than re-detecting it.
fn pin_theme(app: &mut App, mode: ThemeColorMode) {
    app.theme = Theme::new(mode, crate::ui::theme::UNICODE);
}

/// One screen cell, for assertions that are about style rather than text.
fn cell(buffer: &ratatui::buffer::Buffer, x: u16, y: u16) -> &ratatui::buffer::Cell {
    &buffer[(x, y)]
}

/// A frame with a user turn, a tool call, a result and a few notices.
///
/// Built through the public state API on purpose: `push_line` is private to
/// `state`, and a test that reached into it would stop exercising the paths a
/// real session uses.
fn busy_app() -> App {
    let mut app = bench_app(UiState::new());
    app.ui.push_local_input("explain the layout module");
    for text in [
        "The layout module owns the spatial model.",
        "It splits the frame into a header, a transcript, a composer and a footer,",
        "and it decides what happens when the terminal is too small.",
    ] {
        app.ui.push_notice(text);
    }
    app.input = "and the composer?".into();
    app.cursor_chars = app.input.chars().count();
    app
}

#[test]
fn a_full_frame_is_exactly_the_terminal_size_at_every_supported_width() {
    // The property this module exists for. A row wider than the terminal wraps,
    // which turns a one-line bar into two and shifts everything below it — the
    // class of bug that makes a TUI look broken rather than merely plain.
    for (width, height) in [(44u16, 10u16), (60, 20), (80, 24), (100, 30), (160, 48)] {
        let mut app = busy_app();
        let rows = render_rows(&mut app, width, height);
        assert_eq!(
            rows.len(),
            height as usize,
            "{width}x{height}: wrong row count"
        );
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                row.chars().count(),
                width as usize,
                "{width}x{height} row {index} is {} columns: {row:?}",
                row.chars().count()
            );
        }
    }
}

#[test]
fn the_frame_shows_the_four_things_it_is_for() {
    // Header, transcript, composer, footer — each present and identifiable at a
    // size a developer actually uses. If a change squeezes one out at 80x24, that
    // is a regression no unit test would have caught.
    let mut app = busy_app();
    let rows = render_rows(&mut app, 80, 24);
    let screen = rows.join("\n");

    assert!(
        rows[0].contains("idle"),
        "no header on row 0: {:?}",
        rows[0]
    );
    assert!(
        screen.contains("The layout module owns the spatial model."),
        "the transcript is missing its content: {screen}"
    );
    assert!(
        screen.contains("and the composer?"),
        "the composer lost the draft: {screen}"
    );
    assert!(
        rows[rows.len() - 2..].iter().any(|r| r.contains("enter")),
        "no key hints at the bottom: {:?}",
        &rows[rows.len() - 2..]
    );
    // The rule that separates the transcript from the composer, and only that
    // one. There used to be two: a second full-width rule sat between the header
    // and the transcript, drawn by the same function in the same style, so
    // neither line said what it divided — and at 80x24 the pair was 2 of 24 rows.
    // The count below is pinned deliberately: moving it from 2 to 1 is the
    // reviewable evidence that cutting the rule was intended, and that the row it
    // held went to the transcript rather than going missing. The glyph comes from
    // the theme the app resolved rather than from a literal, because the
    // repertoire is a runtime decision (and a console that cannot render the
    // Unicode rule gets the ASCII one).
    let rule = app
        .theme()
        .glyphs
        .horizontal
        .chars()
        .next()
        .expect("a horizontal glyph");
    let rules = rows
        .iter()
        .filter(|r| !r.is_empty() && r.chars().all(|c| c == rule))
        .count();
    assert_eq!(
        rules, 1,
        "expected one full-width rule made of {rule:?}, found {rules}: {screen}"
    );
}

#[test]
fn a_long_transcript_never_pushes_the_composer_off_the_screen() {
    // A long conversation scrolls; it does not grow. If the transcript claimed
    // more rows than it has, the composer and footer would be laid out past the
    // bottom of the frame and the user could not type.
    let mut app = bench_app(UiState::new());
    for index in 0..200 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    let rows = render_rows(&mut app, 60, 20);
    let screen = rows.join("\n");
    assert!(screen.contains("ROW-199"), "the tail must be visible");
    // The composer and the footer are the last two rows whatever the transcript
    // does. Asserting on the footer's hints rather than on the composer's marker
    // is deliberate: the marker scrolls with the draft, and a test that pinned
    // its row would fail on a one-row change to the composer.
    assert!(
        rows[rows.len() - 2..]
            .iter()
            .any(|r| r.contains("enter") || r.contains("ask anything")),
        "the composer and footer were pushed off the frame: {:?}",
        &rows[rows.len() - 4..]
    );
}

#[test]
fn below_the_floor_the_frame_says_so_instead_of_drawing_a_broken_layout() {
    // A 40x8 window cannot hold a conversation. The failure mode to avoid is not
    // "ugly" — it is a frame with a composer but no room for the transcript, so
    // the user types into something they cannot read.
    let mut app = busy_app();
    let rows = render_rows(&mut app, 40, 8);
    let screen = rows.join("\n");
    assert!(screen.contains("too small"), "{screen}");
    assert!(
        !screen.contains("The layout module owns"),
        "a frame below the floor must not draw a partial transcript: {screen}"
    );
}

#[test]
fn resizing_mid_session_keeps_every_row_inside_the_new_width() {
    // Resize is the case where cached coordinates go stale. Drawing wide, then
    // narrow, then wide again must produce well-formed frames each time — the
    // renderer holds no width of its own.
    let mut app = busy_app();
    for (width, height) in [(120u16, 30u16), (50, 14), (120, 30), (44, 10)] {
        let rows = render_rows(&mut app, width, height);
        assert_eq!(rows.len(), height as usize);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                row.chars().count(),
                width as usize,
                "after resizing to {width}x{height}, row {index} is {} columns",
                row.chars().count()
            );
        }
    }
}

#[test]
fn an_empty_session_gets_the_first_run_screen_inside_a_well_formed_frame() {
    // The first frame a new user sees. It must be the welcome screen — a blank
    // body reads as a hung program — and it must still be a frame, not a wall of
    // text with no composer.
    let mut app = bench_app(UiState::new());
    let rows = render_rows(&mut app, 100, 30);
    let screen = rows.join("\n");
    assert!(screen.contains("E P S I L O N"), "{screen}");
    assert!(
        screen.contains("ask anything"),
        "the composer placeholder is missing: {screen}"
    );
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.chars().count(), 100, "row {index} is malformed");
    }
}

#[tokio::test]
async fn a_pending_approval_is_visible_in_a_well_formed_frame() {
    // The one frame where being wrong has consequences. An ask parked on the
    // shared queue must reach the screen: the user is being asked to authorize
    // work, and a dialog that is laid out but not drawn would still answer keys.
    let (mut app, _agent, _receivers) = test_app();
    let approvals = ChannelApprovalUi::new();
    let _response = park_ask(&mut app, &approvals, "req-golden").await;

    let rows = render_rows(&mut app, 90, 26);
    let screen = rows.join("\n");
    assert!(screen.contains("permission required"), "{screen}");
    assert!(screen.contains("rm -rf build"), "{screen}");
    assert!(screen.contains('y') && screen.contains('n'), "{screen}");
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            row.chars().count(),
            90,
            "the approval frame is malformed on row {index}: {row:?}"
        );
    }
}

// ---- the transcript's decoration, seen on a real frame -----------------------
//
// The tests in `ui::transcript` call `decorate` directly, and that is precisely
// how the whole decoration system came to be dead code: they proved the function
// worked while nothing on screen called it. The two tests below go through
// `draw(app)` — the production entry point — and read the *cells*, because a band
// and a gutter glyph's colour are invisible to a text-only assertion.

#[test]
fn a_user_turn_carries_its_gutter_and_band_on_a_real_frame() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Truecolor);
    let surface = app.theme.surface_user().bg;
    let user_style = app.theme.user();
    app.ui
        .push_local_input("what does the gutter look like now?");

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    let row_index = rows
        .iter()
        .position(|r| r.contains("what does the gutter look like now?"))
        .unwrap_or_else(|| panic!("the user turn never reached the screen: {rows:#?}"));
    let row = &rows[row_index];

    // 1. The two-column gutter, and the `▎` that says "this is the person
    //    talking" in the leftmost cell.
    assert!(
        row.starts_with("▎ "),
        "user row {row_index} has no gutter marker: {row:?}"
    );
    assert_eq!(cell(&buffer, 0, row_index as u16).symbol(), "▎");
    assert_eq!(
        cell(&buffer, 0, row_index as u16).style().fg,
        user_style.fg,
        "the gutter glyph must carry the user role's colour"
    );
    // 2. The band, at the glyph, behind the text, and out to the terminal edge.
    for x in [0u16, 2, 20, 79] {
        assert_eq!(
            cell(&buffer, x, row_index as u16).style().bg,
            surface,
            "column {x} of the user row is not on the user surface"
        );
    }
    // 3. Exactly one marker. `❯` from the render cache; the `"> "` that
    //    `push_local_input` used to store is gone, so the row reads
    //    `▎ ❯ what does …` and not `▎ ❯ > what does …`.
    assert!(
        row.contains("▎ ❯ what does"),
        "the user row must carry one marker, not two: {row:?}"
    );
    // 4. The band belongs to that row and nothing else: a blank line above it is
    //    not painted as part of the message.
    assert_ne!(
        cell(&buffer, 20, row_index as u16 - 1).style().bg,
        surface,
        "the band leaked onto the row above the message"
    );
    // 5. And the frame is still exactly the terminal's width — a band that
    //    overflows is a wrapped row, which is what the one column of slack used
    //    to be buying.
    for (index, r) in rows.iter().enumerate() {
        assert_eq!(
            r.chars().count(),
            80,
            "row {index} is {} columns: {r:?}",
            r.chars().count()
        );
    }
}

#[test]
fn a_wrapped_banded_message_keeps_every_row_inside_the_frame() {
    // The width trap the audit named: the cache wraps the BODY to
    // `frame - GUTTER` and `decorate` pads to the FRAME width. Wrap to the frame
    // width instead and every banded row is two cells over the edge — the row
    // wraps, one message becomes two rows and everything below it shifts. This is
    // the production path, at a realistic narrow width.
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Truecolor);
    let surface = app.theme.surface_user().bg;
    app.ui.push_local_input(
        "the user turn is the one row that must never wrap its band past the terminal \
         edge, because a band that overflows turns one message into two rows and moves \
         everything under it",
    );

    let (buffer, rows) = render_buffer(&mut app, 60, 20);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            row.chars().count(),
            60,
            "row {index} is {} columns: {row:?}",
            row.chars().count()
        );
    }
    let banded: Vec<usize> = (0..rows.len())
        .filter(|&y| cell(&buffer, 59, y as u16).style().bg == surface)
        .collect();
    assert!(
        banded.len() >= 3,
        "a wrapped user turn must band every one of its rows, got {banded:?}: {rows:#?}"
    );
    assert_eq!(
        banded,
        (banded[0]..banded[0] + banded.len()).collect::<Vec<_>>(),
        "the banded rows must be contiguous"
    );
    // One marker, on the first row only: a `❯` repeated on every continuation row
    // would read as a new message per line.
    assert!(
        rows[banded[0]].starts_with("▎ ❯ the user turn"),
        "the first banded row must carry the gutter and the one marker: {:?}",
        rows[banded[0]]
    );
    for index in banded.iter().skip(1) {
        assert!(
            !rows[*index].contains('❯'),
            "continuation row {index} repeated the marker: {:?}",
            rows[*index]
        );
    }
}

#[test]
fn a_tool_call_and_a_notice_are_distinguishable_on_a_real_frame_without_colour() {
    // `NO_COLOR`: no color at all, and no filled bands — `surface_*` returns the
    // default style on anything but truecolor — so glyph and position are all
    // that is left. A pending call and a lifecycle notice used to share
    // `glyphs.bullet`, in every colour mode: `• …` and `• …`, the same pixels.
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::None);
    // Built through `apply`, the event reducer a real session feeds: a tool-call
    // row only exists because an assistant message asked for a tool.
    app.ui
        .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(
                epsilon_contracts::message::AssistantMessage {
                    id: "msg_golden_call".into(),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                        id: "tc_golden".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({ "command": "cargo test -p epsilon-tui" }),
                    }],
                    stop_reason: epsilon_contracts::message::StopReason::ToolUse,
                    error_message: None,
                    usage: epsilon_contracts::message::Usage::default(),
                },
            )),
        });
    app.ui.push_notice("run finished · 3 files changed");

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    assert!(
        !app.theme.has_surfaces(),
        "NO_COLOR must not claim filled bands"
    );
    let call_row = rows
        .iter()
        .position(|r| r.contains("tool-call bash cargo test"))
        .unwrap_or_else(|| panic!("the tool call never reached the screen: {rows:#?}"));
    let notice_row = rows
        .iter()
        .position(|r| r.contains("run finished"))
        .unwrap_or_else(|| panic!("the notice never reached the screen: {rows:#?}"));

    assert!(
        rows[call_row].starts_with("▸ ▸ "),
        "tool-call row {call_row} must carry the call gutter and marker: {:?}",
        rows[call_row]
    );
    assert!(
        rows[notice_row].starts_with("• • "),
        "notice row {notice_row} must carry the notice gutter and marker: {:?}",
        rows[notice_row]
    );
    assert_ne!(
        cell(&buffer, 0, call_row as u16).symbol(),
        cell(&buffer, 0, notice_row as u16).symbol(),
        "with colour off, two roles that share a gutter glyph are the same pixels"
    );
    assert_eq!(
        cell(&buffer, 0, call_row as u16).style().fg,
        Some(ratatui::style::Color::Reset),
        "NO_COLOR must not emit colour: {:?}",
        cell(&buffer, 0, call_row as u16).style()
    );
    // The band is withheld, as it must be, and the row still says what it is: the
    // marker is in the body text, not only in a colour that is gone.
    assert_eq!(
        cell(&buffer, 0, call_row as u16).style().bg,
        Some(ratatui::style::Color::Reset),
        "a banded call on a no-surface theme would be an approximation"
    );
    assert!(
        rows[call_row].contains("▸ tool-call bash"),
        "the role marker must survive the loss of colour: {:?}",
        rows[call_row]
    );
}

#[test]
fn a_pending_tool_call_gets_the_pending_band_where_the_terminal_has_one() {
    // The other half of the same decoration: on a terminal that can show filled
    // bands, the call is on `surface_pending()`. This is the negative control for
    // the `LineKind` part of the change — drop the kind on the way out of the
    // cache and the row loses its band here, while the marker assertions in
    // `render_cache` (which read the cache directly) stay green.
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Truecolor);
    let pending = app.theme.surface_pending().bg;
    let ok = app.theme.surface_ok().bg;
    assert_ne!(pending, ok, "pending and ok must not share a surface");
    app.ui
        .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(
                epsilon_contracts::message::AssistantMessage {
                    id: "msg_golden_pending".into(),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                        id: "tc_pending".into(),
                        name: "edit".into(),
                        arguments: serde_json::json!({ "path": "src/lib.rs" }),
                    }],
                    stop_reason: epsilon_contracts::message::StopReason::ToolUse,
                    error_message: None,
                    usage: epsilon_contracts::message::Usage::default(),
                },
            )),
        });

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    let call_row = rows
        .iter()
        .position(|r| r.contains("tool-call edit"))
        .unwrap_or_else(|| panic!("the tool call never reached the screen: {rows:#?}"));
    assert!(
        rows[call_row].starts_with("▸ ▸ "),
        "the call must carry its gutter marker: {:?}",
        rows[call_row]
    );
    for x in [0u16, 2, 40, 79] {
        assert_eq!(
            cell(&buffer, x, call_row as u16).style().bg,
            pending,
            "column {x} of the pending call is not on the pending surface: {:?}",
            cell(&buffer, x, call_row as u16).style()
        );
    }
    // Bands belong to the rows that carry them: the blank rows around the call
    // are not painted as part of it.
    assert_ne!(
        cell(&buffer, 40, call_row as u16 + 1).style().bg,
        pending,
        "the band leaked onto the row below the call"
    );
}

#[test]
fn the_advertised_scroll_key_scrolls_the_real_frame() {
    // The footer now advertises PageUp/PageDown. A hint is a promise about what
    // a key *does*, so the promise is checked through `handle_key` and the
    // production draw path rather than against the handler's own source: if the
    // footer naming the key were the only change, this fails with the transcript
    // still showing its tail.
    let (mut app, agent, _receivers) = test_app();
    // The chord comes from the binding table, which is where the footer got it.
    let pageup = app
        .keybindings
        .key_text("app.transcript.pageUp")
        .expect("the scroll key must resolve, or the footer is advertising nothing");
    assert_eq!(pageup, "pageup", "the default scroll chord moved");

    for index in 0..60 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    let before = render_rows(&mut app, 60, 20);
    assert!(before.join("\n").contains("ROW-059"), "{before:#?}");
    assert_eq!(app.scroll_from_bottom, 0, "a fresh frame follows the tail");

    assert!(!super::keys::handle_key(
        &mut app,
        &agent,
        KeyEvent {
            code: KeyCode::PageUp,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    ));
    let screen = render_rows(&mut app, 60, 20).join("\n");
    assert!(
        !screen.contains("ROW-059"),
        "the frame still shows the tail after {pageup}: the advertised key did not scroll"
    );
    assert!(screen.contains("ROW-"), "the transcript vanished: {screen}");
}

// ---- the approval dialog: its size, and the colour it says it uses -----------
//
// `draw_dialog`'s own doc comment claimed a danger treatment while the code
// passed `focused: true` and got the same accent every other overlay uses, and
// the box was eleven rows for a yes/no where the layout reference asks 5–7. Both
// are properties of the drawn frame, so both are read back from the drawn frame:
// a text-only assertion cannot see a border colour, and a test that asked
// `draw_dialog` for its height would be asking the code under test to grade
// itself.

/// The columns the dialog's frame occupies, across every row it is drawn on.
///
/// Found by looking for the colour the frame is supposed to have. Scanning a
/// single row for "the first styled cell" finds the *transcript* beside the
/// dialog instead — the panel is centred, so the columns around it are not
/// blank, they are the screen it is covering — and the top row is interrupted by
/// the title, which is drawn in the title's style. Taking the extremes over all
/// the frame's rows gives the rectangle's edges whatever the title does.
fn frame_columns(
    buffer: &ratatui::buffer::Buffer,
    rows: &[u16],
    width: u16,
    colour: ratatui::style::Color,
) -> (u16, u16) {
    let mut left = u16::MAX;
    let mut right = 0u16;
    for &row in rows {
        for x in 0..width {
            if buffer[(x, row)].style().fg == Some(colour) {
                left = left.min(x);
                right = right.max(x);
            }
        }
    }
    assert!(
        left != u16::MAX,
        "none of the rows {rows:?} carries the dialog's colour: it is not drawn in it"
    );
    assert!(
        left < right,
        "the frame's edges must be apart: {left}..={right}"
    );
    (left, right)
}

#[tokio::test]
async fn the_approval_dialog_is_six_rows_and_wears_the_danger_border() {
    let (mut app, _agent, _receivers) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    let approvals = ChannelApprovalUi::new();
    let _response = park_ask(&mut app, &approvals, "req-shape").await;

    let (buffer, rows) = render_buffer(&mut app, 100, 24);
    let screen = rows.join("\n");
    let danger = app.theme.danger().fg.expect("the danger role has a colour");
    let focus = app
        .theme
        .border_focus()
        .fg
        .expect("the focus role has a colour");
    assert_ne!(
        danger, focus,
        "the two accents must differ for this test to mean anything"
    );

    // 1. The frame is the danger colour and not the focus accent, on every one
    //    of its edges. This is the assertion the pre-fix dialog failed: it drew
    //    `border_focus()` and claimed otherwise in its doc comment.
    let danger_rows: Vec<u16> = (0..24)
        .filter(|&y| (0..100u16).any(|x| buffer[(x, y)].style().fg == Some(danger)))
        .collect();
    assert_eq!(
        danger_rows.len(),
        6,
        "the permission dialog must be six rows tall — two frame edges and four \
         of body — all of them in the danger accent. Found {danger_rows:?}; one \
         row means the frame is drawn in some other colour and only the `n deny` \
         key is danger, which is what the border_focus() version looked like:\n{screen}"
    );
    assert_eq!(
        danger_rows,
        (danger_rows[0]..danger_rows[0] + 6).collect::<Vec<u16>>(),
        "the dialog's rows must be contiguous: {danger_rows:?}"
    );
    let (left, right) = frame_columns(&buffer, &danger_rows, 100, danger);
    let top = danger_rows[0];
    let bottom = danger_rows[5];
    // The corners are the frame, one at each end of each edge.
    for (row, name) in [(top, "top"), (bottom, "bottom")] {
        for x in [left, right] {
            assert_eq!(
                buffer[(x, row)].style().fg,
                Some(danger),
                "the {name} edge's corner at column {x} is not in the danger accent: {:?}",
                rows[row as usize]
            );
        }
    }
    // The side rails too: a frame is four edges, and a test that read only the
    // top and bottom would pass for a dialog drawn with the wrong sides.
    for row in &danger_rows[1..5] {
        for x in [left, right] {
            assert_eq!(
                buffer[(x, *row)].style().fg,
                Some(danger),
                "column {x} of row {row} must be a danger-coloured rail: {:?}",
                rows[*row as usize]
            );
        }
    }
    // And the rectangle is the width the prompt layout asks for, not a stray
    // band: `Width::PROMPT` is three fifths of the screen, floored at 46 — at 100
    // columns the share wins; the floor is what a narrow terminal keeps.
    assert_eq!(
        right - left + 1,
        100 * 3 / 5,
        "the dialog's width must be the prompt width: {left}..={right}"
    );

    // 2. The body still says everything a decision needs, on those six rows.
    assert!(screen.contains("permission required"), "{screen}");
    assert!(screen.contains("cap"), "{screen}");
    assert!(screen.contains("bash"), "{screen}");
    assert!(
        screen.contains("rm -rf build"),
        "the detail line must show what will run: {screen}"
    );
    assert!(
        !screen.contains(r#"{"command""#),
        "the detail line must be the command, not the JSON object holding it: {screen}"
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
    // 3. Nothing below the dialog is the dialog: the box does not bleed.
    assert!(
        rows[danger_rows[5] as usize + 1..]
            .iter()
            .all(|r| !r.contains("permission required")),
        "the dialog leaked past its bottom edge: {screen}"
    );
}

#[test]
fn a_detail_object_prints_the_field_that_decides_the_question() {
    // The dialog's one row of detail is worth more as the command than as the
    // object containing it: on the PROMPT width the JSON's punctuation is most of
    // what fits.
    assert_eq!(
        summarize_detail(&serde_json::json!({ "command": "rm -rf build" })),
        "rm -rf build"
    );
    assert_eq!(
        summarize_detail(&serde_json::json!({ "path": "src/lib.rs", "offset": 40 })),
        "src/lib.rs"
    );
    assert_eq!(
        summarize_detail(&serde_json::json!({ "query": "openai" })),
        "openai"
    );
    assert_eq!(
        summarize_detail(&serde_json::json!("plain text")),
        "plain text"
    );
    // No known field: the object itself, truncated, rather than a guess.
    let other = summarize_detail(&serde_json::json!({ "count": 3 }));
    assert!(other.contains("count"), "{other}");
    let long = summarize_detail(&serde_json::json!({ "unknown": "x".repeat(400) }));
    assert!(
        long.ends_with('…'),
        "a long detail must be truncated: {long}"
    );
    assert!(long.chars().count() <= 97, "{} chars", long.chars().count());
}

#[test]
fn the_one_rule_sits_between_the_transcript_and_the_composer() {
    // Where the surviving rule lives, on a frame whose transcript is full. The
    // old layout drew this row AND a second rule under the header; the point of
    // cutting one is that this one keeps its job — "everything below this line
    // takes keystrokes" — and the row goes to the conversation.
    let mut app = bench_app(UiState::new());
    for index in 0..40 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    let rows = render_rows(&mut app, 80, 24);
    let rule: char = app
        .theme()
        .glyphs
        .horizontal
        .chars()
        .next()
        .expect("a horizontal glyph");
    let is_rule = |r: &String| !r.is_empty() && r.chars().all(|c| c == rule);
    let rule_rows: Vec<usize> = (0..rows.len()).filter(|&i| is_rule(&rows[i])).collect();
    assert_eq!(
        rule_rows.len(),
        1,
        "there must be one rule, not two: {rows:#?}"
    );
    let rule_row = rule_rows[0];
    // The rule is the line under the composer, and the transcript runs INTO the
    // composer with nothing between them: under the old layout a second rule sat
    // there, at 80x24 on row 19, and this frame's transcript ended above it.
    let composer_row = rule_row - 1;
    assert!(
        rows[composer_row].contains("ask anything"),
        "the one rule must sit directly below the composer, found it on row \
         {rule_row} with {:?} above it: {rows:#?}",
        rows[composer_row]
    );
    assert!(
        rows[composer_row - 1].contains("ROW-"),
        "the transcript must reach the composer with no rule between them: {:?}",
        rows[composer_row - 1]
    );
}
