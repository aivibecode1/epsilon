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
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

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
    /// Welcome lines pinned above the transcript while no conversation exists
    /// yet (empty once conversation started). Decided by the caller — the
    /// shell only supplies the viewport split, so both sides measure the
    /// same rectangles (see [`split_welcome`]).
    pub welcome: Vec<ratatui::text::Line<'static>>,

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

    if !shell.welcome.is_empty() {
        // No conversation yet: the welcome stays pinned above whatever
        // notices exist (or nothing, on a clean launch) instead of being
        // suppressed by them. The body rectangle comes from the same split
        // the caller measured (see `split_welcome`), so the viewport the
        // transcript draws always matches its geometry.
        let draft_rows = shell.draft.len().max(1) as u16;
        let (welcome_rect, body_rect) = split_welcome(area, draft_rows, shell.welcome.len() as u16);
        f.render_widget(
            ratatui::widgets::Paragraph::new(shell.welcome.clone()),
            welcome_rect,
        );
        draw_transcript(f, body_rect);
    } else {
        draw_transcript(f, regions.transcript);
    }

    // The blank that ends the conversation.
    //
    // Rendered explicitly rather than left to the terminal: the transcript used
    // to own this row, so a diff-based redraw would otherwise leave the last
    // line of the old conversation sitting exactly where the gap should be, with
    // the composer drawn one row lower. Drawing an empty Paragraph clears it the
    // same way drawing any other widget clears its region.
    f.render_widget(
        ratatui::widgets::Paragraph::new(ratatui::text::Text::default()),
        regions.gap,
    );

    // The composer is a box, not a bare rule: the top border reuses the blank
    // row the layout already reserved, the bottom border reuses the rule row, and
    // the side rails cost two columns of text width rather than two rows. It is
    // the one control the user's hands are on, so it is also the one border whose
    // colour changes with focus.
    let composer_area = draw_composer_box(f, &regions, theme, shell.composer_focused);
    composer::draw(
        f,
        composer_area,
        theme,
        &shell.draft,
        shell.caret.0,
        shell.caret.1,
        shell.composer_focused,
    );
    footer::draw(f, regions.footer, theme, &shell.hints);

    Painted {
        transcript: regions.transcript,
        composer: regions.composer,
    }
}

/// Draw the composer's box and return the rectangle its text may occupy.
///
/// The box reuses existing rows: [`Regions::gap`] becomes the top border and
/// [`Regions::composer_rule`] the bottom. That keeps every geometry invariant in
/// [`crate::ui::layout`] — and every test that measures the transcript viewport —
/// while giving the prompt the same visual weight as the conversation.
fn draw_composer_box(f: &mut Frame<'_>, regions: &Regions, theme: &Theme, focused: bool) -> Rect {
    let full = regions.composer;
    if full.width < 12
        || full.height == 0
        || regions.gap.height == 0
        || regions.composer_rule.height == 0
    {
        return full;
    }
    let style = if focused {
        theme.border_focus()
    } else {
        theme.border()
    };
    let glyphs = &theme.glyphs;
    let width = full.width as usize;

    // `╭─ prompt ───╮` — the label is part of the border, so the box says what
    // it is without another row of chrome.
    let label = " prompt ";
    let mut top = Vec::with_capacity(5);
    top.push(Span::styled(glyphs.top_left, style));
    top.push(Span::styled(glyphs.horizontal, style));
    if width >= 12 {
        top.push(Span::styled(label, theme.title()));
        top.push(Span::styled(
            glyphs.horizontal.repeat(width.saturating_sub(11)),
            style,
        ));
    } else {
        top.push(Span::styled(
            glyphs.horizontal.repeat(width.saturating_sub(2)),
            style,
        ));
    }
    top.push(Span::styled(glyphs.top_right, style));
    f.render_widget(Line::from(top), regions.gap);

    let bottom = vec![
        Span::styled(glyphs.bottom_left, style),
        Span::styled(glyphs.horizontal.repeat(width.saturating_sub(2)), style),
        Span::styled(glyphs.bottom_right, style),
    ];
    f.render_widget(Line::from(bottom), regions.composer_rule);

    // Side rails. They are drawn before the text so the text, which is inset by
    // one column on each side, never paints over them.
    let rails: Vec<Line<'static>> = (0..full.height)
        .map(|_| Line::from(Span::styled(glyphs.vertical, style)))
        .collect();
    f.render_widget(
        Paragraph::new(rails.clone()),
        Rect {
            x: full.x,
            y: full.y,
            width: 1,
            height: full.height,
        },
    );
    f.render_widget(
        Paragraph::new(rails),
        Rect {
            x: full.x + full.width - 1,
            y: full.y,
            width: 1,
            height: full.height,
        },
    );

    Rect {
        x: full.x + 1,
        y: full.y,
        width: full.width - 2,
        height: full.height,
    }
}

/// Split the transcript rectangle into a welcome prefix and the body below.
///
/// The one home for the split: `paint` renders it and the caller measures
/// the body's sub-viewport through this same function, so the geometry the
/// footer badge reads always describes the rectangle actually drawn. The
/// welcome is capped to the available height (a zero-height body draws
/// nothing — see `draw_transcript`'s viewport guard).
pub fn split_welcome(area: Rect, draft_rows: u16, welcome_rows: u16) -> (Rect, Rect) {
    let regions = Regions::split(area, draft_rows);
    let take = (welcome_rows as usize).min(regions.transcript.height as usize) as u16;
    let mut welcome_rect = regions.transcript;
    welcome_rect.height = take;
    let mut body_rect = regions.transcript;
    body_rect.y += take;
    body_rect.height = body_rect.height.saturating_sub(take);
    (welcome_rect, body_rect)
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
    // Two columns are the box's side rails. The caller wraps the draft before
    // `paint` and the shell draws it inside those rails; counting only the prompt
    // gutter here put the last wrapped column under the right rail.
    (frame_width as usize).saturating_sub(composer::gutter_width(glyphs) + 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{ASCII, ColorMode};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn shell<'a>(theme: &'a Theme, welcome: Vec<ratatui::text::Line<'static>>) -> Shell<'a> {
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
            welcome,
            session_label: "",
        }
    }

    fn render(width: u16, height: u16, welcome: bool) -> Vec<String> {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let lines = if welcome {
            crate::ui::transcript::welcome(&theme, "")
        } else {
            Vec::new()
        };
        let sh = shell(&theme, lines);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
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
            // The composer is a box: a top and bottom border with rails. It is
            // no longer a bare full-width rule, so count the box's corners.
            let boxed = rows
                .iter()
                .filter(|r| {
                    (r.starts_with('+') || r.starts_with('╭') || r.starts_with('╰'))
                        && (r.ends_with('+') || r.ends_with('╮') || r.ends_with('╯'))
                })
                .count();
            assert!(boxed >= 2, "{w}x{h} drew no composer box: {rows:#?}");
        }
    }

    #[test]
    fn an_empty_transcript_gets_a_welcome_screen_not_a_void() {
        let rows = render(90, 24, true);
        let body = rows.join("\n");
        assert!(body.contains("E P S I L O N"), "{body}");
    }

    /// Notices but no conversation: the welcome pins above the body instead
    /// of being suppressed by it. The shell test's BODY ROW stands in for
    /// the notice rows — what matters is the ORDER (welcome first) and that
    /// both reach the frame.
    #[test]
    fn welcome_pins_above_the_body_it_shares_the_region_with() {
        let rows = render(90, 24, true);
        let body = rows.join("\n");
        let welcome_at = rows
            .iter()
            .position(|r| r.contains("E P S I L O N"))
            .expect("welcome renders: {body}");
        let body_at = rows
            .iter()
            .position(|r| r.contains("BODY ROW"))
            .expect("body still renders below: {body}");
        assert!(
            welcome_at < body_at,
            "welcome must pin above the body:\n{body}"
        );
    }

    #[test]
    fn the_rule_fills_its_row_exactly() {
        // The bare rule became the composer box's bottom border. It still has to
        // span the full frame exactly once; a box that leaves a ragged right edge
        // is the same defect under a new name.
        let rows = render(60, 20, false);
        let rule = rows
            .iter()
            .find(|row| {
                row.len() == 60
                    && row.starts_with('+')
                    && row.ends_with('+')
                    && row[1..59].chars().all(|c| c == '-')
            })
            .unwrap_or_else(|| panic!("no full-width composer rule: {rows:#?}"));
        assert_eq!(
            crate::ui::layout::display_width(rule),
            60,
            "the composer rule must fill its row exactly"
        );
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
                ..shell(&theme, Vec::new())
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
