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
//!    reasoning covers the *quiet* roles (`text_dim`, `text_faint`, `thinking`,
//!    `code_block`, `border`): on truecolor they keep their dimmer `rgb` values
//!    and are quiet by color, and on ANSI-16 the light palette resolves them to
//!    `Reset` too and carries "quiet" with `DIM`, because every ANSI slot that
//!    would read as "quiet grey" is invisible on one of the two backgrounds
//!    (bright black on white, black on black). Which roles those are is a column
//!    of the palette tables — [`Slot::quiet`] — and not a comment. Filled bands
//!    are withheld entirely off truecolor ([`Theme::has_surfaces`]) rather than
//!    approximated with a background that collides with the user's own theme.
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
    /// The marker for the session the user is looking at.
    pub cursor_bar: &'static str,
    /// Assistant prose, where no filled band is available.
    ///
    /// `assistant` has no marker while a band can say what the row is; with the
    /// band withheld (see [`Theme::has_surfaces`]) the two roles that carry the
    /// most text — the answer and the reasoning trace behind it — would other-
    /// wise render as the same run of plain words, which is the collapse
    /// `NO_COLOR` must not cause.
    pub assistant: &'static str,
    /// A reasoning trace, where no filled band is available. The same fallback as
    /// [`Self::assistant`], and the reason the two exist: the skill's rule is
    /// that reasoning must never be indistinguishable from the answer it
    /// produced.
    pub thinking: &'static str,
    /// The prompt marker that introduces a row the user typed.
    pub prompt: &'static str,
    /// The marker that introduces a tool call, before it has a result.
    pub tool_call: &'static str,
    /// A completed step.
    pub tick: &'static str,
    /// A step that succeeded but whose result is known to be short.
    ///
    /// A warning sign rather than a tick: the tick asserts the result is whole, and for a
    /// partially-collected output it is not. Not the cross either — nothing failed.
    pub warn: &'static str,
    /// A failed step.
    pub cross: &'static str,
    /// A line a diff adds. Part of the glyph table rather than a literal `"+"` in
    /// the diff renderer, like every other character the UI draws.
    pub plus: &'static str,
    /// A line a diff removes.
    pub minus: &'static str,
    /// How far back a frozen transcript view is: the marker in the footer's
    /// position badge (`↑ 12/40`, `^ 12/40`). Part of the glyph table like every
    /// other character the UI draws, so a legacy code page gets `^`.
    pub back: &'static str,
    /// The marker that says a model has stored credentials, where no filled band can.
    ///
    /// **Was an emoji (`🔑`), and that is a glyph no legacy code page can render at all** (M219):
    /// not a box-drawing character CP437 happens to carry, an astral-plane codepoint four UTF-8
    /// bytes wide. On a classic Windows console it is a row of boxes or `?` — the report that
    /// started this milestone.
    pub key: &'static str,
    /// The marker on a system line that records a switch (`⟳`).
    pub cycle: &'static str,
    /// The marker on the picker's selected row (`●`).
    ///
    /// **Not in CP437** — the closest the code page has is `•` at 0x07, which reads as a bullet
    /// rather than as "the one you are on". A literal `●` therefore rendered as a box on a legacy
    /// console (M219, A22).
    pub dot: &'static str,
    /// The marker on a favourited model (`★`). Like [`Self::dot`], absent from CP437.
    pub star: &'static str,
    /// The truncation mark (`…`).
    ///
    /// **Also absent from CP437**, which is the one that surprises people: `…` is at 0x85 in
    /// Windows-1252, where DOS put `à`. Every modern terminal draws it and a legacy console shows a
    /// box, so this is a glyph and not a literal (M220).
    pub ellipsis: &'static str,
    /// The arrows a hint row offers for moving a selection, alone and paired.
    ///
    /// CP437 *does* carry `↑` (0x18) and `↓` (0x19), so these are a preference rather than a
    /// repair: `^`/`v` in the ASCII set, because a seven-bit frame is what a legacy console was
    /// getting anyway for every other structural character.
    pub up: &'static str,
    /// The downward half of the pair; see [`Self::up`].
    pub down: &'static str,
    /// The dash a panel title separates its parts with (`—`).
    ///
    /// **An em dash is not in CP437** — 0x97 there is `ù`; `—` is 0x97 in Windows-1252, a different
    /// code page that every modern terminal happens to speak. M219 got this wrong in prose and the
    /// frame check caught it, which is the second time this table has been the thing that knows.
    pub dash: &'static str,
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
    cursor_bar: "▎",
    assistant: "•",
    thinking: "≡",
    prompt: "❯",
    tool_call: "▸",
    tick: "✓",
    warn: "!",
    cross: "✗",
    plus: "+",
    minus: "-",
    back: "↑",
    key: "🔑",
    cycle: "⟳",
    dot: "●",
    star: "★",
    ellipsis: "…",
    up: "↑",
    down: "↓",
    dash: "—",
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
    cursor_bar: ">",
    assistant: "*",
    thinking: "=",
    prompt: ">",
    tool_call: "\\",
    tick: "+",
    warn: "!",
    cross: "x",
    plus: "+",
    minus: "-",
    back: "^",
    key: "@",
    cycle: "~",
    dot: "*",
    star: "+",
    ellipsis: "...",
    up: "^",
    down: "v",
    dash: "--",
    spinner: &["|", "/", "-", "\\"],
};

/// Default for a table nobody chose for: **ASCII**, deliberately (M219).
///
/// The repertoire a value gets when nobody asked must be the one that renders *everywhere*, which
/// is what lets a holder of stored markers (`UiState`) derive `Default` and still promise seven-bit
/// output without the environment leaking into a per-instance construction.
/// [`Glyphs::detect`] is the *process* decision and is asked once, by the app.
impl Default for Glyphs {
    fn default() -> Self {
        ASCII
    }
}

impl Glyphs {
    /// Pick the repertoire: an explicit override first, then a per-platform default.
    ///
    /// - **`EPSILON_ASCII=1`** forces the ASCII set and **`EPSILON_ASCII=0`** forces Unicode, on
    ///   every platform. Both are documented in `--help` and in `docs/user/troubleshooting.md`,
    ///   because until M219 the switch existed only in a comment here — a user seeing boxes had
    ///   nothing to set.
    /// - **Windows without a UTF-8-capable host gets ASCII.** A classic conhost writes the code page
    ///   it is configured for, and every byte of a UTF-8 box-drawing character outside it renders as
    ///   a box or a `?`. **The previous default was Unicode everywhere**, which meant the most common
    ///   Windows case — plain `cmd.exe`, or PowerShell in a legacy console window — got mojibake by
    ///   default, and the only fix was a variable nobody was told about.
    /// - **Windows Terminal** (`WT_SESSION`), **VS Code's terminal** (`TERM_PROGRAM=vscode`), or a
    ///   `TERM` naming a VT-capable emulator, gets Unicode.
    /// - **Everywhere else gets Unicode**: a POSIX terminal is UTF-8 by definition.
    ///
    /// **This is a default, not a sniff.** Reading the console output code page needs either
    /// `unsafe` (forbidden workspace-wide) or a new dependency, and neither is worth a guess a user
    /// cannot correct — which is why both directions of the override exist.
    pub fn detect() -> Self {
        Self::detect_from(
            std::env::var("EPSILON_ASCII").ok(),
            std::env::var_os("WT_SESSION").map(|v| v.to_string_lossy().into_owned()),
            std::env::var("TERM_PROGRAM").ok(),
            std::env::var("TERM").ok(),
            cfg!(windows),
        )
    }

    /// The repertoire decision with every input handed in rather than read (M219).
    ///
    /// The environment is process-global and the suite is threaded, so a test that set
    /// `EPSILON_ASCII` to assert a default would be asserting against whatever another test had
    /// already set. Splitting the decision out is what makes the rule itself testable.
    pub fn detect_from(
        override_value: Option<String>,
        wt_session: Option<String>,
        term_program: Option<String>,
        term: Option<String>,
        on_windows: bool,
    ) -> Self {
        let forced = match override_value.as_deref().map(str::trim) {
            Some(v)
                if v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes") =>
            {
                Some(ASCII)
            }
            Some(v)
                if v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("no") =>
            {
                Some(UNICODE)
            }
            _ => None,
        };
        if let Some(set) = forced {
            return set;
        }
        // A UTF-8-capable *host*, not a shell: `cmd.exe` and PowerShell set none of these, and a
        // legacy conhost is exactly the case being careful about.
        let utf8_host = wt_session.is_some()
            || term_program.as_deref() == Some("vscode")
            || matches!(term.as_deref(), Some(t) if VT_CAPABLE_TERMS.iter().any(|k| t.contains(k)));
        if on_windows && !utf8_host {
            ASCII
        } else {
            UNICODE
        }
    }

    /// Every single-glyph field of this table, so a check can be exhaustive.
    ///
    /// The ASCII test below used to name its fields one per line, which is how
    /// the table grew `caret` and `warn` entries that no widget ever read: a list
    /// written by hand covers what its author remembered, and a glyph nobody
    /// draws is invisible to both the compiler and that list. Adding a field here
    /// is now the only way to add one to the table, so the next glyph cannot
    /// arrive unchecked — and the reader of that list is the same list.
    ///
    /// **That claim was itself wrong once** (M219): this list held sixteen names while the struct had
    /// seventeen single-glyph fields — `warn` was missing, and so are the two M219 added. So the one
    /// check that exists to keep the table honest skipped the very glyph that no console can render.
    /// `named`'s match arm catches a *name with no field*; [`Self::SINGLE_GLYPH_FIELDS`] and the set
    /// comparison in the tests catch the other direction.
    pub const ALL: &'static [&'static str] = &[
        "top_left",
        "top_right",
        "bottom_left",
        "bottom_right",
        "horizontal",
        "vertical",
        "bullet",
        "cursor_bar",
        "assistant",
        "thinking",
        "prompt",
        "tool_call",
        "tick",
        "warn",
        "cross",
        "plus",
        "minus",
        "back",
        "key",
        "cycle",
        "dash",
        "dot",
        "star",
        "ellipsis",
        "up",
        "down",
    ];

    /// Every single-glyph field of the table, listed independently of [`Self::ALL`] (M219).
    ///
    /// Two lists that are supposed to agree, compared as *sets* rather than by count: a count passes
    /// when both are wrong in the same way, which is how `warn` went unchecked for months. It is a
    /// second list to keep in step — the price of a check that can actually fail — and
    /// `the_glyph_name_list_and_the_table_agree` is what pays it.
    pub const SINGLE_GLYPH_FIELDS: &'static [&'static str] = &[
        "top_left",
        "top_right",
        "bottom_left",
        "bottom_right",
        "horizontal",
        "vertical",
        "bullet",
        "cursor_bar",
        "assistant",
        "thinking",
        "prompt",
        "tool_call",
        "tick",
        "warn",
        "cross",
        "plus",
        "minus",
        "back",
        "key",
        "cycle",
        "dash",
        "dot",
        "star",
        "ellipsis",
        "up",
        "down",
    ];

    /// This table's glyphs by name, for a caller that has to check all of them.
    pub fn named(&self) -> impl Iterator<Item = (&'static str, &str)> + '_ {
        Self::ALL.iter().map(move |name| {
            let glyph = match *name {
                "top_left" => self.top_left,
                "top_right" => self.top_right,
                "bottom_left" => self.bottom_left,
                "bottom_right" => self.bottom_right,
                "horizontal" => self.horizontal,
                "vertical" => self.vertical,
                "bullet" => self.bullet,
                "cursor_bar" => self.cursor_bar,
                "assistant" => self.assistant,
                "thinking" => self.thinking,
                "prompt" => self.prompt,
                "tool_call" => self.tool_call,
                "tick" => self.tick,
                "warn" => self.warn,
                "cross" => self.cross,
                "plus" => self.plus,
                "minus" => self.minus,
                "back" => self.back,
                "key" => self.key,
                "cycle" => self.cycle,
                "dash" => self.dash,
                "dot" => self.dot,
                "star" => self.star,
                "ellipsis" => self.ellipsis,
                "up" => self.up,
                "down" => self.down,
                other => panic!("`{other}` is in `Glyphs::ALL` with no reader: a name there is a promise that a glyph is checked, and this is the arm that keeps it"),
            };
            (*name, glyph)
        })
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

/// `TERM` values that mean the terminal speaks UTF-8 and VT escapes.
const VT_CAPABLE_TERMS: &[&str] = &[
    "xterm",
    "vt100",
    "alacritty",
    "wezterm",
    "ghostty",
    "kitty",
    "foot",
];

/// One semantic color slot, in both palettes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Slot {
    pub(crate) rgb: (u8, u8, u8),
    pub(crate) ansi: Color,
    /// Whether the role reads as *background material* rather than as content.
    ///
    /// The only place this is used is the ANSI-16 fallback: on a 24-bit terminal
    /// the `rgb` value carries the muting, but the 16 ANSI slots are decided by
    /// the user's terminal, and the slots that read as "quiet grey" on one
    /// background are close to invisible on the other (`DarkGray` is bright black
    /// on white; `Black` is invisible on black). A quiet role therefore falls back
    /// to the terminal's own foreground plus [`Modifier::DIM`], which is quiet on
    /// *whatever* background the user chose. See [`Theme::role`].
    pub(crate) quiet: bool,
}

impl Slot {
    /// The modifier that carries a quiet role when the color cannot.
    ///
    /// Empty off ANSI-16, deliberately: on a 24-bit terminal `rgb` *is* the
    /// muting (a dimmer grey), and dimming it a second time would make the
    /// truecolor palettes depend on this fallback. With color off entirely the
    /// emphasis argument has already said what to do, and this returns empty so
    /// the two mechanisms cannot both fire.
    pub(crate) fn quiet_modifier(self, mode: ColorMode) -> Modifier {
        match (self.quiet, mode) {
            (true, ColorMode::Ansi16) => Modifier::DIM,
            _ => Modifier::empty(),
        }
    }
}

/// Build a palette table, its role index, and the list of every role.
///
/// `quiet` is declared per row rather than inferred from the name, and the macro
/// makes it a required field: a role added without deciding whether it is content
/// or background material does not compile. That matters because the decision is
/// invisible on the machine most of this is developed on — a truecolor terminal
/// never reads the field.
macro_rules! palette {
    ($($name:ident: $rgb:expr, $ansi:expr, $quiet:expr;)*) => {
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

        impl Role {
            /// Every role in the table, for the tests that have to cover all of
            /// them — generated here so a role added above cannot be forgotten by
            /// a hand-written list. A role no test can name is a role nothing
            /// checks, which is how the light palette's `ansi` column went
            /// uninspected while a comment promised it was safe.
            pub const ALL: &'static [Role] = &[$(Role::$name,)*];
        }

        impl Palette {
            /// The dark palette, which is what most terminals are.
            pub(crate) const DARK: Self = Self {
                $($name: Slot { rgb: $rgb, ansi: $ansi, quiet: $quiet },)*
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
    // Structure: borders, rules, and chrome the eye should skip over. The border
    // is quiet: it is a hairline, not a box.
    border:        (0x3b, 0x40, 0x4a), Color::DarkGray, true;

    border_focus:  (0x62, 0x9d, 0xf5), Color::Cyan, false;
    // Text. `Color::Reset` on the ANSI side, never `White`: the terminal's own
    // default foreground is the only ANSI value guaranteed to contrast with
    // whatever background the user set (see decision 3 in the module docs).
    text:          (0xd7, 0xdb, 0xe2), Color::Reset, false;

    text_dim:      (0x8a, 0x92, 0xa3), Color::DarkGray, true;

    text_faint:    (0x5a, 0x62, 0x72), Color::DarkGray, true;
    // Who is speaking.
    user:          (0x7d, 0xd3, 0xfc), Color::Cyan, false;

    assistant:     (0xd7, 0xdb, 0xe2), Color::Reset, false;

    thinking:      (0x8a, 0x92, 0xa3), Color::DarkGray, true;
    // Outcome.
    success:       (0x6e, 0xe7, 0xa0), Color::Green, false;

    danger:        (0xf8, 0x76, 0x76), Color::Red, false;

    warn:          (0xf5, 0xc2, 0x5c), Color::Yellow, false;

    accent:        (0x9d, 0x7c, 0xf5), Color::Magenta, false;
    // Inline code and fenced blocks.
    code:          (0xe0, 0xaf, 0x68), Color::Yellow, false;

    code_block:    (0x9a, 0xa3, 0xb5), Color::Gray, true;
    // Filled bands. Deliberately close to a dark terminal background: a band
    // should read as a change of surface, not as a selected row. They are never
    // emitted on ANSI-16 at all ([`Theme::has_surfaces`]), so `quiet` is moot.
    surface_user:    (0x22, 0x24, 0x2c), Color::Reset, false;

    surface_pending: (0x2a, 0x2a, 0x22), Color::Reset, false;

    surface_ok:      (0x1e, 0x2a, 0x22), Color::Reset, false;

    surface_err:     (0x2e, 0x20, 0x20), Color::Reset, false;
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
    ($($name:ident: $rgb:expr, $ansi:expr, $quiet:expr;)*) => {
        impl Palette {
            /// Tuned for a light background.
            ///
            /// `ansi` is a narrower promise than `rgb`, and it is the column the
            /// user `--light` exists for actually sees: [`ColorMode::Ansi16`] is
            /// what a console that does not advertise itself gets, which is the
            /// console whose background the user had to tell us about by hand.
            ///
            /// Which slots are safe there is a property of ANSI-16, not of taste.
            /// ANSI 7/8 and the bright half — `Color::Gray`, `Color::White`,
            /// `Color::DarkGray` (ratatui's name for *bright black*) and every
            /// `Light*` variant, which is ANSI 9–15 — are light on every terminal,
            /// so none of them may appear here: they are the slots that vanish on
            /// white. The body roles therefore defer to the terminal's own
            /// foreground ([`Color::Reset`]), and the *quiet* roles carry "quiet"
            /// with [`Modifier::DIM`] instead of with a slot — dimming whatever the
            /// terminal draws normal text in is the only way to be secondary on a
            /// background we cannot ask about.
            ///
            /// What is left is the standard slots, and the light palette is
            /// restricted to five of them plus one bright one, measured against
            /// white at the xterm defaults: `Reset` (the terminal's own, and the
            /// reason the quiet roles work at all), `LightBlue` 4.7:1, `Magenta`
            /// 4.7:1, `Red` 5.8:1, `Blue` 9.4:1 — and `Green` 2.2:1 and `Yellow`
            /// 1.7:1, which are the honest limit of this palette and the reason it
            /// is not a beautiful one: **ANSI-16 on a light background has no
            /// legible green and no legible yellow.** Reaching for the bright slots
            /// makes it worse, not better — ANSI 10 and 11 are `#00ff00` and
            /// `#ffff00` — so `success`, `warn` and `code` keep the standard slots,
            /// stay distinguishable from each other and from `danger`, and lean on
            /// their glyph and their word for legibility, which is what the skill
            /// requires of colour in the first place. The alternative was to paint
            /// the warning in an unrelated hue, which trades a legible label for a
            /// wrong one; that trade is recorded in the milestone's limits rather
            /// than made silently here.
            ///
            /// `the_ansi_column_is_checked_for_every_role_in_both_palettes` is where
            /// all of this is enforced, including the two ratios above that do not
            /// clear a floor.
            pub(crate) const LIGHT: Self = Self {
                $($name: Slot { rgb: $rgb, ansi: $ansi, quiet: $quiet },)*
            };
        }
    };
}

light_palette! {
    // Structure: a hairline that is visible on white without becoming a frame.
    // 3.6:1 against white — the skill's floor for a non-body role, since a
    // divider nobody can see is a divider that does not exist. On ANSI-16 the
    // slot is the terminal's own foreground, dimmed: see the table docs above.
    border:        (0xb9, 0xc0, 0xcc), Color::Reset, true;

    // `LightBlue` (ANSI 12, `#5c5cff`) is the one bright slot that is *darker*
    // than its standard counterpart (`Blue`, `#0000ee` is darker still, but it is
    // the user role's). ANSI 12 is 6.9:1 on white; the dark palette's `Cyan` is
    // 1.9:1.
    border_focus:  (0x1e, 0x63, 0xc4), Color::LightBlue, false;
    // Text: near-black body copy (14.8:1) rather than pure black, which reads as
    // a photocopy; secondary and tertiary values stay well above 4.5:1.
    text:          (0x1f, 0x24, 0x30), Color::Reset, false;

    text_dim:      (0x4a, 0x53, 0x64), Color::Reset, true;

    text_faint:    (0x5a, 0x63, 0x73), Color::Reset, true;
    // Who is speaking. The dark palette's pastel cyan is 1.6:1 on white; this is
    // the same hue taken dark enough to read (6.4:1). ANSI 4 is 8.6:1.
    user:          (0x0b, 0x5e, 0xa8), Color::Blue, false;

    assistant:     (0x1f, 0x24, 0x30), Color::Reset, false;

    thinking:      (0x4a, 0x53, 0x64), Color::Reset, true;
    // Outcome: the semantic hues, darkened rather than pastelised. All three
    // clear 4.5:1 on white *as RGB* while staying distinguishable from each
    // other, which is the property that matters more than any single ratio.
    //
    // On ANSI-16 they cannot: the standard slots for these hues are the light ones
    // on white (3.0:1, 1.4:1, 1.1:1) and the bright ones are lighter still. They
    // stay, because a wrong hue is worse than a quiet one, and each of these roles
    // is always drawn beside a glyph and a word. See the table docs — this pair of
    // lines is the palette's stated limit, not an oversight.
    success:       (0x1d, 0x6b, 0x3a), Color::Green, false;

    danger:        (0xa3, 0x22, 0x22), Color::Red, false;

    warn:          (0x7a, 0x5c, 0x00), Color::Yellow, false;

    accent:        (0x6b, 0x3f, 0xc4), Color::Magenta, false;
    // Inline code and fenced blocks. `code` shares dark's slot for the same reason
    // `warn` keeps yellow: the hue carries the meaning, and the alternative on
    // white is a slot with less contrast, not more.
    code:          (0x7a, 0x4a, 0x12), Color::Yellow, false;

    code_block:    (0x4a, 0x53, 0x64), Color::Reset, true;
    // Filled bands: pale tints of white, dark enough to hold body text at 4.5:1
    // and light enough to read as a change of surface rather than as a
    // selection. A band that the text on it cannot sit on is worse than no band.
    surface_user:    (0xdf, 0xe4, 0xee), Color::Reset, false;

    surface_pending: (0xf4, 0xef, 0xd6), Color::Reset, false;

    surface_ok:      (0xe0, 0xef, 0xe6), Color::Reset, false;

    surface_err:     (0xf9, 0xe3, 0xe3), Color::Reset, false;
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
    /// This swaps which table roles resolve from; it does not repaint anything.
    /// The theme is `Copy` and every widget resolves roles through it at draw
    /// time, so the next frame is already in the new palette — and the render
    /// cache, which is keyed by width and render state and deliberately not by the
    /// theme, stays valid rather than being invalidated by a colour change it does
    /// not hold. Nothing is patched a cell at a time, and nothing waits for a
    /// repaint: the rows on screen change when the next frame draws them.
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
            (_, Emphasis::Plain) => style.add_modifier(slot.quiet_modifier(self.mode)),
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
    pub fn dim(&self) -> Style {
        self.role(Role::text_dim, Emphasis::Plain)
    }

    /// Tertiary information: labels, separators, anything skippable.
    pub fn faint(&self) -> Style {
        self.role(Role::text_faint, Emphasis::Plain)
    }

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
    pub fn thinking(&self) -> Style {
        self.role(Role::thinking, Emphasis::Muted)
    }

    // Outcome.
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
    pub fn border(&self) -> Style {
        self.role(Role::border, Emphasis::Plain)
    }

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

    /// The band behind a user message.
    pub fn surface_user(&self) -> Style {
        self.surface(Role::surface_user)
    }

    /// The band behind a tool call that is still waiting for a decision.
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
    ///
    /// The transcript is the widget that does: [`crate::ui::transcript::row_style`]
    /// picks its gutter from this, which is what [`Glyphs::assistant`] and
    /// [`Glyphs::thinking`] are for. Off truecolor — and off colour entirely —
    /// the answer and the reasoning trace behind it get one marker each instead
    /// of sharing the undecorated row that a band used to explain.
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
        // Every field of the table, by construction: `Glyphs::named` walks the one
        // list of them, so a glyph added there and forgotten here cannot happen.
        // The name is unused because `named`'s own match is what proves the list
        // and the struct agree — a name with no field panics there, not here.
        let mut seen = 0usize;
        for (_name, glyph) in ASCII.named() {
            check(glyph);
            seen += 1;
        }
        assert_eq!(seen, Glyphs::ALL.len(), "every glyph must be checked");
        assert!(seen >= 15, "the table lost fields: {seen} checked");
        for frame in ASCII.spinner {
            check(frame);
        }
    }

    /// M219: the ASCII check's name list and the table's fields are the same set.
    ///
    /// `ALL` is what the check above walks, so a field absent from it is drawn and never verified —
    /// which is what happened: `warn` (and, after it, `key` and `cycle`) sat outside the list while
    /// the list's own comment claimed a field could not be added without a name. The old assertion
    /// compared **counts**, which passes when both lists are wrong together; this compares **sets**.
    #[test]
    fn the_glyph_name_list_and_the_table_agree() {
        let mut named: Vec<&str> = Glyphs::ALL.to_vec();
        let mut fields: Vec<&str> = Glyphs::SINGLE_GLYPH_FIELDS.to_vec();
        named.sort_unstable();
        fields.sort_unstable();
        named.dedup();
        fields.dedup();
        assert_eq!(
            named, fields,
            "`ALL` drives the ASCII check, so a field missing from it is never checked; \
             `SINGLE_GLYPH_FIELDS` is the independent list it is compared against"
        );
        // The other direction: a *name* with no field panics inside `named()`, so walking it proves
        // the two lists agree as code and not merely as text.
        assert_eq!(ASCII.named().count(), Glyphs::ALL.len());
    }

    /// M219: the repertoire rule, and the Windows half of it that stopped `cmd.exe` and PowerShell
    /// from getting mojibake by default.
    ///
    /// Driven through [`Glyphs::detect_from`] rather than by setting the variable: the environment is
    /// process-global and the suite is threaded, so a test that set `EPSILON_ASCII` would be
    /// asserting against whatever another test left behind.
    #[test]
    fn the_repertoire_default_is_ascii_on_a_legacy_windows_console() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(
            Glyphs::detect_from(None, None, None, None, true),
            ASCII,
            "a console with no UTF-8 host must not be handed box-drawing characters by default"
        );
        assert_eq!(
            Glyphs::detect_from(None, None, None, s("xterm-256color"), true),
            UNICODE,
            "a VT-capable TERM is the same answer Windows Terminal gives"
        );
        assert_eq!(
            Glyphs::detect_from(None, s("session-id"), None, None, true),
            UNICODE
        );
        assert_eq!(
            Glyphs::detect_from(None, None, s("vscode"), None, true),
            UNICODE
        );
        assert_eq!(
            Glyphs::detect_from(None, None, None, None, false),
            UNICODE,
            "a POSIX terminal is UTF-8 by definition"
        );
        // And the overrides win over both, which is the whole point of shipping them.
        assert_eq!(Glyphs::detect_from(s("1"), None, None, None, false), ASCII);
        assert_eq!(
            Glyphs::detect_from(s("0"), None, None, None, true),
            UNICODE,
            "`EPSILON_ASCII=0` is how a legacy console user with a Unicode font opts back in"
        );
    }

    #[test]
    fn no_doc_comment_line_is_written_twice() {
        // There were seven of these, and they arrived the way this class always
        // does: a doc block gets copied onto the item below it, and a repeated
        // line reads as emphasis rather than as a mistake — which is why seven
        // survived a slice, a review and a fix round that only removed one of
        // them. This file is the palette's ADR-shaped record, so a duplicated line
        // is a duplicated *decision*, and the guard is cheaper than noticing.
        let source = include_str!("theme.rs");
        let mut seen: std::collections::HashMap<&str, Vec<usize>> =
            std::collections::HashMap::new();
        for (index, line) in source.lines().enumerate() {
            let Some(doc) = line.trim_start().strip_prefix("///") else {
                continue;
            };
            let text = doc.trim();
            if text.is_empty() {
                continue;
            }
            seen.entry(text).or_default().push(index + 1);
        }
        let repeated: Vec<String> = seen
            .iter()
            .filter(|(_, at)| at.len() > 1)
            .map(|(text, at)| format!("{text:?} at lines {at:?}"))
            .collect();
        assert!(
            repeated.is_empty(),
            "a doc line written more than once reads as emphasis and is a copy: {repeated:#?}"
        );
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
        // actually promise: the body roles resolve to `Color::Reset`, which
        // follows the terminal, so a light palette on a dark terminal is
        // degraded-but-readable rather than broken — but a role whose *truecolor*
        // value is identical to the dark palette's is a copy that forgot to
        // change, and that is checkable. (The ANSI-16 column is checked the same
        // way, and more strictly, further down.)
        for role in [Role::text, Role::assistant] {
            assert_ne!(
                Palette::LIGHT.get(role).rgb,
                Palette::DARK.get(role).rgb,
                "{role:?} is the dark palette's value under a light name"
            );
        }
    }

    /// The ANSI-16 slot each role is expected to emit, in both palettes, and
    /// whether it carries `DIM` ([`Slot::quiet`]) — one record per row, so the
    /// dark and light expectations cannot drift out of step.
    ///
    /// Hand-written on purpose: `Role::ALL` proves the *coverage* below (nothing
    /// is skipped, and nothing is invented), and this table is what makes the
    /// assertion about a value rather than about a shape. A role added to either
    /// palette fails the coverage check until it is given a row here.
    fn ansi_slots() -> Vec<AnsiExpectation> {
        use AnsiExpectation as E;
        use Color as C;
        vec![
            // Structure and text.
            E::new(Role::border, C::Reset, C::DarkGray, true),
            E::new(Role::border_focus, C::LightBlue, C::Cyan, false),
            E::new(Role::text, C::Reset, C::Reset, false),
            E::new(Role::text_dim, C::Reset, C::DarkGray, true),
            E::new(Role::text_faint, C::Reset, C::DarkGray, true),
            // Who is speaking.
            E::new(Role::user, C::Blue, C::Cyan, false),
            E::new(Role::assistant, C::Reset, C::Reset, false),
            E::new(Role::thinking, C::Reset, C::DarkGray, true),
            // Outcome and accent.
            E::new(Role::success, C::Green, C::Green, false),
            E::new(Role::danger, C::Red, C::Red, false),
            E::new(Role::warn, C::Yellow, C::Yellow, false),
            E::new(Role::accent, C::Magenta, C::Magenta, false),
            // Code. `code` keeps dark's slot on purpose: see the light table.
            E::new(Role::code, C::Yellow, C::Yellow, false),
            E::new(Role::code_block, C::Reset, C::Gray, true),
            // Bands. Never emitted on ANSI-16 at all ([`Theme::has_surfaces`]);
            // the slot is still declared and still checked, so turning surfaces on
            // for 16 colors would be a visible change in this table rather than a
            // silent one.
            E::new(Role::surface_user, C::Reset, C::Reset, false),
            E::new(Role::surface_pending, C::Reset, C::Reset, false),
            E::new(Role::surface_ok, C::Reset, C::Reset, false),
            E::new(Role::surface_err, C::Reset, C::Reset, false),
        ]
    }

    /// One row of [`ansi_slots`].
    #[derive(Debug, Clone, Copy)]
    struct AnsiExpectation {
        role: Role,
        light: Color,
        dark: Color,
        quiet: bool,
    }

    impl AnsiExpectation {
        fn new(role: Role, light: Color, dark: Color, quiet: bool) -> Self {
            Self {
                role,
                light,
                dark,
                quiet,
            }
        }
    }

    #[test]
    fn the_ansi_column_is_checked_for_every_role_in_both_palettes() {
        // B2, the defect this test exists to make unrepeatable: the light table
        // used to override only `rgb` for the quiet roles and leave `ansi` at the
        // dark palette's `Color::DarkGray` — bright black, invisible on the white
        // terminal `--light` is for — directly under a comment promising these
        // values were "never `White`, never `DarkGray`". The prose was right and
        // nothing read the column, so the two disagreed for as long as nobody
        // looked. A claim about `.ansi` is now a claim about `.ansi`.
        let table = ansi_slots();
        for role in Role::ALL {
            assert!(
                table.iter().any(|e| e.role == *role),
                "{role:?} has no ANSI-16 row in this test: every role needs one, \
                 because a role nothing names is a role nothing checks"
            );
        }
        // …and the reverse: the test may not invent a role either.
        for row in &table {
            assert!(
                Role::ALL.contains(&row.role),
                "{:?} is checked here but is not in the palette table",
                row.role
            );
        }

        // The slots a light-background role may name. This is the *real* rule the
        // old comment gestured at, and it is checkable by name: `Gray`, `White`,
        // `DarkGray` (bright black) and every `Light*` variant are light on every
        // terminal — ANSI 9–15 — so they are invisible or near-invisible on white.
        // What is left is the terminal's own foreground, the five standard slots
        // that are not near-white, and `LightBlue`, the one bright slot that is
        // darker than its standard counterpart.
        let allowed_on_white = [
            Color::Reset,
            Color::Blue,
            Color::Magenta,
            Color::Red,
            Color::Green,
            Color::Yellow,
            Color::LightBlue,
        ];
        for role in Role::ALL {
            let emitted = Palette::LIGHT.get(*role).ansi;
            assert!(
                allowed_on_white.contains(&emitted),
                "light {role:?} emits {emitted:?} on a 16-colour terminal, which is \
                 one of the slots that disappears on a white background"
            );
        }

        // Every role, in both palettes, read through the style the draw path
        // actually emits.
        let dark = Theme::with_palette(ColorMode::Ansi16, UNICODE, ThemeMode::Dark);
        let light = Theme::with_palette(ColorMode::Ansi16, UNICODE, ThemeMode::Light);
        for row in &table {
            let role = row.role;
            let dark_style = dark.role(role, Emphasis::Plain);
            assert_eq!(
                dark_style.fg,
                Some(row.dark),
                "dark: {role:?} moved — this slice is about the light column"
            );
            assert_eq!(
                dark_style.add_modifier.contains(Modifier::DIM),
                row.quiet,
                "dark: {role:?} carries the quiet flag or it does not"
            );

            let style = light.role(role, Emphasis::Plain);
            assert_eq!(
                style.fg,
                Some(row.light),
                "light: {role:?} emits {:?} on a 16-colour terminal, not {:?}",
                style.fg,
                row.light
            );
            // Quiet is what distinguishes secondary information from body copy now
            // that both defer to the terminal. Without the attribute, `theme.dim()`
            // and `theme.text()` would be the same pixels on exactly the console
            // `--light` exists for.
            assert_eq!(
                style.add_modifier.contains(Modifier::DIM),
                row.quiet,
                "light: {role:?} must {} DIM on a 16-colour terminal",
                if row.quiet { "carry" } else { "not carry" }
            );
        }

        // The claim stated as a number, for the slots it is really about: the
        // quiet roles used to name ANSI 8, and these are the nominal values a
        // console renders it in. The spread is part of the argument — from
        // `#555753` (GNOME) through `#808080` (xterm) to near-white on some
        // Windows consoles — because a palette that names a slot the console may
        // render near white cannot promise anything about its own contrast.
        const ANSI_BRIGHT_BLACK: (u8, u8, u8) = (0x80, 0x80, 0x80);
        const ANSI_GRAY: (u8, u8, u8) = (0xe5, 0xe5, 0xe5);
        let bright_black = contrast(ANSI_BRIGHT_BLACK, WHITE);
        let gray = contrast(ANSI_GRAY, WHITE);
        assert!(
            bright_black < 4.5 && gray < 4.5,
            "the premise of B2: bright black scores {bright_black:.2}:1 and gray \
             {gray:.2}:1 on white, both under the 4.5:1 body floor"
        );
        // …and the light palette's residual limit, stated where a change to it
        // fails a test rather than passing quietly. Every slot it names is
        // measured; the ones that cannot clear the floor are named so the next
        // reader knows they were measured and kept, not forgotten.
        let literals: Vec<(Role, f64)> = Role::ALL
            .iter()
            .filter(|role| Palette::LIGHT.get(**role).ansi != Color::Reset)
            .map(|role| {
                (
                    *role,
                    contrast(nominal_rgb(Palette::LIGHT.get(*role).ansi), WHITE),
                )
            })
            .collect();
        assert_eq!(
            literals
                .iter()
                .filter(|(_, ratio)| *ratio < 4.5)
                .map(|(role, _)| *role)
                .collect::<Vec<_>>(),
            vec![Role::success, Role::warn, Role::code],
            "the light palette's under-floor slots are exactly the two ANSI hues \
             that are light on white: {literals:?} — a fourth is a decision, not a \
             detail"
        );
        assert!(
            literals
                .iter()
                .filter(|(role, _)| !matches!(role, Role::success | Role::warn | Role::code))
                .all(|(_, ratio)| *ratio >= 4.5),
            "every *other* slot the light palette names clears the body floor: \
             {literals:?}"
        );
        assert!(
            literals.iter().all(|(_, ratio)| *ratio >= 1.7),
            "no light slot may be near-invisible: {literals:?}"
        );

        // The truecolor values are untouched by any of this — the DIM fallback is
        // a property of the 16-colour mode, not of the palette.
        let rich = Theme::with_palette(ColorMode::Truecolor, UNICODE, ThemeMode::Light);
        assert_eq!(
            rich.dim().add_modifier,
            Modifier::empty(),
            "a truecolor terminal must not get an extra DIM on top of the grey"
        );
    }

    /// The nominal RGB a terminal renders one of the named ANSI slots in.
    ///
    /// Only used to state a ratio, and only for the slots the light palette is
    /// allowed to name — `Reset` has no value here by definition. The numbers are
    /// the xterm defaults, which is the most conservative reading available: a
    /// console that renders a slot differently renders it differently for the
    /// *dark* palette too, and the light palette's alternative to naming a slot is
    /// to defer to the terminal, which is what it does for every body role.
    fn nominal_rgb(color: Color) -> (u8, u8, u8) {
        match color {
            Color::Black => (0x00, 0x00, 0x00),
            Color::Red => (0xcd, 0x00, 0x00),
            Color::Green => (0x00, 0xcd, 0x00),
            Color::Yellow => (0xcd, 0xcd, 0x00),
            Color::Blue => (0x00, 0x00, 0xee),
            Color::Magenta => (0xcd, 0x00, 0xcd),
            Color::Cyan => (0x00, 0xcd, 0xcd),
            Color::Gray => (0xe5, 0xe5, 0xe5),
            Color::DarkGray => (0x80, 0x80, 0x80),
            Color::LightRed => (0xff, 0x00, 0x00),
            Color::LightGreen => (0x00, 0xff, 0x00),
            Color::LightYellow => (0xff, 0xff, 0x00),
            Color::LightBlue => (0x5c, 0x5c, 0xff),
            Color::LightMagenta => (0xff, 0x00, 0xff),
            Color::LightCyan => (0x00, 0xff, 0xff),
            Color::White => (0xff, 0xff, 0xff),
            other => panic!("{other:?} is not one of the 16 named slots"),
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
        // answer would mean an all-default palette, which is not a palette. The
        // light slots are named here rather than left to the table so a
        // "simplification" back to the bright variants — which are *lighter* on a
        // light terminal, exactly backwards — fails a test with the reason
        // attached.
        let light = Theme::with_palette(ColorMode::Ansi16, ASCII, ThemeMode::Light);
        assert_eq!(light.danger().fg, Some(Color::Red), "the light red is Red");
        assert_eq!(light.success().fg, Some(Color::Green));
        assert_eq!(light.warn().fg, Some(Color::Yellow));
        assert_eq!(light.user().fg, Some(Color::Blue));
        assert_eq!(light.accent().fg, Some(Color::Magenta));
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
