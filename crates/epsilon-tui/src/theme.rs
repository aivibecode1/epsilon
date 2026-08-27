//! Central TUI palette (pi's theme-controller pattern: one struct, no scattered
//! color literals). Dark defaults; TOML palettes land later if ever needed.

use ratatui::style::{Color, Modifier, Style};

/// Semantic colors used across the frontend.
#[derive(Debug, Clone, Copy)]
pub struct Theme;
impl Theme {
    /// User input / local echo.
    pub fn user() -> Style {
        Style::default().fg(Color::Cyan)
    }
    /// Assistant prose.
    pub fn assistant() -> Style {
        Style::default()
    }
    /// Thinking rows.
    pub fn thinking() -> Style {
        Style::default().fg(Color::DarkGray)
    }
    /// Requested tool call.
    pub fn tool_call() -> Style {
        Style::default().fg(Color::Yellow)
    }
    /// Successful tool output.
    pub fn tool_ok() -> Style {
        Style::default().fg(Color::Green)
    }
    /// Failed tool output.
    pub fn tool_err() -> Style {
        Style::default().fg(Color::Red)
    }
    /// System/lifecycle notices.
    pub fn system() -> Style {
        Style::default().fg(Color::DarkGray)
    }
    /// Inline code + fenced blocks.
    pub fn code_inline() -> Style {
        Style::default().fg(Color::LightBlue)
    }
    /// Fenced block gutter/body.
    pub fn code_block() -> Style {
        Style::default().fg(Color::DarkGray)
    }
    /// Headings inside assistant markdown.
    pub fn heading() -> Style {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    }
    /// Accent for borders of active overlays.
    pub fn accent() -> Style {
        Style::default().fg(Color::Cyan)
    }
    /// Muted accent for secondary chrome (titles, hints, borders).
    pub fn muted() -> Style {
        Style::default().fg(Color::DarkGray)
    }
    /// Subtle border style (overlays, input, picker).
    pub fn border() -> Style {
        Style::default().fg(Color::DarkGray)
    }
    /// Active border (focused picker / palette).
    pub fn border_active() -> Style {
        Style::default().fg(Color::Cyan)
    }
    /// Auth overlay border/title (opencode DialogPrompt parity).
    pub fn auth() -> Style {
        Style::default().fg(Color::Yellow)
    }
    /// Danger accents (deny options, errors).
    pub fn danger() -> Style {
        Style::default().fg(Color::Red)
    }
    /// Positive accents (allow options).
    pub fn success() -> Style {
        Style::default().fg(Color::Green)
    }
    /// Selection highlight (reversed accent).
    pub fn selected() -> Style {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::REVERSED)
    }
    /// Dim selection description (reversed muted).
    pub fn selected_muted() -> Style {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::REVERSED)
    }
}
