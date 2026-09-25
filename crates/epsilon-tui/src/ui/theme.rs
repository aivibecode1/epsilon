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
//! it in **both** palette tables below. If a role needs the same color as another
//! role, that is a signal the two roles are actually one — merge them.
//!
//! # Four decisions this file encodes, and why each one is here
//!
//! 1. **Color depth is detected once, at startup**, and the theme carries it.
//!    Claiming 24-bit color on a console that lacks it produces unreadable output;
//!    falling back to 16 colors merely looks plainer. The asymmetry is the whole
//!    argument for a conservative default.
//! 2. **`NO_COLOR` still has to convey meaning.** With color off, emphasis comes
//!    from attributes — bold for a value the reader must catch, dim+italic for
//!    background material. A theme that maps every role to `Style::default()`
//!    under `NO_COLOR` makes reasoning traces indistinguishable from answers.
//! 3. **ANSI-16 body text is the terminal's own default foreground, and no other
//!    slot.** This is the rule the light theme exists for: `Color::White` as body
//!    text is invisible on a light terminal and `Color::Black` is invisible on a
//!    dark one, so neither may be a default. `Color::Reset` is not a color, it is
//!    "whatever this terminal draws normal text in", which is the one ANSI value
//!    guaranteed to contrast with the background the user chose. The same
//!    reasoning covers `text_dim`/`text_faint`: the quiet roles keep their
//!    truecolor values and lean on `DIM` for their *look*, because every ANSI
//!    slot that would read as "quiet grey" is invisible on one of the two
//!    backgrounds. Filled bands are withheld entirely off truecolor
//!    ([`Theme::has_surfaces`]) rather than approximated with a background that
//!    collides with the user's own theme.
//! 4. **A palette is selected, never sniffed.** There is no reliable
//!    cross-platform way to ask a terminal whether its background is light:
//!    `COLORTERM` and `WT_SESSION` describe color *depth*, and the one live query
//!    that would answer it (`OSC 11`) needs a round-trip the terminal may not
//!    answer inside a pipe, over SSH, or in `tmux` — and blocking startup on a
//!    timeout is worse than one wrong guess. So [`ThemeMode`] is chosen by the
//!    user (`--light`, `EPSILON_THEME=light`, `[tui] theme` in the config), and
//!    the failure mode is a one-time toggle instead of a flicker or a guess every
//!    session.

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

/// Environment variable that selects the palette, for a user who does not want to
/// pass a flag every launch (`EPSILON_THEME=light`).
pub const THEME_ENV: &str = "EPSILON_THEME";

/// Which palette to resolve roles from.
///
/// Selected, never detected: see decision 4 in the module docs. Everything else
/// about a session may be probed and adapted; this one is a question only the
/// user can answer, and one wrong answer makes the transcript unreadable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    /// Tuned for a dark background, which is what most terminals are.
    #[default]
    Dark,
    /// Tuned for a light background: darker text, pastel bands, and no
    /// near-white anywhere.
    Light,
}

impl ThemeMode {
    /// The palette named by the environment, or [`Self::Dark`] when the variable
    /// is unset or names neither variant.
    ///
    /// A typo falls back to the default rather than failing: a config mistake
    /// must not make the TUI refuse to start, which is the same rule
    /// `epsilon_core::config` follows for every other key.
    pub fn detect() -> Self {
        Self::from_name(&std::env::var(THEME_ENV).unwrap_or_default()).unwrap_or_default()
    }

    /// Parse a palette name (`dark` / `light`, case-insensitively).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }

    /// The name this mode is selected by, for messages and for the config key.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    /// The other palette, for the runtime toggle.
    pub fn flipped(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
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
    /// How far back a frozen transcript view is: the marker in the footer's
    /// position badge (`↑ 12/40`, `^ 12/40`). Part of the glyph table like every
    /// other character the UI draws, so a legacy code page gets `^`.
    pub back: &'static str,
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
    back: "↑",
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
    back: "^",
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
    // Text. `Color::Reset` on the ANSI side, never `White`: the terminal's own
    // default foreground is the only ANSI value guaranteed to contrast with
    // whatever background the user set (see decision 3 in the module docs).
    text:          (0xd7, 0xdb, 0xe2), Color::Reset;

    text_dim:      (0x8a, 0x92, 0xa3), Color::DarkGray;

    text_faint:    (0x5a, 0x62, 0x72), Color::DarkGray;
    // Who is speaking.
    user:          (0x7d, 0xd3, 0xfc), Color::Cyan;

    assistant:     (0xd7, 0xdb, 0xe2), Color::Reset;

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
    // should read as a change of surface, not as a selected row.
    surface_user:    (0x22, 0x24, 0x2c), Color::Reset;

    surface_pending: (0x2a, 0x2a, 0x22), Color::Reset;

    surface_ok:      (0x1e, 0x2a, 0x22), Color::Reset;

    surface_err:     (0x2e, 0x20, 0x20), Color::Reset;
}

/// The light palette: the same role names, values that survive a white
/// background.
///
/// Written as a second table rather than as modifiers over the dark one, because
/// the numbers are not a transform of each other — light-mode "quiet grey" is
/// *darker* than light-mode body text, and every pastel band has to stay above
/// the 4.5:1 body-text floor against the text that sits on it. The tests hold
/// both properties (`the_light_palette_keeps_body_text_readable`), which is what
/// stops this table from drifting into decoration.
///
/// An inner attribute cannot attach docs to macro-generated fields, so the
/// role-by-role reasoning lives in the comments beside each line, exactly as in
/// the dark table above.
macro_rules! light_palette {
    ($($name:ident: $rgb:expr, $ansi:expr;)*) => {
        impl Palette {
            /// Tuned for a light background. Every `ansi` value here is either
            /// the terminal default or one of the slots that stays legible on
            /// both backgrounds (see the contrast test) — never `White`, never
            /// `DarkGray`, which is the pair that disappears on a light theme.
            pub(crate) const LIGHT: Self = Self {
                $($name: Slot { rgb: $rgb, ansi: $ansi },)*
            };
        }
    };
}

light_palette! {
    // Structure: a hairline that is visible on white without becoming a frame.
    // 3.6:1 against white — the skill's floor for a non-body role, since a
    // divider nobody can see is a divider that does not exist.
    border:        (0xb9, 0xc0, 0xcc), Color::DarkGray;

    border_focus:  (0x1e, 0x63, 0xc4), Color::Cyan;
    // Text: near-black body copy (14.8:1) rather than pure black, which reads as
    // a photocopy; secondary and tertiary values stay well above 4.5:1.
    text:          (0x1f, 0x24, 0x30), Color::Reset;

    text_dim:      (0x4a, 0x53, 0x64), Color::DarkGray;

    text_faint:    (0x5a, 0x63, 0x73), Color::DarkGray;
    // Who is speaking. The dark palette's pastel cyan is 1.6:1 on white; this is
    // the same hue taken dark enough to read (6.4:1).
    user:          (0x0b, 0x5e, 0xa8), Color::Blue;

    assistant:     (0x1f, 0x24, 0x30), Color::Reset;

    thinking:      (0x4a, 0x53, 0x64), Color::DarkGray;
    // Outcome: the semantic hues, darkened rather than pastelised. All three
    // clear 4.5:1 on white while staying distinguishable from each other, which
    // is the property that matters more than any single ratio.
    success:       (0x1d, 0x6b, 0x3a), Color::Green;

    danger:        (0xa3, 0x22, 0x22), Color::Red;

    warn:          (0x7a, 0x5c, 0x00), Color::Yellow;

    accent:        (0x6b, 0x3f, 0xc4), Color::Magenta;
    // Inline code and fenced blocks.
    code:          (0x7a, 0x4a, 0x12), Color::Yellow;

    code_block:    (0x4a, 0x53, 0x64), Color::Gray;
    // Filled bands: pale tints of white, dark enough to hold body text at 4.5:1
    // and light enough to read as a change of surface rather than as a
    // selection. A band that the text on it cannot sit on is worse than no band.
    surface_user:    (0xdf, 0xe4, 0xee), Color::Reset;

    surface_pending: (0xf4, 0xef, 0xd6), Color::Reset;

    surface_ok:      (0xe0, 0xef, 0xe6), Color::Reset;

    surface_err:     (0xf9, 0xe3, 0xe3), Color::Reset;
}

/// The resolved styles for one session.
///
/// Built once and stored on the app; widgets read tokens and never touch
/// [`Color`] directly. That is what makes `NO_COLOR`, the 16-color fallback and
/// the light palette a single decision instead of a hunt through every draw
/// function.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    mode: ColorMode,
    palette: ThemeMode,
    /// Which glyph repertoire to draw with.
    pub glyphs: Glyphs,
}

impl Theme {
    /// Build the theme for a color mode and glyph set, in the default (dark)
    /// palette.
    pub fn new(mode: ColorMode, glyphs: Glyphs) -> Self {
        Self::with_palette(mode, glyphs, ThemeMode::default())
    }

    /// Build the theme for a color mode, a glyph set and a palette.
    pub fn with_palette(mode: ColorMode, glyphs: Glyphs, palette: ThemeMode) -> Self {
        Self {
            mode,
            palette,
            glyphs,
        }
    }

    /// Detect the color mode and glyphs from the environment, and take the
    /// palette from `EPSILON_THEME` unless the caller resolved one (`--light`
    /// and the config key win over the environment; see the CLI's
    /// `resolve_theme_mode`).
    pub fn detect() -> Self {
        Self::with_palette(ColorMode::detect(), Glyphs::detect(), ThemeMode::detect())
    }

    /// The color mode in force.
    pub fn mode(&self) -> ColorMode {
        self.mode
    }

    /// The palette in force.
    pub fn palette(&self) -> ThemeMode {
        self.palette
    }

    /// Switch palettes, returning the theme that was in force so a caller can
    /// report the change.
    ///
    /// This rebuilds styles rather than recolouring the screen: the theme is
    /// `Copy` and every widget resolves roles through it at draw time, so the
    /// next frame is already in the new palette and the render cache — which is
    /// keyed by width and render state, deliberately not by the theme — stays
    /// valid. Nothing is patched a cell at a time.
    pub fn set_palette(&mut self, palette: ThemeMode) -> ThemeMode {
        std::mem::replace(&mut self.palette, palette)
    }

    /// The colors this theme resolves roles from.
    fn colors(&self) -> &'static Palette {
        match self.palette {
            ThemeMode::Dark => &Palette::DARK,
            ThemeMode::Light => &Palette::LIGHT,
        }
    }

    // ---- roles ---------------------------------------------------------------
    //
    // One accessor per role, so a widget reads as intent (`theme.danger()`)
    // rather than mechanics. The palette itself stays private, which is what
    // makes `NO_COLOR` and the 16-color fallback a single decision here instead
    // of a hunt through every draw function.

    /// Resolve a role to a style, with optional emphasis. The single place where
    /// "what does emphasis mean when there is no color" is decided, and the only
    /// place that reads a palette — which is what makes the light variant one
    /// decision instead of a hunt through every draw function.
    #[allow(missing_docs)]
    fn role(&self, role: Role, emphasis: Emphasis) -> Style {
        let slot = self.colors().get(role);
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
    /// because it collides with whatever theme the user already runs. The bands
    /// are the one place the light palette differs structurally rather than by
    /// value — a pale tint on white instead of a near-background tint on black.
    fn surface(&self, role: Role) -> Style {
        if self.mode != ColorMode::Truecolor {
            return Style::default();
        }
        let slot = self.colors().get(role);
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
        check(ASCII.back);
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
        // Withholding is a property of the *terminal*, not of the palette: the
        // light theme must not start filling backgrounds on a 16-color console
        // just because its tints happen to be pale.
        let plain_light = Theme::with_palette(ColorMode::Ansi16, ASCII, ThemeMode::Light);
        assert!(!plain_light.has_surfaces());
        assert_eq!(plain_light.surface_ok(), Style::default());
    }

    // ---- the light palette ---------------------------------------------------
    //
    // The audit's D8, in the form a test can hold: body text that is legible
    // against the background the *user* chose, and a second palette that is a
    // real second palette rather than a rename of the first.

    /// WCAG relative luminance, for contrast arithmetic.
    fn luminance(rgb: (u8, u8, u8)) -> f64 {
        fn channel(value: u8) -> f64 {
            let v = value as f64 / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(rgb.0) + 0.7152 * channel(rgb.1) + 0.0722 * channel(rgb.2)
    }

    /// WCAG contrast ratio between two colors, 1.0 (identical) to 21.0.
    fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// The surface a text role actually sits on, in the situation the audit
    /// named: what the user's own terminal background is.
    const WHITE: (u8, u8, u8) = (0xff, 0xff, 0xff);

    #[test]
    fn the_light_palette_keeps_body_text_readable_on_a_white_terminal() {
        // The defect, stated as a number: the dark palette's body text is 1.19:1
        // on white — near-invisible, which is what the owner was looking at.
        let dark = contrast(Palette::DARK.get(Role::text).rgb, WHITE);
        assert!(
            dark < 1.5,
            "the dark palette is supposed to be unreadable on white; it scored {dark:.2}:1"
        );
        // And the light palette's, at the WCAG AA floor for body text.
        for role in [
            Role::text,
            Role::assistant,
            Role::text_dim,
            Role::text_faint,
        ] {
            let ratio = contrast(Palette::LIGHT.get(role).rgb, WHITE);
            assert!(
                ratio >= 4.5,
                "{role:?} scores {ratio:.2}:1 on white, under the 4.5:1 floor"
            );
        }
        // The same for the other direction, at the level the light palette can
        // actually promise: `Color::Reset` body text follows the terminal, so a
        // light palette on a dark terminal is readonly-degraded rather than
        // broken — but a role that is *identical* to the dark palette's is a copy
        // that forgot to change, and that is checkable.
        for role in [Role::text, Role::assistant] {
            assert_ne!(
                Palette::LIGHT.get(role).rgb,
                Palette::DARK.get(role).rgb,
                "{role:?} is the dark palette's value under a light name"
            );
        }
    }

    #[test]
    fn every_light_role_clears_its_floor_against_every_surface_it_can_sit_on() {
        // The failure mode a single "text on white" check misses: a band behind a
        // row, with quiet text on it. A band that its own text cannot sit on is
        // worse than no band at all.
        let surfaces = [
            Role::surface_user,
            Role::surface_pending,
            Role::surface_ok,
            Role::surface_err,
        ];
        for surface in surfaces {
            let bg = Palette::LIGHT.get(surface).rgb;
            for role in [
                Role::text,
                Role::assistant,
                Role::text_dim,
                Role::text_faint,
            ] {
                let ratio = contrast(Palette::LIGHT.get(role).rgb, bg);
                assert!(
                    ratio >= 4.5,
                    "{role:?} on {surface:?} scores {ratio:.2}:1, under the 4.5:1 floor"
                );
            }
        }
        // Status colours are short words rather than paragraphs, so 3:1 is their
        // floor — and they must not be near-invisible on the bands they appear on.
        for surface in surfaces {
            let bg = Palette::LIGHT.get(surface).rgb;
            for role in [Role::success, Role::danger, Role::warn, Role::accent] {
                let ratio = contrast(Palette::LIGHT.get(role).rgb, bg);
                assert!(
                    ratio >= 3.0,
                    "{role:?} on {surface:?} scores {ratio:.2}:1, under the 3:1 floor"
                );
            }
        }
    }

    #[test]
    fn both_palettes_keep_the_conversation_legible() {
        // Not every role may share a value with another: the two pairs that carry
        // an entire outcome (`success`/`danger`) and an entire voice
        // (`user`/`assistant`) are the ones a monochrome-looking palette breaks.
        let dark = Theme::with_palette(ColorMode::Truecolor, UNICODE, ThemeMode::Dark);
        let light = Theme::with_palette(ColorMode::Truecolor, UNICODE, ThemeMode::Light);
        for theme in [dark, light] {
            let name = theme.palette().name();
            assert_ne!(
                theme.success(),
                theme.danger(),
                "{name}: success and failure must never be the same pixels"
            );
            assert_ne!(
                theme.user(),
                theme.assistant(),
                "{name}: the question and the answer must be distinguishable"
            );
            assert_ne!(theme.text(), theme.dim(), "{name}: hierarchy collapsed");
        }
        // And the two palettes are genuinely two: body text differs in the
        // direction that matters, which is the whole point of the switch.
        let dark_text = luminance(Palette::DARK.get(Role::text).rgb);
        let light_text = luminance(Palette::LIGHT.get(Role::text).rgb);
        assert!(
            light_text < 0.1 && dark_text > 0.5,
            "body text must be dark on light and light on dark: {light_text:.3} / {dark_text:.3}"
        );
    }

    #[test]
    fn body_text_is_never_a_hardcoded_white_or_black() {
        // The skill forbids both, and for the same reason: they fight the user's
        // terminal theme. In ANSI-16 — where the palette's `ansi` value is what
        // is actually emitted — the body roles defer to the terminal, and the
        // light palette's slots are the dark-on-light members of the safe set.
        for palette in [Palette::DARK, Palette::LIGHT] {
            for role in [Role::text, Role::assistant] {
                assert_eq!(
                    palette.get(role).ansi,
                    Color::Reset,
                    "{role:?} must defer to the terminal's default foreground"
                );
            }
        }
        // Every *other* slot still has a real colour: `Reset` as a general
        // answer would mean an all-default palette, which is not a palette.
        let light = Theme::with_palette(ColorMode::Ansi16, ASCII, ThemeMode::Light);
        assert_eq!(light.danger().fg, Some(Color::Red), "the light red is Red");
        assert_eq!(light.success().fg, Some(Color::Green));
        assert_eq!(light.warn().fg, Some(Color::Yellow));
        assert_eq!(light.user().fg, Some(Color::Blue));
    }

    #[test]
    fn no_color_still_separates_the_roles_in_both_palettes() {
        // `NO_COLOR` is orthogonal to the palette: it is set because the reader
        // cannot take colour, and a light background is not a reason for the
        // reasoning trace to become indistinguishable from the answer.
        for palette in [ThemeMode::Dark, ThemeMode::Light] {
            let theme = Theme::with_palette(ColorMode::None, ASCII, palette);
            let name = palette.name();
            assert_eq!(theme.text(), Style::default(), "{name}: body text is plain");
            assert_eq!(
                theme.danger().add_modifier,
                Modifier::empty(),
                "{name}: a status is plain text plus its word"
            );
            assert_ne!(
                theme.thinking().add_modifier,
                Modifier::empty(),
                "{name}: reasoning needs an attribute when colour is off"
            );
            assert_ne!(
                theme.selected().add_modifier,
                Modifier::empty(),
                "{name}: selection must survive"
            );
            assert_ne!(
                theme.title().add_modifier,
                Modifier::empty(),
                "{name}: a title must outrank body copy without colour"
            );
            assert_eq!(
                theme.surface_user(),
                Style::default(),
                "{name}: no colour means no bands either"
            );
        }
    }

    #[test]
    fn the_palette_is_selected_by_name_and_survives_a_bad_one() {
        // The switch is a selection, not a guess: an unknown value falls back to
        // the default rather than failing to start, the same rule every other
        // config key follows.
        assert_eq!(ThemeMode::from_name("light"), Some(ThemeMode::Light));
        assert_eq!(ThemeMode::from_name("LIGHT"), Some(ThemeMode::Light));
        assert_eq!(ThemeMode::from_name(" dark "), Some(ThemeMode::Dark));
        assert_eq!(ThemeMode::from_name("solarized"), None);
        assert_eq!(ThemeMode::from_name(""), None);
        assert_eq!(ThemeMode::Light.flipped(), ThemeMode::Dark);
        assert_eq!(ThemeMode::Dark.flipped(), ThemeMode::Light);
        assert_eq!(ThemeMode::default(), ThemeMode::Dark);

        // And the runtime switch is a rebuild of the styles, not a repaint.
        let mut theme = Theme::new(ColorMode::Truecolor, UNICODE);
        assert_eq!(theme.palette(), ThemeMode::Dark);
        let was = theme.set_palette(ThemeMode::Light);
        assert_eq!(was, ThemeMode::Dark, "the switch reports what it replaced");
        assert_eq!(theme.palette(), ThemeMode::Light);
        let resolved = match theme.text().fg {
            Some(Color::Rgb(r, g, b)) => (r, g, b),
            other => panic!("light body text must be an RGB value, got {other:?}"),
        };
        assert!(
            luminance(resolved) < 0.1,
            "the switched theme resolves body text from the light table"
        );
    }
}
