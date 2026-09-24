//! Overlay chrome: the modal surfaces that sit *above* the main screen.
//!
//! There are two kinds, and they are drawn differently on purpose:
//!
//! * **Panels** — the model picker, command palette, session browser, timeline.
//!   These are long lists the reader scans, so they get a rounded frame, a
//!   title, and a selection bar. Exactly one border separates them from the
//!   screen behind.
//! * **Prompts** — the permission dialog and the API-key prompt. These ask for a
//!   decision, so they get the same frame plus a colour that says "this owns the
//!   keyboard" (the accent for a question, the danger colour for a decision that
//!   can authorize work).
//!
//! Both clear their own rectangle first. A modal painted over the transcript
//! without clearing shows the conversation through the gaps, which reads as a
//! rendering bug rather than as a dialog.
//!
//! The frame is drawn from [`Theme::glyphs`], so a console that cannot render
//! box-drawing characters falls back to `+`/`-`/`|` rather than to mojibake.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::ui::theme::{Glyphs, Theme};

/// Where a panel sits and how it is titled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    /// A share of the screen, with a floor so it stays readable.
    Share {
        /// Numerator of the share.
        numerator: u16,
        /// Denominator of the share.
        denominator: u16,
        /// Never narrower than this.
        min: u16,
    },
    /// A fixed width, clamped to the screen.
    Fixed(u16),
}

impl Width {
    /// The widest a list panel needs to be to stay readable.
    pub const PANEL: Self = Self::Share {
        numerator: 4,
        denominator: 5,
        min: 48,
    };
    /// A prompt is narrower: it holds a few short lines, not a list.
    pub const PROMPT: Self = Self::Share {
        numerator: 3,
        denominator: 5,
        min: 46,
    };

    /// Resolve against the screen.
    pub fn resolve(self, outer: Rect) -> u16 {
        match self {
            Self::Share {
                numerator,
                denominator,
                min,
            } => (outer.width * numerator / denominator)
                .max(min)
                .min(outer.width),
            Self::Fixed(width) => width.min(outer.width),
        }
    }
}

/// A rectangle in the middle of the screen.
///
/// Centred rather than top-aligned: a panel that appears where the eye already
/// is needs no search, and terminal users read the middle of the screen first
/// after the prompt line.
///
/// Both dimensions are clamped to the screen. An overlay larger than its parent
/// would be clipped by the buffer, and the clipped edge is where the frame's
/// right rail and bottom rule live — so the panel would lose its border exactly
/// when the terminal is smallest.
pub fn centered(outer: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(outer.width);
    let height = height.min(outer.height);
    Rect {
        x: outer.x + outer.width.saturating_sub(width) / 2,
        y: outer.y + outer.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

/// One overlay ready to be drawn.
pub struct Panel<'a> {
    /// The resolved look.
    pub theme: &'a Theme,
    /// Where it goes.
    pub area: Rect,
    /// Label on the top edge.
    pub title: &'a str,
    /// `true` when the overlay owns the keyboard.
    pub focused: bool,
    /// An explicit frame colour, overriding what [`Panel::focused`] implies.
    ///
    /// "This owns the keyboard" and "this can authorize a write" are different
    /// statements, and the permission dialog is the case where they come apart:
    /// it does own the keyboard, and what it needs to say is the second thing.
    /// Before this field existed the dialog had no way to say it — it passed
    /// `focused: true` and got the accent, while its own doc comment claimed a
    /// danger treatment that nothing applied.
    ///
    /// `None` keeps the default: the focus accent when focused, the resting
    /// border otherwise.
    pub border: Option<ratatui::style::Style>,
}

impl Panel<'_> {
    /// Clear the rectangle and draw the frame, returning the area left inside.
    pub fn open(&self, f: &mut Frame<'_>) -> Rect {
        f.render_widget(Clear, self.area);
        let glyphs: &Glyphs = &self.theme.glyphs;
        let width = self.area.width as usize;
        // An explicit colour wins: a dialog that can authorize a write says so
        // with its frame, and no value of `focused` can express that.
        let border = self.border.unwrap_or_else(|| {
            if self.focused {
                self.theme.border_focus()
            } else {
                self.theme.border()
            }
        });

        // Build the frame by hand rather than with `Block`: it is a rounded,
        // titled edge with no nesting, and drawing the three pieces directly
        // keeps the title's padding exact at every width.
        let (left, title, right) = glyphs.titled_top(self.title, width);
        let top = Line::from(vec![
            Span::styled(left, border),
            Span::styled(title, self.theme.title()),
            Span::styled(right, border),
        ]);
        f.render_widget(
            Paragraph::new(top),
            Rect {
                height: 1,
                ..self.area
            },
        );

        if self.area.height >= 2 {
            let bottom = Line::from(Span::styled(
                format!(
                    "{}{}{}",
                    glyphs.bottom_left,
                    glyphs.rule(width.saturating_sub(2)),
                    glyphs.bottom_right
                ),
                border,
            ));
            f.render_widget(
                Paragraph::new(bottom),
                Rect {
                    y: self.area.y + self.area.height - 1,
                    height: 1,
                    ..self.area
                },
            );
            // The two vertical rails, one cell each.
            for y in 1..self.area.height - 1 {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(glyphs.vertical, border))),
                    Rect {
                        x: self.area.x,
                        y: self.area.y + y,
                        width: 1,
                        height: 1,
                    },
                );
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(glyphs.vertical, border))),
                    Rect {
                        x: self.area.x + self.area.width - 1,
                        y: self.area.y + y,
                        width: 1,
                        height: 1,
                    },
                );
            }
        }
        self.inner()
    }

    /// The area inside the frame, with one column of padding on each side.
    pub fn inner(&self) -> Rect {
        Rect {
            x: self.area.x + 2,
            y: self.area.y + 1,
            width: self.area.width.saturating_sub(4),
            height: self.area.height.saturating_sub(2),
        }
    }
}

/// A row in an overlay list.
///
/// Selection is shown with a leading bar *and* a style, never with style alone:
/// on a monochrome terminal, or with `NO_COLOR`, a style-only selection is
/// invisible and the list becomes unusable.
pub fn list_row(
    theme: &Theme,
    selected: bool,
    label: &str,
    meta: Option<&str>,
    width: usize,
) -> Line<'static> {
    use crate::ui::layout::display_width;

    let bar = if selected {
        theme.glyphs.cursor_bar
    } else {
        " "
    };
    let label_style = if selected {
        theme.selected()
    } else {
        theme.text()
    };
    let mut spans = vec![
        Span::styled(
            format!("{bar}{}", " ".repeat(GUTTER_AFTER_BAR)),
            theme.accent(),
        ),
        Span::styled(label.to_string(), label_style),
    ];
    if let Some(meta) = meta {
        // Right-align the metadata so a column of hints or counts lines up and
        // can be scanned without reading the labels. The gutter returns the
        // column it took, so the row lands exactly on `width`.
        let used = display_width(bar) + GUTTER_AFTER_BAR + display_width(label);
        let meta_width = display_width(meta);
        let pad = width.saturating_sub(used + meta_width);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(meta.to_string(), theme.faint()));
    }
    Line::from(spans)
}

/// Columns between the selection bar and the label.
const GUTTER_AFTER_BAR: usize = 1;

/// A line of key hints at the bottom of a panel, drawn inside the frame.
pub fn hint_row(theme: &Theme, hints: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (key, what)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", theme.faint()));
        }
        spans.push(Span::styled((*key).to_string(), theme.dim_strong()));
        spans.push(Span::styled(format!(" {what}"), theme.faint()));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::row_text;
    use crate::ui::theme::{ASCII, ColorMode, UNICODE};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn theme() -> Theme {
        Theme::new(ColorMode::Ansi16, ASCII)
    }

    #[test]
    fn a_centred_panel_is_actually_centred_and_never_off_screen() {
        let outer = Rect::new(0, 0, 100, 30);
        let area = centered(outer, 60, 9);
        assert_eq!(area.x, 20);
        assert_eq!(area.y, 10);
        // A panel larger than the screen is clamped rather than drawn outside it.
        let huge = centered(outer, 200, 90);
        assert!(huge.width <= outer.width);
        assert!(huge.height <= outer.height);
    }

    #[test]
    fn the_frame_carries_the_title_and_leaves_the_inside_free() {
        let theme = theme();
        let area = Rect::new(0, 0, 20, 5);
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).expect("test terminal");
        terminal
            .draw(|f| {
                let panel = Panel {
                    theme: &theme,
                    area,
                    title: "models",
                    focused: true,
                    border: None,
                };
                let inner = panel.open(f);
                assert_eq!(inner.x, 2);
                assert_eq!(inner.width, 16);
                assert_eq!(inner.height, 3);
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let top = row_text(buffer, 0, 20);
        assert!(top.starts_with('+'), "{top}");
        assert!(top.contains(" models "), "{top}");
        assert_eq!(
            crate::ui::layout::display_width(&top),
            20,
            "the frame must fill its width exactly: {top:?}"
        );
        let middle = row_text(buffer, 2, 20);
        assert!(middle.starts_with('|') && middle.ends_with('|'), "{middle}");
    }

    #[test]
    fn the_frame_is_exactly_as_wide_as_asked_at_every_size() {
        for width in [20u16, 46, 80, 120] {
            let theme = theme();
            let area = Rect::new(0, 0, width, 3);
            let mut terminal = Terminal::new(TestBackend::new(width, 3)).expect("test terminal");
            terminal
                .draw(|f| {
                    Panel {
                        theme: &theme,
                        area,
                        title: "pick a model",
                        focused: false,
                        border: None,
                    }
                    .open(f);
                })
                .expect("draw");
            let top = row_text(terminal.backend().buffer(), 0, width);
            assert_eq!(
                crate::ui::layout::display_width(&top),
                width as usize,
                "width {width}: {top:?}"
            );
        }
    }

    #[test]
    fn unicode_frames_use_rounded_corners_and_ascii_does_not() {
        let check = |theme: Theme, corner: char| {
            let area = Rect::new(0, 0, 20, 3);
            let mut terminal = Terminal::new(TestBackend::new(20, 3)).expect("test terminal");
            terminal
                .draw(|f| {
                    Panel {
                        theme: &theme,
                        area,
                        title: "t",
                        focused: false,
                        border: None,
                    }
                    .open(f);
                })
                .expect("draw");
            let top = row_text(terminal.backend().buffer(), 0, 20);
            assert!(top.starts_with(corner), "{top:?}");
        };
        check(
            Theme::new(ColorMode::Truecolor, UNICODE),
            UNICODE.top_left.chars().next().expect("corner"),
        );
        check(theme(), '+');
    }

    #[test]
    fn a_selected_row_is_marked_even_without_colour() {
        let plain = Theme::new(ColorMode::None, ASCII);
        let selected = list_row(&plain, true, "anthropic", Some("12 models"), 40);
        let unselected = list_row(&plain, false, "openai", Some("9 models"), 40);
        let text = |line: &Line<'static>| -> String {
            line.spans.iter().map(|s| s.content.as_ref()).collect()
        };
        assert!(text(&selected).starts_with('>'), "{}", text(&selected));
        assert!(text(&unselected).starts_with(' '));
    }

    #[test]
    fn row_metadata_is_right_aligned_inside_the_width() {
        let theme = theme();
        let row = list_row(&theme, false, "deepseek-chat", Some("164k"), 30);
        let text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(crate::ui::layout::display_width(&text), 30);
        assert!(text.ends_with("164k"), "{text:?}");
    }
}
