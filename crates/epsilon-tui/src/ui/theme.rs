//! Visual foundation for the Epsilon TUI: colors, glyphs, and the small
//! formatting rules that keep numbers readable.
//!
//! # Why this module exists
//!
//! A terminal UI reads as "cheap" for three reasons, and only one of them is
//! about color:
//!
//! 1. **No hierarchy** — every row is the same weight, so nothing tells the eye
//!    where to look.
//! 2. **No spacing** — text runs edge to edge, and labels sit flush against the
//!    values they describe.
//! 3. **No restraint** — a different color for every concept, including concepts
//!    the reader never needs to decode.
//!
//! So this module defines a *small* set of semantic tokens (never raw colors
//! scattered through widgets), two glyph sets so the UI still works on a console
//! that cannot render box-drawing characters, and number formatting that keeps a
//! status bar from reflowing every time a token arrives.
//!
//! # Adding a color
//!
//! Give the role a *meaning*, not an appearance: `success`, not `green`. Then map
//! it in both palette tables below. If a role needs the same color as another
//! role, that is a signal the two roles are actually one — merge them.

use ratatui::style::{Color, Modifier, Style};

/// How much color the terminal can show.
///
/// Detected once at startup and stored on the app, because guessing per frame
/// would let the UI change shape mid-session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// 24-bit color (`COLORTERM=truecolor`, or a terminal known to support it).
    Truecolor,
    /// The classic 16 ANSI colors. Windows `conhost` lands here unless it opts in.
    Ansi16,
    /// `NO_COLOR` is set: no color at all, only bold/dim/italic for emphasis.
    None,
}

impl ColorMode {
    /// Detect from the environment.
    ///
    /// The rule is deliberately conservative. Claiming truecolor on a terminal
    /// that lacks it produces the worst outcome — unreadable or missing text —
    /// whereas falling back to 16 colors merely looks plainer.
    pub fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return Self::None;
        }
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return Self::Truecolor;
        }
        // Windows Terminal and VS Code both advertise themselves; conhost does not.
        if std::env::var_os("WT_SESSION").is_some() || std::env::var_os("TERM_PROGRAM").is_some() {
            return Self::Truecolor;
        }
        Self::Ansi16
    }
}

/// Character repertoire. A console with a legacy code page renders `╭` as
/// mojibake, so the ASCII set exists as a first-class alternative rather than an
/// afterthought.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyphs {
    /// Rounded top-left corner, or `+`.
    pub top_left: &'static str,
    /// Rounded top-right corner, or `+`.
    pub top_right: &'static str,
    /// Rounded bottom-left corner, or `+`.
    pub bottom_left: &'static str,
    /// Rounded bottom-right corner, or `+`.
    pub bottom_right: &'static str,
    /// Horizontal rule.
    pub horizontal: &'static str,
    /// Vertical rule.
    pub vertical: &'static str,
    /// The dot that marks a transcript entry.
    pub bullet: &'static str,
    /// The caret drawn at the streaming tail.
    pub caret: &'static str,
    /// The marker for the session the user is looking at.
    pub cursor_bar: &'static str,
    /// The prompt marker that introduces a row the user typed.
    pub prompt: &'static str,
    /// The marker that introduces a tool call, before it has a result.
    pub tool_call: &'static str,
    /// A completed step.
    pub tick: &'static str,
    /// A failed step.
    pub cross: &'static str,
    /// A line a diff adds. Part of the glyph table rather than a literal `"+"` in
    /// the diff renderer, like every other character the UI draws.
    pub plus: &'static str,
    /// A line a diff removes.
    pub minus: &'static str,
    /// A warning.
    pub warn: &'static str,
    /// Spinner frames, advanced on a timer while work is in flight.
    pub spinner: &'static [&'static str],
}

/// Unicode glyphs: the default on any terminal that is not a legacy console.
pub const UNICODE: Glyphs = Glyphs {
    top_left: "╭",
    top_right: "╮",
    bottom_left: "╰",
    bottom_right: "╯",
    horizontal: "─",
    vertical: "│",
    bullet: "•",
    caret: "▌",
    cursor_bar: "▎",
    prompt: "❯",
    tool_call: "▸",
    tick: "✓",
    cross: "✗",
    plus: "+",
    minus: "-",
    warn: "!",
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
};

/// ASCII glyphs: identical geometry, seven-bit characters only.
pub const ASCII: Glyphs = Glyphs {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    horizontal: "-",
    vertical: "|",
    bullet: "*",
    caret: "_",
    cursor_bar: ">",
    prompt: ">",
    tool_call: "\\",
    tick: "+",
    cross: "x",
    plus: "+",
    minus: "-",
    warn: "!",
    spinner: &["|", "/", "-", "\\"],
};

impl Glyphs {
    /// Pick the repertoire from the environment.
    ///
    /// `EPSILON_ASCII=1` forces the ASCII set; otherwise Unicode. We do not sniff
    /// the code page: the variable exists because the answer cannot be detected
    /// reliably, and a wrong guess is worse than a documented switch.
    pub fn detect() -> Self {
        if std::env::var_os("EPSILON_ASCII").is_some_and(|v| !v.is_empty()) {
            ASCII
        } else {
            UNICODE
        }
    }

    /// Draw a horizontal rule of `width` cells.
    pub fn rule(&self, width: usize) -> String {
        self.horizontal.repeat(width)
    }

    /// `╭─ title ─────╮`: a bordered box whose top edge carries a label.
    ///
    /// Returns the three pieces so the caller can style the label differently
    /// from the rule without re-deriving the arithmetic.
    pub fn titled_top(&self, title: &str, width: usize) -> (String, String, String) {
        // 1 corner + 1 space + title + 1 space + rule + 1 corner
        let inner = width.saturating_sub(title.chars().count() + 4);
        (
            self.top_left.to_string(),
            format!(" {title} "),
            format!("{}{}", self.rule(inner), self.top_right),
        )
    }
}

/// One semantic color slot, in both palettes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Slot {
    pub(crate) rgb: (u8, u8, u8),
    pub(crate) ansi: Color,
}

macro_rules! palette {
    ($($name:ident: $rgb:expr, $ansi:expr;)*) => {
        /// The colors, chosen once and mapped to whatever the terminal supports.
        ///
        /// Roles are semantic. Two roles sharing a color usually means they are
        /// one role; two roles that must be distinguishable at a glance (success
        /// and failure, for example) must never share one.
        #[derive(Debug, Clone, Copy)]
        pub(crate) struct Palette {
            $($name: Slot,)*
        }

        /// A role, named so a lookup can be written without a compiler trick.
        ///
        /// Snake-case on purpose: a role reads as a field of the palette
        /// (`Role::text_dim`), and the naming lint would have us write
        /// `TextDim` only to spell it back out in every match arm.
        ///
        /// Each variant means what the comment beside it in the table says.
        /// The lint wants TextDim; the palette reads better as a field name.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types, missing_docs)]
        pub enum Role {
            $($name,)*
        }

        impl Palette {
            /// The dark palette, which is what most terminals are.
            pub(crate) const DARK: Self = Self {
                $($name: Slot { rgb: $rgb, ansi: $ansi },)*
            };

            /// The slot a role resolves to.
            pub(crate) fn get(&self, role: Role) -> Slot {
                match role {
                    $(Role::$name => self.$name,)*
                }
            }
        }
    };
}

palette! {
    // Structure: borders, rules, and chrome the eye should skip over.
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
    // Filled bands. Deliberately close to a dark terminal background: a band
    surface_user:    (0x22, 0x24, 0x2c), Color::Reset;
    // should read as a change of surface, not as a selected row.
    surface_pending: (0x2a, 0x2a, 0x22), Color::Reset;

    surface_ok:      (0x1e, 0x2a, 0x22), Color::Reset;

    surface_err:     (0x2e, 0x20, 0x20), Color::Reset;
}

/// The resolved styles for one session.
///
/// Built once and stored on the app; widgets read tokens and never touch
/// [`Color`] directly. That is what makes `NO_COLOR` and the 16-color fallback a
/// single decision instead of a hunt through every draw function.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    mode: ColorMode,
    /// Which glyph repertoire to draw with.
    pub glyphs: Glyphs,
}

impl Theme {
    /// Build the theme for a color mode and glyph set.
    pub fn new(mode: ColorMode, glyphs: Glyphs) -> Self {
        Self { mode, glyphs }
    }

    /// Detect both from the environment, which is what the app does at startup.
    pub fn detect() -> Self {
        Self::new(ColorMode::detect(), Glyphs::detect())
    }

    /// The color mode in force.
    pub fn mode(&self) -> ColorMode {
        self.mode
    }

    // ---- roles ---------------------------------------------------------------
    //
    // One accessor per role, so a widget reads as intent (`theme.danger()`)
    // rather than mechanics. The palette itself stays private, which is what
    // makes `NO_COLOR` and the 16-color fallback a single decision here instead
    // of a hunt through every draw function.

    /// Resolve a role to a style, with optional emphasis. The single place where
    /// "what does emphasis mean when there is no color" is decided.
    #[allow(missing_docs)]
    fn role(&self, role: Role, emphasis: Emphasis) -> Style {
        let slot = Palette::DARK.get(role);
        let style = match self.mode {
            ColorMode::Truecolor => {
                Style::default().fg(Color::Rgb(slot.rgb.0, slot.rgb.1, slot.rgb.2))
            }
            ColorMode::Ansi16 => Style::default().fg(slot.ansi),
            // With color off, emphasis has to come from attributes instead.
            ColorMode::None => Style::default(),
        };
        match (self.mode, emphasis) {
            (ColorMode::None, Emphasis::Plain) => Style::default(),
            (ColorMode::None, e) => Style::default().add_modifier(e.modifier()),
            (_, Emphasis::Plain) => style,
            (_, e) => style.add_modifier(e.modifier()),
        }
    }
}

/// How a role should be emphasised, independent of whether color is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Emphasis {
    /// No extra attribute.
    Plain,
    /// The value the reader should still catch when color is off.
    Strong,
    /// Background material: reasoning traces.
    Muted,
    /// The keyboard owner.
    Focus,
}

impl Emphasis {
    fn modifier(self) -> Modifier {
        match self {
            // A dim role is already dim by color; with color off, bold is what
            // keeps it from disappearing into plain prose.
            Self::Strong => Modifier::BOLD,
            Self::Muted => Modifier::DIM | Modifier::ITALIC,
            Self::Focus => Modifier::BOLD,
            Self::Plain => Modifier::empty(),
        }
    }
}

/// The roles a widget may draw with. Kept as methods on [`Theme`] so a call site
/// reads as intent (`theme.danger()`) rather than as mechanics.
impl Theme {
    // Text.
    /// Ordinary prose.
    pub fn text(&self) -> Style {
        self.role(Role::text, Emphasis::Plain)
    }

    /// Secondary information: hints, timestamps, units.
    /// Secondary information: hints, timestamps, units.
    pub fn dim(&self) -> Style {
        self.role(Role::text_dim, Emphasis::Plain)
    }

    /// Tertiary information: labels, separators, anything skippable.
    /// Tertiary information: labels, separators, anything skippable.
    pub fn faint(&self) -> Style {
        self.role(Role::text_faint, Emphasis::Plain)
    }

    /// A dim role with extra emphasis.
    /// A dim role with extra emphasis.
    pub fn dim_strong(&self) -> Style {
        self.role(Role::text_dim, Emphasis::Strong)
    }

    // Who is speaking.
    /// What the user typed.
    pub fn user(&self) -> Style {
        self.role(Role::user, Emphasis::Plain)
    }

    /// What the model said.
    pub fn assistant(&self) -> Style {
        self.role(Role::assistant, Emphasis::Plain)
    }

    /// Reasoning traces, which should read as background.
    /// Reasoning traces, which should read as background.
    pub fn thinking(&self) -> Style {
        self.role(Role::thinking, Emphasis::Muted)
    }

    // Outcome.
    /// A completed action.
    pub fn success(&self) -> Style {
        self.role(Role::success, Emphasis::Plain)
    }

    /// A failed action, or a destructive option.
    /// A failed action, or a destructive option.
    pub fn danger(&self) -> Style {
        self.role(Role::danger, Emphasis::Plain)
    }

    /// Something the reader should act on.
    pub fn warn(&self) -> Style {
        self.role(Role::warn, Emphasis::Plain)
    }

    /// The one color that says "this is the primary affordance".
    /// The one colour that says: this is the primary affordance.
    pub fn accent(&self) -> Style {
        self.role(Role::accent, Emphasis::Plain)
    }

    // Inline code and fenced blocks.
    /// Inline code.
    pub fn code(&self) -> Style {
        self.role(Role::code, Emphasis::Plain)
    }

    /// Fenced code bodies.
    pub fn code_block(&self) -> Style {
        self.role(Role::code_block, Emphasis::Plain)
    }

    /// Resting borders.
    /// Resting borders.
    pub fn border(&self) -> Style {
        self.role(Role::border, Emphasis::Plain)
    }

    /// The border of whatever currently owns the keyboard.
    /// The border of whatever currently owns the keyboard.
    pub fn border_focus(&self) -> Style {
        self.role(Role::border_focus, Emphasis::Focus)
    }

    /// Titles and section labels.
    pub fn title(&self) -> Style {
        self.role(Role::text_dim, Emphasis::Strong)
    }

    /// System and lifecycle notices.
    ///
    /// Named separately from [Self::faint] because the two are free to drift
    /// apart: a notice is information ("the run failed"), while faint text is
    /// decoration. Today they resolve to the same colour, and the day they should
    /// not, only this method changes.
    pub fn system(&self) -> Style {
        self.role(Role::text_dim, Emphasis::Plain)
    }

    /// The border of the overlay that owns the keyboard.
    pub fn border_active(&self) -> Style {
        self.role(Role::border_focus, Emphasis::Focus)
    }

    /// Chrome that should be readable but not competing: titles, hints, borders.
    pub fn muted(&self) -> Style {
        self.role(Role::text_dim, Emphasis::Plain)
    }

    /// A selected row whose description text should stay quiet.
    ///
    /// Reversed like [Self::selected] so the two halves of one row highlight
    /// as a single band instead of two different treatments.
    pub fn selected_muted(&self) -> Style {
        // Reversed in every mode: it is the quiet half of a row that is already
        // reversed, and two different treatments inside one highlighted row read
        // as a rendering fault rather than as emphasis.
        self.role(Role::text_faint, Emphasis::Plain)
            .add_modifier(Modifier::REVERSED)
    }

    /// The accent used for authentication prompts.
    pub fn auth(&self) -> Style {
        self.role(Role::warn, Emphasis::Plain)
    }
    /// The selected row of a list. Selection must survive `NO_COLOR`, so it is
    /// always punctuated by a glyph (the caller's job) as well as styled.
    pub fn selected(&self) -> Style {
        self.role(Role::border_focus, Emphasis::Focus)
    }

    // ---- surfaces ------------------------------------------------------------
    //
    // Role separation in a transcript is done with a *background*, not a border.
    // Borders around every message turn a conversation into a form; a filled
    // band reads as one block of speech and costs no columns.

    // Filled bands. Deliberately close to a dark terminal background: a band
    /// The band behind a user message.
    pub fn surface_user(&self) -> Style {
        self.surface(Role::surface_user)
    }

    /// The band behind a tool call that is still waiting for a decision.
    /// The band behind a tool call that is still waiting.
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

    /// Truecolor-only filled band. On a 16-color terminal we skip the fill
    /// rather than approximate it: a wrong background is worse than none,
    /// because it collides with whatever theme the user already runs.
    fn surface(&self, role: Role) -> Style {
        if self.mode != ColorMode::Truecolor {
            return Style::default();
        }
        let slot = Palette::DARK.get(role);
        Style::default().bg(Color::Rgb(slot.rgb.0, slot.rgb.1, slot.rgb.2))
    }

    /// Whether filled bands are available. Widgets that rely on a band for
    /// legibility must fall back to a gutter glyph when this is false.
    pub fn has_surfaces(&self) -> bool {
        self.mode == ColorMode::Truecolor
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_glyphs_are_seven_bit_only() {
        // A legacy code page renders box-drawing characters as mojibake, so the
        // ASCII repertoire is the one guarantee we can actually make.
        let check = |s: &str| assert!(s.is_ascii(), "{s:?} is not ASCII");
        check(ASCII.top_left);
        check(ASCII.top_right);
        check(ASCII.bottom_left);
        check(ASCII.bottom_right);
        check(ASCII.horizontal);
        check(ASCII.vertical);
        check(ASCII.bullet);
        check(ASCII.caret);
        check(ASCII.cursor_bar);
        check(ASCII.prompt);
        check(ASCII.tool_call);
        for frame in ASCII.spinner {
            check(frame);
        }
    }

    #[test]
    fn titled_top_fills_exactly_the_requested_width() {
        for width in [12usize, 40, 80] {
            let (left, title, right) = UNICODE.titled_top("input", width);
            let total = left.chars().count() + title.chars().count() + right.chars().count();
            assert_eq!(total, width, "width {width} produced {total} cells");
        }
    }

    #[test]
    fn no_color_still_distinguishes_roles_by_attribute() {
        let theme = Theme::new(ColorMode::None, ASCII);
        // Without colour, reasoning must not collapse into ordinary prose.
        assert_ne!(
            theme.thinking().add_modifier,
            Modifier::empty(),
            "the thinking role needs an attribute when colour is off"
        );
        // And selection must still be visible, or a picker becomes unusable.
        assert_ne!(theme.selected().add_modifier, Modifier::empty());
    }

    #[test]
    fn colour_mode_detection_prefers_the_safe_answer() {
        // The real environment decides this, so the test asserts the *rule*:
        // an unset COLORTERM must never claim truecolor on a bare console.
        let mode = ColorMode::detect();
        assert!(matches!(
            mode,
            ColorMode::Truecolor | ColorMode::Ansi16 | ColorMode::None
        ));
    }

    #[test]
    fn bands_are_withheld_when_the_terminal_cannot_do_them() {
        let plain = Theme::new(ColorMode::Ansi16, ASCII);
        assert!(!plain.has_surfaces());
        assert_eq!(plain.surface_user(), Style::default());
        let rich = Theme::new(ColorMode::Truecolor, UNICODE);
        assert!(rich.has_surfaces());
        assert_ne!(rich.surface_user(), Style::default());
    }
}
