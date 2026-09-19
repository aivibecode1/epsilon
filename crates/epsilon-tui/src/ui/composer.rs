//! The composer: where the user types.
//!
//! The old composer was a full box with a title that read
//! `input — ⚠ no key: /login <provider> pastes masked`, which is three problems
//! in one row: a border that adds nothing, a label the reader has to parse before
//! every keystroke, and a warning that has nothing to do with typing repeated on
//! every frame. This one is a prompt character, the draft, and a single rule
//! above it.
//!
//! ```text
//! ──────────────────────────────────────────────────────────────
//! ❯ ask anything
//! ```
//!
//! Two behaviours are non-negotiable, because both have bitten users:
//!
//! * **The caret is always visible.** When a long draft scrolls, the composer
//!   scrolls with it rather than showing the start of the text and hiding the
//!   cursor.
//! * **The height grows, then stops.** A pasted document must not push the
//!   conversation off the screen (see [`crate::ui::layout`]).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ui::theme::{Glyphs, Theme};

/// What the composer shows when the draft is empty.
pub const PLACEHOLDER: &str = "ask anything";

/// Draw the rule that separates the transcript from the composer.
///
/// A rule rather than a border: it costs one row instead of a box, and it reads
/// as a fold in the page instead of a form field.
pub fn draw_rule(f: &mut Frame<'_>, area: Rect, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    f.render_widget(
        Line::from(Span::styled(
            theme.glyphs.rule(area.width as usize),
            theme.border(),
        )),
        area,
    );
}

/// Draw the composer and place the terminal cursor.
///
/// `rows` is the draft already wrapped to `area.width - gutter`; `caret_row` and
/// `caret_col` locate the caret within it, so this function never has to
/// re-derive the layout arithmetic that the editor already did.
pub fn draw(
    f: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    rows: &[String],
    caret_row: usize,
    caret_col: usize,
    focused: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let glyphs: &Glyphs = &theme.glyphs;
    let gutter = gutter_width(glyphs);
    let text_width = (area.width as usize).saturating_sub(gutter);
    debug_assert!(caret_col <= text_width);

    let empty = rows.len() == 1 && rows[0].is_empty();
    // Scroll so the caret is on screen: showing the top of a long draft is what
    // makes a user think their typing was lost.
    let skip = caret_row.saturating_sub(area.height as usize - 1);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (index, text) in rows
        .iter()
        .enumerate()
        .skip(skip)
        .take(area.height as usize)
    {
        let is_first = index == 0;
        let prompt = if is_first { glyphs.bullet } else { " " };
        let prompt_style = if focused {
            theme.accent()
        } else {
            theme.faint()
        };
        let mut spans = vec![
            Span::styled(format!("{prompt} "), prompt_style),
            Span::raw(" ".repeat(gutter.saturating_sub(2))),
        ];
        if empty && is_first {
            spans.push(Span::styled(PLACEHOLDER, theme.faint()));
        } else {
            spans.push(Span::styled(text.clone(), theme.text()));
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), area);
    if focused {
        // The caret is a real terminal cursor, not a drawn block: a drawn block
        // cannot blink, and on Windows it survives a resize differently from the
        // cursor the terminal itself maintains.
        let x = area.x + (gutter + caret_col) as u16;
        let y = area.y + (caret_row - skip) as u16;
        if x < area.x + area.width && y < area.y + area.height {
            f.set_cursor_position((x, y));
        }
    }
}

/// Columns reserved before the text: the prompt glyph plus one space.
pub fn gutter_width(glyphs: &Glyphs) -> usize {
    glyphs.bullet.chars().count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{ASCII, ColorMode};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_rows(width: u16, rows: &[String], caret: (usize, usize)) -> String {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let mut terminal =
            Terminal::new(TestBackend::new(width, rows.len() as u16)).expect("test terminal");
        terminal
            .draw(|f| draw(f, f.area(), &theme, rows, caret.0, caret.1, true))
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    #[test]
    fn an_empty_composer_invites_input() {
        let screen = render_rows(40, &[String::new()], (0, 0));
        assert!(screen.contains(PLACEHOLDER), "{screen}");
    }

    #[test]
    fn a_draft_replaces_the_placeholder() {
        let screen = render_rows(40, &["hello".to_string()], (0, 5));
        assert!(screen.contains("hello"));
        assert!(!screen.contains(PLACEHOLDER));
    }

    #[test]
    fn a_long_draft_scrolls_to_keep_the_caret_visible() {
        // Three visible rows, caret on the last line: the first line must scroll
        // off rather than the caret going off screen.
        let rows: Vec<String> = (0..3).map(|i| format!("line {i}")).collect();
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let mut terminal = Terminal::new(TestBackend::new(40, 2)).expect("test terminal");
        terminal
            .draw(|f| draw(f, f.area(), &theme, &rows, 2, 3, true))
            .expect("draw");
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(screen.contains("line 2"), "caret row hidden: {screen}");
        assert!(!screen.contains("line 0"), "should have scrolled: {screen}");
    }

    #[test]
    fn the_rule_fills_its_row_exactly() {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("test terminal");
        terminal
            .draw(|f| draw_rule(f, f.area(), &theme))
            .expect("draw");
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert_eq!(screen, "-".repeat(20));
    }
}
