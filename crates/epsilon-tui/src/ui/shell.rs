//! Assembling one frame from application state.
//!
//! The widgets in this module's siblings take plain values; this file is the one
//! place that knows both about the application and about them. Keeping the
//! translation in a single function is what makes the rest of the presentation
//! layer testable without an agent.
//!
//! # What the shell decides, and what it deliberately does not
//!
//! * It decides **where things go** ([`layout`]) and **what the chrome says**
//!   ([`header`], [`footer`], [`composer`]).
//! * It does **not** decide what the transcript contains. That is
//!   [`crate::render_cache`] plus the draw code in [`crate::app`], which already
//!   wraps and parses each row exactly once; the shell only supplies the viewport
//!   it should fill.
//! * It does **not** decide which overlay is open. Overlays are painted last, by
//!   the caller, so one cannot be hidden behind another.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::ui::composer;
use crate::ui::footer::{self, Hints};
use crate::ui::header::{self, RunState};
use crate::ui::layout::{self, Regions};
use crate::ui::metrics::Metrics;
use crate::ui::theme::{Glyphs, Theme};

/// Everything the frame needs that is not the transcript body.
pub struct Shell<'a> {
    /// The resolved look.
    pub theme: &'a Theme,
    /// Run state for the header's left half.
    pub run: RunState,
    /// Measurements for the header's right half.
    pub metrics: Metrics,
    /// Key hints for the footer's left half.
    pub hints: Hints,
    /// Draft rows, already wrapped to `composer_width`.
    pub draft: Vec<String>,
    /// Caret position within `draft`.
    pub caret: (usize, usize),
    /// Whether the composer owns the keyboard; false while an overlay is open.
    pub composer_focused: bool,
    /// Whether there is anything to show in the transcript.
    pub transcript_empty: bool,

    /// Shown on the welcome panel: where this session lives. Empty is fine and
    /// means "unnamed" — the panel drops the row rather than printing a blank.
    pub session_label: &'a str,
}

/// The result of laying out a frame, so the caller can paint the transcript and
/// any overlays into the right rectangles.
pub struct Painted {
    /// The rectangle the transcript (or the welcome screen) occupies.
    pub transcript: Rect,
    /// The rectangle the composer occupies.
    pub composer: Rect,
}

/// Draw the chrome, and report the rectangles the caller still has to fill.
///
/// `draw_transcript` is called for the middle region; passing it in rather than
/// returning rows keeps the render-cache borrow inside the caller, where the
/// cache lives.
pub fn paint<F>(f: &mut Frame<'_>, area: Rect, shell: &Shell<'_>, draw_transcript: F) -> Painted
where
    F: FnOnce(&mut Frame<'_>, Rect),
{
    let theme = shell.theme;
    let glyphs: &Glyphs = &theme.glyphs;

    // Below the floor, the layout cannot be a layout. Say so, and hand back empty
    // rectangles so nothing else paints into a frame that cannot hold it.
    if layout::Regions::too_small(area) {
        paint_too_small(f, area, theme, glyphs);
        return Painted {
            transcript: Rect::default(),
            composer: Rect::default(),
        };
    }

    let regions = Regions::split(area, shell.draft.len().max(1) as u16);

    header::draw(f, regions.header, theme, &shell.run, &shell.metrics);

    if shell.transcript_empty {
        let lines = crate::ui::transcript::welcome(theme, shell.session_label);
        f.render_widget(ratatui::widgets::Paragraph::new(lines), regions.transcript);
    } else {
        draw_transcript(f, regions.transcript);
    }

    // One rule, and it is the composer's. It was two — the second sat between the
    // header and the transcript, drawn by this same function in this same style,
    // so neither line said what it divided while the pair cost a row of
    // conversation at every terminal size.
    composer::draw(
        f,
        regions.composer,
        theme,
        &shell.draft,
        shell.caret.0,
        shell.caret.1,
        shell.composer_focused,
    );
    composer::draw_rule(f, regions.composer_rule, theme);
    footer::draw(f, regions.footer, theme, &shell.hints);

    Painted {
        transcript: regions.transcript,
        composer: regions.composer,
    }
}

/// The one line shown when the window cannot hold the layout.
pub fn paint_too_small(f: &mut Frame<'_>, area: Rect, theme: &Theme, _glyphs: &Glyphs) {
    f.render_widget(
        ratatui::widgets::Paragraph::new(layout::too_small_message(area.width, area.height))
            .centered()
            .style(theme.warn()),
        area,
    );
}

/// The composer's text width for a given frame width.
///
/// Exposed because the caller wraps the draft before calling [`paint`], and the
/// two must agree exactly: wrap to a different width and the caret lands in the
/// wrong column.
pub fn composer_text_width(frame_width: u16, glyphs: &Glyphs) -> usize {
    (frame_width as usize).saturating_sub(composer::gutter_width(glyphs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{ASCII, ColorMode};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn shell(theme: &Theme, empty: bool) -> Shell<'_> {
        Shell {
            theme,
            run: RunState::Idle,
            metrics: Metrics {
                model_label: "DeepSeek V3.1".into(),
                context_used: Some(48_000),
                context_window: Some(200_000),
                tokens_in: 12_400,
                tokens_out: 3_400,
                cache_read_session: 36_000,
                cache_read_last: 36_000,
                prompt_tokens_last: Some(48_000),
                cost_micro_usd: Some(12_345),
                ..Metrics::default()
            },
            hints: Hints::idle(&crate::keybindings::KeybindingsManager::from_map(
                std::collections::HashMap::new(),
            )),
            draft: vec![String::new()],
            caret: (0, 0),
            composer_focused: true,
            transcript_empty: empty,
            session_label: "",
        }
    }

    fn render(width: u16, height: u16, empty: bool) -> Vec<String> {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        let sh = shell(&theme, empty);
        terminal
            .draw(|f| {
                paint(f, f.area(), &sh, |f, area| {
                    f.render_widget(ratatui::widgets::Paragraph::new("BODY ROW"), area);
                });
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn the_shell_shows_a_header_a_body_and_a_footer_at_every_supported_size() {
        for (w, h) in [(44u16, 10u16), (60, 20), (100, 30), (160, 48)] {
            let rows = render(w, h, false);
            assert_eq!(rows.len(), h as usize);
            assert!(rows[0].contains("idle"), "{w}x{h} header: {:?}", rows[0]);
            assert!(
                rows.iter().any(|r| r.contains("BODY ROW")),
                "{w}x{h} lost the transcript"
            );
            let footer = rows.last().expect("footer row");
            assert!(
                footer.contains("enter") || footer.contains("send"),
                "{w}x{h} footer: {footer:?}"
            );
            // Exactly one full-width rule, between the composer and the footer.
            // There used to be two, and `filter` must never see an empty row:
            // ''.chars().all(..) is vacuously true, which once made this count
            // every blank row too.
            let rules = rows
                .iter()
                .filter(|r| !r.is_empty() && r.chars().all(|c| c == '-'))
                .count();
            assert_eq!(rules, 1, "{w}x{h} drew {rules} rules: {rows:#?}");
        }
    }

    #[test]
    fn an_empty_transcript_gets_a_welcome_screen_not_a_void() {
        let rows = render(90, 24, true);
        let body = rows.join("\n");
        assert!(body.contains("E P S I L O N"), "{body}");
        assert!(!body.contains("BODY ROW"), "welcome and body both drawn");
    }

    #[test]
    fn a_screen_below_the_floor_says_so_and_draws_nothing_else() {
        let rows = render(30, 6, false);
        let body = rows.join("\n");
        assert!(body.contains("too small"), "{body}");
        assert!(!body.contains("BODY ROW"));
        assert!(!body.contains("E P S I L O N"));
    }

    #[test]
    fn the_composer_width_matches_what_the_composer_draws() {
        // A mismatch puts the caret in the wrong column for every keystroke, so
        // this is the contract between wrapping and drawing.
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        for width in [44u16, 80, 120] {
            let text_width = composer_text_width(width, &theme.glyphs);
            let draft = vec!["x".repeat(text_width)];
            let sh = Shell {
                draft: draft.clone(),
                caret: (0, text_width),
                ..shell(&theme, false)
            };
            let mut terminal = Terminal::new(TestBackend::new(width, 12)).expect("test terminal");
            terminal
                .draw(|f| {
                    paint(f, f.area(), &sh, |_, _| {});
                })
                .expect("draw");
            // The row that holds the draft must end exactly at the last column.
            let buffer = terminal.backend().buffer();
            let mut found = false;
            for y in 0..12 {
                let row = crate::ui::layout::row_text(buffer, y, width);
                if row.contains(&"x".repeat(text_width)) {
                    assert_eq!(
                        crate::ui::layout::display_width(row.trim_end()),
                        width as usize,
                        "width {width}: draft did not reach the edge"
                    );
                    found = true;
                }
            }
            assert!(found, "width {width}: draft row not found");
        }
    }
}
