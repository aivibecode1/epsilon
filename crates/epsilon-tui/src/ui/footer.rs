//! The footer: the keys that work *right now*, and where the session lives.
//!
//! ```text
//! enter send · esc stop · ctrl-p palette                  ~/work/api · main
//! ```
//!
//! Two rules make a key hint useful rather than noise:
//!
//! * **Only live keys.** A footer that lists `esc stop` while nothing is running
//!   trains the reader to stop reading it. The set is chosen per state.
//! * **The location is not a key.** The workspace path and branch answer "which
//!   project am I about to let this thing edit", which is a safety question, so
//!   it gets the right-hand side where the eye lands after the hints.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;

use crate::ui::layout::{Priority, clamp_line, side_by_side};
use crate::ui::theme::Theme;

/// What the footer should say, chosen by the caller from the live app state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hints {
    /// Pairs of `(key, action)`, in the order they should read.
    pub keys: Vec<(String, String)>,
    /// The workspace path, already shortened for display.
    pub location: Option<String>,
    /// The git branch, when the workspace is a repository.
    pub branch: Option<String>,
    /// Set while a run is in flight, so the footer can say how to stop it, and
    /// how many messages are waiting behind it.
    pub queued: usize,
}

impl Hints {
    /// The hints for an idle session at the composer.
    pub fn idle() -> Self {
        Self {
            keys: vec![
                ("enter".into(), "send".into()),
                ("ctrl-j".into(), "newline".into()),
                ("ctrl-p".into(), "palette".into()),
                ("?".into(), "help".into()),
            ],
            ..Self::default()
        }
    }

    /// The hints for a running session.
    pub fn running(queued: usize) -> Self {
        let mut keys = vec![
            ("esc".into(), "stop".into()),
            ("enter".into(), "queue".into()),
        ];
        if queued > 0 {
            keys.push(("queued".into(), queued.to_string()));
        }
        keys.push(("ctrl-p".into(), "palette".into()));
        Self {
            keys,
            queued,
            ..Self::default()
        }
    }

    /// The hints while a permission ask owns the keyboard.
    pub fn approval() -> Self {
        Self {
            keys: vec![
                ("y".into(), "allow once".into()),
                ("a".into(), "always".into()),
                ("n".into(), "deny".into()),
                ("e".into(), "deny + guidance".into()),
            ],
            ..Self::default()
        }
    }

    /// The rendered left half, without styling: used by tests and by the width
    /// budget so both agree on how long the hints are.
    pub fn keys_text(&self) -> String {
        self.keys
            .iter()
            .map(|(key, action)| format!("{key} {action}"))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// Draw the footer row.
///
/// The location is a safety signal — it answers "which project am I about to let
/// this edit" — so it wins the space argument, and the key hints shorten first.
/// [`side_by_side`] guarantees the row stays inside its width.
pub fn draw(f: &mut Frame<'_>, area: Rect, theme: &Theme, hints: &Hints) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut left: Vec<Span<'static>> = Vec::new();
    for (index, (key, action)) in hints.keys.iter().enumerate() {
        if index > 0 {
            left.push(Span::styled(" · ", theme.faint()));
        }
        // The key is the affordance and gets the brighter treatment; the action
        // is the explanation and stays quiet. Reading down the keys alone should
        // still tell an experienced user what is available.
        left.push(Span::styled(key.clone(), theme.dim_strong()));
        left.push(Span::styled(format!(" {action}"), theme.faint()));
    }

    let mut right: Vec<Span<'static>> = Vec::new();
    if let Some(location) = &hints.location {
        right.push(Span::styled(location.clone(), theme.faint()));
    }
    if let Some(branch) = &hints.branch {
        right.push(Span::styled(" · ", theme.faint()));
        right.push(Span::styled(branch.clone(), theme.faint()));
    }

    let line = side_by_side(left, right, area.width as usize, Priority::Right);
    let line = clamp_line(line, area.width as usize);
    f.render_widget(line, area);
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{ASCII, ColorMode, Theme};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render(width: u16, hints: &Hints) -> String {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("test terminal");
        terminal
            .draw(|f| draw(f, f.area(), &theme, hints))
            .expect("draw");
        crate::ui::layout::row_text(terminal.backend().buffer(), 0, width)
            .trim_end()
            .to_string()
    }

    #[test]
    fn idle_hints_name_only_keys_that_do_something() {
        let text = Hints::idle().keys_text();
        assert!(text.contains("enter send"));
        assert!(text.contains("ctrl-p palette"));
        assert!(!text.contains("stop"), "nothing is running: {text}");
    }

    #[test]
    fn running_hints_offer_the_stop_key() {
        let text = Hints::running(0).keys_text();
        assert!(text.contains("esc stop"), "{text}");
        let queued = Hints::running(2).keys_text();
        assert!(queued.contains('2'), "{queued}");
    }

    #[test]
    fn approval_hints_are_the_approval_keys() {
        let text = Hints::approval().keys_text();
        for key in ["y allow once", "a always", "n deny", "e deny + guidance"] {
            assert!(text.contains(key), "missing {key} in {text}");
        }
    }

    #[test]
    fn the_location_survives_a_narrow_terminal() {
        let hints = Hints {
            keys: vec![("enter".into(), "send".into())],
            location: Some("~/work/api".into()),
            branch: Some("main".into()),
            queued: 0,
        };
        for width in [30u16, 50, 80, 120] {
            let line = render(width, &hints);
            assert_eq!(crate::ui::layout::display_width(&line), width as usize, "width {width}");
            assert!(line.contains("main"), "width {width}: {line}");
        }
        // Narrower than the location: show the location, drop the hints.
        let narrow = render(12, &hints);
        assert!(narrow.contains("api"), "{narrow}");
    }

    #[test]
    fn nothing_is_drawn_past_the_row_width() {
        let hints = Hints::running(0);
        for width in [24u16, 40, 80] {
            let line = render(width, &hints);
            assert!(
                crate::ui::layout::display_width(&line) <= width as usize,
                "width {width} overflowed to {} cells: {line:?}",
                crate::ui::layout::display_width(&line)
            );
            assert!(!line.ends_with(' '), "trailing padding looks like a bug: {line:?}");
        }
    }
}
