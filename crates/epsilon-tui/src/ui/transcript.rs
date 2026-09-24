//! How the conversation is presented.
//!
//! The difference between a terminal UI that looks considered and one that looks
//! like a 1980s BBS is almost never the widget toolkit. It is this:
//!
//! | Dated | Considered |
//! |---|---|
//! | A box around every message | One filled band, or a two-column gutter |
//! | `user:` / `assistant:` labels in caps | Position, glyph, and colour carry the role |
//! | Tool calls as JSON on one long line | A card: name, status, duration, then the detail |
//! | Every row the same weight | Speaker bright, payload dim, metadata faint |
//!
//! So: the **gutter** is the primary device. Two columns on the left say who is
//! speaking and, for tools, whether the call succeeded. Colour reinforces it but
//! never carries it alone — a reader on a 16-colour terminal, or with `NO_COLOR`
//! set, must still be able to follow the conversation.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::state::LineKind;
use crate::ui::theme::Theme;

/// Columns reserved at the left of every transcript row.
pub const GUTTER: usize = 2;

/// The glyph and body style for one transcript row.
///
/// Returns `(gutter_glyph, gutter_style, body_style, band)`, where `band` is the
/// background fill (if the terminal has colour to spare).
pub fn row_style(theme: &Theme, kind: LineKind) -> (String, Style, Style, Style) {
    let glyphs = &theme.glyphs;
    match kind {
        // The user's own words: a filled band plus a caret glyph, because a
        // question and an answer must be distinguishable at a glance when
        // scrolling back through a long session.
        LineKind::User => (
            glyphs.cursor_bar.to_string(),
            theme.user(),
            theme.text(),
            theme.surface_user(),
        ),
        // The model's prose is the default surface: no marker, no band. It is
        // what the page is *for*, so it should not be decorated.
        LineKind::Assistant => (
            " ".to_string(),
            theme.assistant(),
            theme.assistant(),
            Style::default(),
        ),
        // Reasoning is background material and says so.
        LineKind::Thinking => (
            glyphs.vertical.to_string(),
            theme.faint(),
            theme.thinking(),
            Style::default(),
        ),
        LineKind::ToolCall => (
            glyphs.tool_call.to_string(),
            theme.warn(),
            theme.warn(),
            theme.surface_pending(),
        ),
        LineKind::ToolResultOk => (
            glyphs.tick.to_string(),
            theme.success(),
            theme.dim(),
            theme.surface_ok(),
        ),
        LineKind::ToolResultErr => (
            glyphs.cross.to_string(),
            theme.danger(),
            theme.danger(),
            theme.surface_err(),
        ),
        // A lifecycle notice is not a tool call. They shared `glyphs.bullet`, so
        // on a terminal with no surfaces — or with `NO_COLOR` set — a pending call
        // and a notice were the same pixels. The bullet is the notice's now; the
        // call owns `glyphs.tool_call`, and the row's own marker repeats it in the
        // body position so the distinction survives at a glance.
        LineKind::System => (
            glyphs.bullet.to_string(),
            theme.faint(),
            theme.faint(),
            Style::default(),
        ),
    }
}

/// Wrap one already-rendered transcript row in its gutter.
///
/// The render cache holds the *body* (markdown expanded, wrapped, styled); this
/// adds the two-column gutter and, where the terminal supports it, the band that
/// runs to the right edge. Doing it here rather than in the cache keeps the cache
/// independent of the theme, so switching colour mode does not invalidate it.
///
/// `width` is the **whole row**, not the body: a band has to reach the terminal
/// edge to read as a block of speech rather than as a highlight on the text, so
/// the fill is `width - gutter - body`. Callers therefore wrap the body to
/// `width - GUTTER` and pass the frame width here; wrapping to `width` and then
/// prefixing two columns is how a transcript row ends up over the edge and wraps.
///
/// This is the production path: `app::draw_transcript` runs every visible row
/// through it. It used to be called only by this module's tests, which is exactly
/// how the gutter and the bands came to be missing from the screen while the
/// suite reported them covered.
pub fn decorate(
    theme: &Theme,
    kind: LineKind,
    body: Vec<Span<'static>>,
    width: usize,
) -> Line<'static> {
    let (glyph, gutter_style, _body_style, band) = row_style(theme, kind);
    let mut spans = Vec::with_capacity(body.len() + 2);
    // Measured, not `format!("{glyph:<GUTTER$}")`: a width counted in `char`s is
    // wrong the moment a repertoire holds a double-width glyph, and the gutter is
    // the one column every row of the transcript aligns on.
    let glyph_width = crate::ui::layout::display_width(&glyph);
    spans.push(Span::styled(
        format!("{glyph}{}", " ".repeat(GUTTER.saturating_sub(glyph_width))),
        gutter_style,
    ));
    spans.extend(body);
    if band != Style::default() {
        // The band has to reach the right edge, or it reads as a highlight on
        // the text rather than as a block of speech.
        let used: usize = spans
            .iter()
            .map(|s| crate::ui::layout::display_width(&s.content))
            .sum();
        if used < width {
            spans.push(Span::styled(" ".repeat(width - used), band));
        }
        // Re-apply the band to every span so the fill is continuous behind the
        // body's own colours.
        for span in spans.iter_mut() {
            span.style = span.style.patch(band);
        }
    }
    Line::from(spans)
}

/// The first-run screen.
///
/// A blank transcript on launch is the single worst first impression a TUI can
/// make: the user cannot tell whether the program is working, waiting, or broken.
/// This says what the harness is, what to type, and which keys matter — and it
/// disappears the moment there is a real conversation to show.
pub fn welcome(theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let g = &theme.glyphs;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let title = "EPSILON";
    // A plain, wide-tracked wordmark. Not ASCII art: art breaks on a narrow
    // terminal and reads as noise on a re-read.
    let tracked = title
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(tracked, theme.accent())));
    lines.push(Line::from(Span::styled(
        "a coding agent in your terminal",
        theme.dim(),
    )));
    lines.push(Line::from(""));
    let hint = |key: &str, what: &str| {
        Line::from(vec![
            Span::styled(format!("  {:<10}", key), theme.dim_strong()),
            Span::styled(what.to_string(), theme.dim()),
        ])
    };
    lines.push(hint("type", "describe a task and press enter"));
    lines.push(hint("/", "commands — try /help, /models, /sessions"));
    lines.push(hint("@", "reference a file by name"));
    lines.push(hint("ctrl-p", "command palette"));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  this session runs against the model shown at the top right",
        theme.faint(),
    )));
    let _ = (g, width);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{ASCII, ColorMode, UNICODE};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen(lines: Vec<Line<'static>>, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal
            .draw(|f| {
                f.render_widget(ratatui::widgets::Paragraph::new(lines), f.area());
            })
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
    fn every_role_is_distinguishable_without_colour() {
        // The acid test: colour off, ASCII glyphs. Each role must still produce a
        // different left margin.
        let theme = Theme::new(ColorMode::None, ASCII);
        let mut gutters = Vec::new();
        for kind in [
            LineKind::User,
            LineKind::Assistant,
            LineKind::Thinking,
            LineKind::ToolCall,
            LineKind::ToolResultOk,
            LineKind::ToolResultErr,
            LineKind::System,
        ] {
            let (glyph, _, _, _) = row_style(&theme, kind);
            gutters.push((kind, glyph));
        }
        // User, tool call, tool ok, tool error and system each have a marker;
        // assistant prose deliberately has none, and thinking has a rule. The
        // call and the notice are the pair that used to share `*` — with colour
        // gone, that made a pending call and a lifecycle notice the same pixels.
        assert_eq!(gutters[0].1, ">"); // user
        assert_eq!(gutters[1].1, " "); // assistant: no marker, by design
        assert_eq!(gutters[2].1, "|"); // thinking
        assert_eq!(gutters[3].1, "\\"); // tool call
        assert_eq!(gutters[4].1, "+"); // ok
        assert_eq!(gutters[5].1, "x"); // error
        assert_eq!(gutters[6].1, "*"); // system notice
        assert_ne!(gutters[0].1, gutters[5].1);
        assert_ne!(gutters[3].1, gutters[6].1);
    }

    #[test]
    fn the_gutter_is_exactly_two_columns_for_every_role() {
        for theme in [
            Theme::new(ColorMode::Truecolor, UNICODE),
            Theme::new(ColorMode::None, ASCII),
        ] {
            for kind in [
                LineKind::User,
                LineKind::Assistant,
                LineKind::Thinking,
                LineKind::ToolCall,
                LineKind::ToolResultOk,
                LineKind::ToolResultErr,
                LineKind::System,
            ] {
                let line = decorate(&theme, kind, vec![Span::raw("body")], 40);
                let first = line.spans.first().expect("gutter span");
                assert_eq!(
                    crate::ui::layout::display_width(&first.content),
                    GUTTER,
                    "gutter width drifted for {kind:?}"
                );
                // Body text starts at exactly GUTTER for every role, which is
                // what makes a conversation read as two aligned columns.
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                let body: String = text.chars().skip(GUTTER).collect();
                assert!(body.starts_with("body"), "{kind:?}: {text:?}");
            }
        }
    }

    #[test]
    fn a_banded_row_fills_to_the_right_edge() {
        let theme = Theme::new(ColorMode::Truecolor, UNICODE);
        let line = decorate(&theme, LineKind::User, vec![Span::raw("hi")], 20);
        let total: usize = line
            .spans
            .iter()
            .map(|s| crate::ui::layout::display_width(&s.content))
            .sum();
        assert_eq!(total, 20, "band did not reach the edge");
    }

    #[test]
    fn a_plain_row_is_not_padded_with_invisible_fill() {
        // Only banded roles pad: padding every row would write cells for nothing
        // and make `NO_COLOR` output differ from coloured output for no reason.
        let theme = Theme::new(ColorMode::None, ASCII);
        let line = decorate(&theme, LineKind::Assistant, vec![Span::raw("hi")], 20);
        let total: usize = line
            .spans
            .iter()
            .map(|s| crate::ui::layout::display_width(&s.content))
            .sum();
        assert_eq!(total, GUTTER + 2);
    }

    #[test]
    fn the_welcome_screen_is_not_a_blank_void() {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let text = screen(welcome(&theme, 80), 80, 20);
        assert!(text.contains("E P S I L O N"), "{text}");
        assert!(text.contains("describe a task"), "{text}");
        assert!(text.contains("/help"), "{text}");
    }
}
