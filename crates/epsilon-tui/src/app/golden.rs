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

use super::tests_support::{park_ask, render_rows, test_app};
use super::*;

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
    // The rules that separate the three stacked regions, and only those. The
    // glyph comes from the theme the app resolved rather than from a literal,
    // because the repertoire is a runtime decision (and a console that cannot
    // render the Unicode rule gets the ASCII one).
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
        rules, 2,
        "expected two full-width rules made of {rule:?}, found {rules}"
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
