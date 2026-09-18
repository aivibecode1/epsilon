//! Semantic theme tokens for a Ratatui AI-harness TUI.
//! Copy into `src/ui/theme.rs` and route every style through `Theme`.

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Debug)]
pub struct Theme {
    pub bg_base: Style,
    pub bg_surface: Style,
    pub bg_overlay: Style,
    pub text_primary: Style,
    pub text_muted: Style,
    pub accent: Style,
    pub border: Style,
    pub border_focus: Style,
    pub selection: Style,
    pub status_info: Style,
    pub status_ok: Style,
    pub status_warn: Style,
    pub status_err: Style,
    pub role_user: Style,
    pub role_assistant: Style,
    pub role_tool: Style,
}

impl Theme {
    pub fn dark() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some();
        if no_color {
            return Self::mono();
        }
        Self {
            bg_base: Style::new(),
            bg_surface: Style::new().bg(Color::Rgb(22, 22, 30)),
            bg_overlay: Style::new().bg(Color::Rgb(28, 28, 38)),
            text_primary: Style::new().fg(Color::Rgb(220, 222, 226)),
            text_muted: Style::new().fg(Color::Rgb(122, 126, 138)),
            accent: Style::new().fg(Color::Rgb(122, 162, 247)),
            border: Style::new().fg(Color::Rgb(58, 60, 74)),
            border_focus: Style::new().fg(Color::Rgb(122, 162, 247)),
            selection: Style::new().add_modifier(Modifier::REVERSED),
            status_info: Style::new().fg(Color::Rgb(122, 162, 247)),
            status_ok: Style::new().fg(Color::Rgb(158, 206, 106)),
            status_warn: Style::new().fg(Color::Rgb(224, 175, 104)),
            status_err: Style::new().fg(Color::Rgb(247, 118, 142)),
            role_user: Style::new()
                .fg(Color::Rgb(158, 206, 106))
                .add_modifier(Modifier::BOLD),
            role_assistant: Style::new()
                .fg(Color::Rgb(122, 162, 247))
                .add_modifier(Modifier::BOLD),
            role_tool: Style::new()
                .fg(Color::Rgb(187, 154, 247))
                .add_modifier(Modifier::BOLD),
        }
    }

    pub fn mono() -> Self {
        let dim = Style::new().add_modifier(Modifier::DIM);
        let bold = Style::new().add_modifier(Modifier::BOLD);
        Self {
            bg_base: Style::new(),
            bg_surface: Style::new(),
            bg_overlay: Style::new(),
            text_primary: Style::new(),
            text_muted: dim,
            accent: bold,
            border: dim,
            border_focus: bold,
            selection: Style::new().add_modifier(Modifier::REVERSED),
            status_info: Style::new(),
            status_ok: bold,
            status_warn: Style::new(),
            status_err: bold,
            role_user: bold,
            role_assistant: bold,
            role_tool: bold,
        }
    }
}
