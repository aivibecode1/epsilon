//! Semantic theme tokens for a Ratatui AI-harness TUI.
//!
//! This file is a *distillation* of a working implementation, not a sketch. The
//! three things it does that a naive token struct does not are the three things
//! that decide whether the UI is usable on a real terminal:
//!
//! 1. **Colour depth is detected once, at startup**, and the theme carries it.
//!    Claiming 24-bit colour on a console that lacks it produces unreadable
//!    output; falling back to 16 colours merely looks plainer. The asymmetry is
//!    the whole argument for a conservative default.
//! 2. **`NO_COLOR` still has to convey meaning.** When colour is off, emphasis
//!    comes from attributes — bold for a value the reader must catch, dim+italic
//!    for background material. A theme that maps every role to `Style::default()`
//!    under `NO_COLOR` makes reasoning traces indistinguishable from answers.
//! 3. **Widths are measured in columns, never in `char`s.** A braille spinner
//!    (one column), a box rule (one) and a CJK glyph (two) are each one `char`.
//!    Every derived layout width is wrong by the difference, which shows up as a
//!    row one cell too wide that wraps.
//!
//! Copy into `src/ui/theme.rs` and route every style through the theme.

use ratatui::style::{Color, Modifier, Style};

/// How much colour the terminal can show. Detected once; a look that changed
/// mid-session would flicker between two designs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// 24-bit colour.
    Truecolor,
    /// The classic 16 ANSI colours — where Windows `conhost` lands unless it
    /// opts in.
    Ansi16,
    /// `NO_COLOR` is set: attributes only.
    None,
}

impl ColorMode {
    /// The environment decides, and the rule is deliberately conservative.
    pub fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return Self::None;
        }
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return Self::Truecolor;
        }
        // Windows Terminal and VS Code advertise themselves; conhost does not.
        if std::env::var_os("WT_SESSION").is_some() || std::env::var_os("TERM_PROGRAM").is_some() {
            return Self::Truecolor;
        }
        Self::Ansi16
    }
}

/// One semantic role, with its truecolor and 16-colour values.
///
/// A *role*, not a colour: `success`, not `green`. Two roles sharing a colour
/// usually means they are one role; two that must be distinguishable at a glance
/// (success and failure) must never share one.
#[derive(Debug, Clone, Copy)]
struct Slot {
    rgb: (u8, u8, u8),
    ansi: Color,
}

macro_rules! palette {
    ($($name:ident: $rgb:expr, $ansi:expr;)*) => {
        #[derive(Debug, Clone, Copy)]
        struct Palette {
            $($name: Slot,)*
        }

        /// Snake-case on purpose: a role reads as a palette field.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types, missing_docs)]
        pub enum Role {
            $($name,)*
        }

        impl Palette {
            const DARK: Self = Self {
                $($name: Slot { rgb: $rgb, ansi: $ansi },)*
            };

            fn get(&self, role: Role) -> Slot {
                match role {
                    $(Role::$name => self.$name,)*
                }
            }
        }
    };
}

palette! {
    // Structure: chrome the eye should skip over.
    border:        (0x3b, 0x40, 0x4a), Color::DarkGray;
    border_focus:  (0x62, 0x9d, 0xf5), Color::Cyan;
    // Text.
    text:          (0xd7, 0xdb, 0xe2), Color::White;
    text_dim:      (0x8a, 0x92, 0xa3), Color::DarkGray;
    text_faint:    (0x5a, 0x62, 0x72), Color::DarkGray;
    // Who is speaking.
    user:          (0x7d, 0xd3, 0xfc), Color::Cyan;
    assistant:     (0xd7, 0xdb, 0xe2), Color::White;
    thinking:      (0x8a, 0x92, 0xa3), Color::DarkGray;
    // Outcome.
    success:       (0x6e, 0xe7, 0xa0), Color::Green;
    danger:        (0xf8, 0x76, 0x76), Color::Red;
    warn:          (0xf5, 0xc2, 0x5c), Color::Yellow;
    accent:        (0x9d, 0x7c, 0xf5), Color::Magenta;
    // Inline code and fenced blocks.
    code:          (0xe0, 0xaf, 0x68), Color::Yellow;
    code_block:    (0x9a, 0xa3, 0xb5), Color::Gray;
    // Filled bands, truecolor only. Close to a dark background on purpose: a
    // band should read as a change of surface, not as a selected row.
    surface_user:    (0x22, 0x24, 0x2c), Color::Reset;
    surface_pending: (0x2a, 0x2a, 0x22), Color::Reset;
    surface_ok:      (0x1e, 0x2a, 0x22), Color::Reset;
    surface_err:     (0x2e, 0x20, 0x20), Color::Reset;
}

/// How a role should be emphasised, independent of whether colour is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Emphasis {
    Plain,
    /// A value the reader must still catch when colour is off.
    Strong,
    /// Background material: reasoning traces.
    Muted,
    /// The keyboard owner.
    Focus,
}

impl Emphasis {
    fn modifier(self) -> Modifier {
        match self {
            Self::Strong | Self::Focus => Modifier::BOLD,
            Self::Muted => Modifier::DIM | Modifier::ITALIC,
            Self::Plain => Modifier::empty(),
        }
    }
}

/// The resolved look for one session. `Copy`, so it can be passed by value into
/// a closure that also needs `&mut App` — holding a borrow of `app.theme` across
/// such a call is exactly what the borrow checker refuses.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    mode: ColorMode,
}

impl Theme {
    /// Build for a colour mode.
    pub fn new(mode: ColorMode) -> Self {
        Self { mode }
    }

    /// Detect from the environment, which is what an app does at startup.
    pub fn detect() -> Self {
        Self::new(ColorMode::detect())
    }

    /// Resolve a role. The single place where "what does emphasis mean when
    /// there is no colour" is decided.
    fn role(&self, role: Role, emphasis: Emphasis) -> Style {
        let slot = Palette::DARK.get(role);
        let coloured = match self.mode {
            ColorMode::Truecolor => {
                Style::default().fg(Color::Rgb(slot.rgb.0, slot.rgb.1, slot.rgb.2))
            }
            ColorMode::Ansi16 => Style::default().fg(slot.ansi),
            ColorMode::None => Style::default(),
        };
        match (self.mode, emphasis) {
            (ColorMode::None, Emphasis::Plain) => Style::default(),
            (ColorMode::None, e) => Style::default().add_modifier(e.modifier()),
            (_, Emphasis::Plain) => coloured,
            (_, e) => coloured.add_modifier(e.modifier()),
        }
    }

    /// Ordinary prose.
    pub fn text(&self) -> Style {
        self.role(Role::text, Emphasis::Plain)
    }
    /// Secondary information: hints, timestamps, units.
    pub fn dim(&self) -> Style {
        self.role(Role::text_dim, Emphasis::Plain)
    }
    /// Tertiary information: labels, separators, anything skippable.
    pub fn faint(&self) -> Style {
        self.role(Role::text_faint, Emphasis::Plain)
    }
    /// What the user typed.
    pub fn user(&self) -> Style {
        self.role(Role::user, Emphasis::Plain)
    }
    /// What the model said.
    pub fn assistant(&self) -> Style {
        self.role(Role::assistant, Emphasis::Plain)
    }
    /// Reasoning traces, which should read as background.
    pub fn thinking(&self) -> Style {
        self.role(Role::thinking, Emphasis::Muted)
    }
    /// A completed action.
    pub fn success(&self) -> Style {
        self.role(Role::success, Emphasis::Plain)
    }
    /// A failed action, or a destructive option.
    pub fn danger(&self) -> Style {
        self.role(Role::danger, Emphasis::Plain)
    }
    /// Something the reader should act on.
    pub fn warn(&self) -> Style {
        self.role(Role::warn, Emphasis::Plain)
    }
    /// The primary affordance.
    pub fn accent(&self) -> Style {
        self.role(Role::accent, Emphasis::Plain)
    }
    /// Resting borders.
    pub fn border(&self) -> Style {
        self.role(Role::border, Emphasis::Plain)
    }
    /// The border of whatever owns the keyboard.
    pub fn border_focus(&self) -> Style {
        self.role(Role::border_focus, Emphasis::Focus)
    }
    /// Titles and section labels.
    pub fn title(&self) -> Style {
        self.role(Role::text_dim, Emphasis::Strong)
    }
    /// The selected row of a list. Always pair with a glyph: selection must
    /// survive `NO_COLOR`, and a style-only selection is invisible without it.
    pub fn selected(&self) -> Style {
        self.role(Role::border_focus, Emphasis::Focus)
    }
    /// Whether filled bands are available. A widget that relies on one for
    /// legibility must fall back to a gutter glyph when this is false.
    pub fn has_surfaces(&self) -> bool {
        self.mode == ColorMode::Truecolor
    }
    /// The band behind a user message, or nothing when colour is off.
    fn surface(&self, role: Role) -> Style {
        if self.mode != ColorMode::Truecolor {
            return Style::default();
        }
        let slot = Palette::DARK.get(role);
        Style::default().bg(Color::Rgb(slot.rgb.0, slot.rgb.1, slot.rgb.2))
    }
    /// The band behind a user message.
    pub fn surface_user(&self) -> Style {
        self.surface(Role::surface_user)
    }
    /// The band behind a tool call awaiting a decision.
    pub fn surface_pending(&self) -> Style {
        self.surface(Role::surface_pending)
    }
    /// The band behind a tool call that succeeded.
    pub fn surface_ok(&self) -> Style {
        self.surface(Role::surface_ok)
    }
    /// The band behind a tool call that failed.
    pub fn surface_err(&self) -> Style {
        self.surface(Role::surface_err)
    }
}

/// How many terminal columns a string occupies.
///
/// Not `chars().count()`. Use this for every width decision: layout maths,
/// truncation, and "does this fit" checks.
pub fn display_width(text: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    text.width()
}

/// Trim a styled line to `width` columns, keeping the **last** characters.
///
/// For a status row written most-important-last, so the leading fragments give
/// way and the model name survives. Widening the other way keeps `12k` and loses
/// the one fragment that says what every other number means.
pub fn trim_keeping_tail(
    spans: Vec<ratatui::text::Span<'static>>,
    width: usize,
) -> ratatui::text::Line<'static> {
    use ratatui::text::{Line, Span};
    use unicode_width::UnicodeWidthChar;

    let total: usize = spans.iter().map(|s| display_width(&s.content)).sum();
    if total <= width {
        return Line::from(spans);
    }
    let mut skip = total - width;
    let mut out: Vec<Span<'static>> = Vec::new();
    for span in spans {
        if skip == 0 {
            out.push(span);
            continue;
        }
        let span_width = display_width(&span.content);
        if skip >= span_width {
            skip -= span_width;
            continue;
        }
        let mut kept = String::new();
        let mut dropped = 0;
        for ch in span.content.chars() {
            if dropped < skip {
                dropped += ch.width().unwrap_or(0);
                continue;
            }
            kept.push(ch);
        }
        skip = 0;
        if !kept.is_empty() {
            out.push(Span::styled(kept, span.style));
        }
    }
    Line::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_colour_still_distinguishes_roles_by_attribute() {
        let theme = Theme::new(ColorMode::None);
        assert_ne!(
            theme.thinking().add_modifier,
            Modifier::empty(),
            "without colour, reasoning must not collapse into ordinary prose"
        );
        assert_ne!(
            theme.selected().add_modifier,
            Modifier::empty(),
            "and selection must stay visible"
        );
    }

    #[test]
    fn bands_are_withheld_when_the_terminal_cannot_do_them() {
        let plain = Theme::new(ColorMode::Ansi16);
        assert!(!plain.has_surfaces());
        assert_eq!(plain.surface_user(), Style::default());
        let rich = Theme::new(ColorMode::Truecolor);
        assert!(rich.has_surfaces());
        assert_ne!(rich.surface_user(), Style::default());
    }

    #[test]
    fn widths_are_columns_not_characters() {
        // Each of these is one `char`; only the last is two columns.
        assert_eq!(display_width("⠹"), 1);
        assert_eq!(display_width("─"), 1);
        assert_eq!(display_width("界"), 2);
        assert_ne!(display_width("界"), "界".chars().count());
    }

    #[test]
    fn trimming_a_status_row_keeps_its_tail() {
        let line = trim_keeping_tail(
            vec![
                ratatui::text::Span::raw("12k tokens "),
                ratatui::text::Span::raw("DeepSeek V3.1"),
            ],
            10,
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.ends_with("V3.1"), "{text:?}");
        assert_eq!(display_width(&text), 10);
    }
}
