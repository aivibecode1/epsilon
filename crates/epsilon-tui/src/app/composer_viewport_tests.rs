//! Composer layout and transcript viewport.
//!
//! Lives beside `app.rs` rather than inside it so the runtime file reads as the
//! code it is. Rust still compiles these as part of the `app` module, so
//! `super::*` reaches every private item — a test forced through a public API
//! would be testing a different thing.

use super::*;
use crate::state::UiState;
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn streaming_tail_is_visible_after_full_transcript() {
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let mut app = bench_app(UiState::new());
    for index in 0..100 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    app.ui.apply(&AgentEvent::MessageUpdate {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "stream-test".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::text(format!(
                "{}LATEST",
                "live answer\n".repeat(100)
            ))],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Usage::default(),
        })),
    });
    let mut terminal = Terminal::new(TestBackend::new(60, 5)).unwrap();
    terminal
        .draw(|frame| draw_transcript(frame, &mut app, frame.area()))
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
        .collect();
    assert!(screen.contains("LATEST"), "stream hidden: {screen}");
}

#[test]
fn long_transcript_renders_latest_visible_rows() {
    let mut app = bench_app(UiState::new());
    for index in 0..100 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    let mut terminal = Terminal::new(TestBackend::new(60, 5)).unwrap();
    terminal
        .draw(|frame| draw_transcript(frame, &mut app, frame.area()))
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
        .collect();
    assert!(screen.contains("ROW-099"), "tail disappeared: {screen}");
    assert!(screen.contains("ROW-095"));
}

#[test]
fn wrapped_draft_grows_composer_at_supported_widths() {
    for (width, height) in [(60, 20), (80, 24), (120, 30), (160, 40)] {
        let mut app = bench_app(UiState::new());
        app.input = "§".repeat(width as usize + 8);
        app.cursor_chars = app.input.chars().count();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert_eq!(
            screen.matches('§').count(),
            app.input.chars().count(),
            "draft clipped at {width}x{height}"
        );
    }
}

#[test]
fn undersized_terminal_has_explicit_fallback() {
    let mut app = bench_app(UiState::new());
    let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
        .collect();
    assert!(screen.contains("terminal too small"));
}

#[test]
fn long_unicode_draft_wraps_and_places_caret_in_display_columns() {
    use ratatui::backend::Backend;
    let mut app = bench_app(UiState::new());
    app.input = "界".repeat(35);
    app.cursor_chars = 35;
    let theme =
        crate::ui::theme::Theme::new(crate::ui::theme::ColorMode::Ansi16, crate::ui::theme::ASCII);
    let width = 60;
    let text_width = crate::ui::shell::composer_text_width(width, &theme.glyphs);
    let (rows, caret_row, caret_col) = composer_rows(&app.input, app.cursor_chars, text_width);
    let mut terminal = Terminal::new(TestBackend::new(width, 5)).unwrap();
    terminal
        .draw(|frame| {
            crate::ui::composer::draw(
                frame,
                frame.area(),
                &theme,
                &rows,
                caret_row,
                caret_col,
                true,
            );
        })
        .unwrap();
    // The caret sits in *display* columns: each wide glyph is two cells, so
    // counting scalars would put it half a line away from the text.
    let caret = terminal.backend_mut().get_cursor_position().unwrap();
    assert_eq!(
        usize::from(caret.y),
        caret_row,
        "caret on the wrong draft row"
    );
    assert_eq!(
        usize::from(caret.x),
        crate::ui::composer::gutter_width(&theme.glyphs) + caret_col
    );
    let glyph = terminal.backend().buffer()[(caret.x - 2, caret.y)]
        .symbol()
        .to_string();
    assert_eq!(glyph, "界", "the caret is not next to the text it edits");
}

#[test]
fn editing_start_of_long_draft_keeps_caret_line_visible() {
    let mut app = bench_app(UiState::new());
    app.input = "FIRST\nsecond\nthird\nfourth\nfifth\nLAST".into();
    app.cursor_chars = 2;
    let theme =
        crate::ui::theme::Theme::new(crate::ui::theme::ColorMode::Ansi16, crate::ui::theme::ASCII);
    let width = 60;
    let text_width = crate::ui::shell::composer_text_width(width, &theme.glyphs);
    let (rows, caret_row, caret_col) = composer_rows(&app.input, app.cursor_chars, text_width);
    let mut terminal = Terminal::new(TestBackend::new(width, 5)).unwrap();
    terminal
        .draw(|frame| {
            crate::ui::composer::draw(
                frame,
                frame.area(),
                &theme,
                &rows,
                caret_row,
                caret_col,
                true,
            );
        })
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
        .collect();
    assert!(
        screen.contains("FIRST"),
        "the line being edited scrolled out of view: {screen}"
    );
}
