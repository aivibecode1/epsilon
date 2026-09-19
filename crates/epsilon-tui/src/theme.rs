//! The palette the *overlays* draw with (pickers, palettes, the approval
//! dialog, the timeline).
//!
//! # Read this before adding a colour here
//!
//! The main screen — header, transcript, composer, footer — draws through
//! [`crate::ui::theme::Theme`], which carries a resolved [`ColorMode`], a glyph
//! repertoire, and the `NO_COLOR` fallback. This module is the older path: a set
//! of free functions with no state, used by the overlay widgets.
//!
//! It is a compatibility layer, not a second design system, and it is scheduled
//! to disappear. Two rules keep it honest until then:
//!
//! 1. **It resolves through the same palette.** [`Theme::accent`] here and
//!    `ui::theme::Theme::accent()` there are the same RGB value, so a picker
//!    opened over the transcript cannot clash with it.
//! 2. **Do not add new roles.** A new widget should take a
//!    [`crate::ui::theme::Theme`] and call the role it means. Adding a colour
//!    here extends the migration rather than finishing it.
//!
//! [#1]: crate::ui::theme
//! [`ColorMode`]: crate::ui::theme::ColorMode

use ratatui::style::{Color, Modifier, Style};

use crate::ui::theme::{ColorMode, Palette, Role};

/// Resolve one role for the overlay path.
fn role(role: Role) -> Style {
    let slot = Palette::DARK.get(role);
    match ColorMode::detect() {
        ColorMode::Truecolor => Style::default().fg(Color::Rgb(slot.rgb.0, slot.rgb.1, slot.rgb.2)),
        ColorMode::Ansi16 => Style::default().fg(slot.ansi),
        ColorMode::None => Style::default(),
    }
}

/// Semantic colors used by the overlay widgets.
#[derive(Debug, Clone, Copy)]
pub struct Theme;

impl Theme {
    /// User input / local echo.
    pub fn user() -> Style {
        role(Role::user)
    }

    /// Assistant prose.
    pub fn assistant() -> Style {
        role(Role::assistant)
    }

    /// Thinking rows.
    pub fn thinking() -> Style {
        role(Role::thinking).add_modifier(Modifier::ITALIC)
    }

    /// Requested tool call.
    pub fn tool_call() -> Style {
        role(Role::warn)
    }

    /// Successful tool output.
    pub fn tool_ok() -> Style {
        role(Role::success)
    }

    /// Failed tool output.
    pub fn tool_err() -> Style {
        role(Role::danger)
    }

    /// System/lifecycle notices.
    pub fn system() -> Style {
        role(Role::text_dim)
    }

    /// Inline code + fenced blocks.
    pub fn code_inline() -> Style {
        role(Role::code)
    }

    /// Fenced block gutter/body.
    pub fn code_block() -> Style {
        role(Role::code_block)
    }

    /// Headings inside assistant markdown.
    pub fn heading() -> Style {
        role(Role::user).add_modifier(Modifier::BOLD)
    }

    /// Accent for borders of active overlays.
    pub fn accent() -> Style {
        role(Role::border_focus)
    }

    /// Muted accent for secondary chrome (titles, hints, borders).
    pub fn muted() -> Style {
        role(Role::text_dim)
    }

    /// Subtle border style (overlays, input, picker).
    pub fn border() -> Style {
        role(Role::border)
    }

    /// Active border (focused picker / palette).
    pub fn border_active() -> Style {
        role(Role::border_focus)
    }

    /// Auth overlay border/title (opencode DialogPrompt parity).
    pub fn auth() -> Style {
        role(Role::warn)
    }

    /// Danger accents (deny options, errors).
    pub fn danger() -> Style {
        role(Role::danger)
    }

    /// Positive accents (allow options).
    pub fn success() -> Style {
        role(Role::success)
    }

    /// Selection highlight (reversed accent).
    pub fn selected() -> Style {
        role(Role::border_focus).add_modifier(Modifier::REVERSED)
    }

    /// Dim selection description (reversed muted).
    pub fn selected_muted() -> Style {
        role(Role::text_faint).add_modifier(Modifier::REVERSED)
    }
}
