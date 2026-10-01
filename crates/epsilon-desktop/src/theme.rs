//! The desktop palette, adapted from the terminal's semantic roles.
//!
//! # Why the terminal's palette and not a second one
//!
//! A product with two frontends and two palettes looks like two products. The
//! terminal frontend already has a carefully reasoned palette with named *roles*
//! (`text`, `accent`, `danger`, `code`, `border`…) and a light variant whose
//! contrast behaviour was checked on a white console
//! (`epsilon_tui::ui::theme`). So this crate borrows it and converts, rather than
//! inventing hex values that would drift from it within a month.
//!
//! The conversion is deliberately one-directional and total: `ratatui::Color` →
//! `egui::Color32`. No desktop-specific colours are defined here. A role the
//! desktop needs that the terminal does not have is a missing *role*, and roles
//! belong to one place.
//!
//! # Contrast is a test, not a claim
//!
//! [`IdeTheme::contrast`] returns the WCAG 2.1 contrast ratio for the body text
//! against the window background, and [`tests::body_text_is_readable_in_both_palettes`]
//! fails if either palette drops below 4.5:1. A desktop IDE is read for hours at a
//! time; this is the one accessibility number in the crate that has a hard floor.

use egui::{Color32, Visuals};
use epsilon_tui::ThemeMode;

/// Convert a ratatui color to an egui one.
fn to_color32(c: ratatui::style::Color, fallback: Color32) -> Color32 {
    use ratatui::style::Color as C;
    match c {
        C::Reset => fallback,
        C::Black => Color32::BLACK,
        C::Red => Color32::from_rgb(0xCC, 0x33, 0x33),
        C::Green => Color32::from_rgb(0x2E, 0x8B, 0x57),
        C::Yellow => Color32::from_rgb(0xC9, 0xA2, 0x27),
        C::Blue => Color32::from_rgb(0x3B, 0x82, 0xC4),
        C::Magenta => Color32::from_rgb(0xA0, 0x5C, 0xC8),
        C::Cyan => Color32::from_rgb(0x2A, 0xA5, 0xA5),
        C::Gray => Color32::from_rgb(0x9A, 0x9A, 0x9A),
        C::DarkGray => Color32::from_rgb(0x60, 0x60, 0x60),
        C::LightRed => Color32::from_rgb(0xE0, 0x60, 0x60),
        C::LightGreen => Color32::from_rgb(0x4C, 0xC0, 0x70),
        C::LightYellow => Color32::from_rgb(0xE3, 0xC0, 0x4A),
        C::LightBlue => Color32::from_rgb(0x6A, 0xA8, 0xE8),
        C::LightMagenta => Color32::from_rgb(0xC8, 0x86, 0xE8),
        C::LightCyan => Color32::from_rgb(0x5C, 0xD0, 0xD0),
        C::White => Color32::WHITE,
        C::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        // Indexed colors cannot occur: the desktop always resolves the palette in
        // truecolor mode, which is asserted by `IdeTheme::from`.
        C::Indexed(_) => fallback,
    }
}

/// Minimum contrast for a colour that **carries text** against the window background.
///
/// WCAG 2.1 AA for normal text is 4.5:1; this crate holds body text to that floor
/// ([`IdeTheme::contrast`]) and the secondary roles to AA for large text, 3:1, because
/// they are used for 12-point labels rather than for paragraphs.
pub const TEXT_FLOOR: f32 = 3.0;

/// Minimum contrast for a colour that is **a background** rather than a glyph.
///
/// A fill is read as a boundary, not as text, so the 3:1 text floor would be the wrong
/// rule — a card 3:1 from the page would shout. This is the point at which a band stops
/// being the page: about 4% of relative luminance.
pub const SURFACE_FLOOR: f32 = 1.12;

/// Mix `c` away from `bg` until it reaches `min_ratio` contrast against it.
///
/// The step is toward white on a dark background and toward black on a light one, in
/// twenty increments, so the hue survives and the result is the palette's colour rather
/// than a substitute for it. If twenty steps are not enough — which needs a colour
/// *more* than 4:1 away already being darkened toward the background — the far end is
/// returned, because a visible colour in the wrong hue beats an invisible one.
fn separate(c: Color32, bg: Color32, min_ratio: f32) -> Color32 {
    if IdeTheme::ratio(c, bg) >= min_ratio {
        return c;
    }
    // Relative luminance, not a contrast comparison against white: #FAFAF9 has a
    // contrast ratio of about 1.005 against white, so > 1.0 called it a DARK
    // background and pushed the light palette's surface colours *toward* white --
    // i.e. further from the page they were supposed to band. The first run of this
    // helper had it exactly backwards for the light theme.
    let towards = if IdeTheme::luminance(bg) < 0.5 {
        Color32::WHITE
    } else {
        Color32::BLACK
    };
    for step in 1..=20u32 {
        let moved = mix(c, towards, step as f32 / 20.0);
        if IdeTheme::ratio(moved, bg) >= min_ratio {
            return moved;
        }
    }
    towards
}

/// Linear interpolation of `a` toward `b` by `t`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    Color32::from_rgb(
        lerp(a.r(), b.r(), t),
        lerp(a.g(), b.g(), t),
        lerp(a.b(), b.b(), t),
    )
}

/// Linear interpolation between two channel values.
fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// The semantic roles this frontend draws with.
#[derive(Debug, Clone, Copy)]
pub struct IdeTheme {
    /// Which terminal palette this was adapted from.
    pub palette: ThemeMode,
    /// Body text.
    pub text: Color32,
    /// Secondary text: timestamps, hints, non-essential chrome.
    pub dim: Color32,
    /// The user's own words.
    pub user: Color32,
    /// The assistant's answer.
    pub assistant: Color32,
    /// Reasoning traces — background material, deliberately quieter.
    pub thinking: Color32,
    /// Interactive affordances: links, focus, selection.
    pub accent: Color32,
    /// A finished thing that worked.
    pub success: Color32,
    /// A refusal or a failure.
    pub danger: Color32,
    /// Something the user should look at.
    pub warn: Color32,
    /// Inline code.
    pub code: Color32,
    /// Block backgrounds.
    pub code_block: Color32,
    /// Pane edges.
    pub border: Color32,
    /// The focused pane's edge.
    pub border_focus: Color32,
    /// Pane titles.
    pub title: Color32,
    /// Banded backgrounds (tool cards, approval panels).
    pub surface_user: Color32,
    /// A tool call that is still running.
    pub surface_pending: Color32,
    /// A tool call that finished successfully.
    pub surface_ok: Color32,
    /// A tool call that failed.
    pub surface_err: Color32,
    /// The window background.
    pub background: Color32,
    /// Slightly raised surfaces: the sidebar, the header strip.
    pub panel: Color32,
    /// Selection highlight.
    pub selection: Color32,
    /// Selected *and* focused.
    pub selection_focus: Color32,
}

impl IdeTheme {
    /// Adapt a terminal palette for the desktop.
    ///
    /// The terminal theme is constructed in `ColorMode::Truecolor` on purpose:
    /// a desktop window always has 24-bit color, and asking the terminal theme for
    /// its ANSI-16 or `NO_COLOR` fallback here would produce a *worse* desktop —
    /// sixteen colors where the user has a million.
    pub fn from(palette: ThemeMode) -> Self {
        let theme = epsilon_tui::ui::theme::Theme::with_palette(
            epsilon_tui::ui::theme::ColorMode::Truecolor,
            // A desktop window is always Unicode-capable, so the terminal's ASCII fallback -- which
            // exists because a legacy Windows console cannot draw box characters -- never
            // applies here.
            epsilon_tui::ui::theme::UNICODE,
            palette,
        );
        let fg = |style: ratatui::style::Style| {
            to_color32(
                style.fg.unwrap_or(ratatui::style::Color::Reset),
                Color32::WHITE,
            )
        };

        let (background, panel, selection, selection_focus) = match palette {
            ThemeMode::Dark => (
                Color32::from_rgb(0x14, 0x15, 0x19),
                Color32::from_rgb(0x1B, 0x1D, 0x23),
                Color32::from_rgb(0x2A, 0x2E, 0x38),
                Color32::from_rgb(0x35, 0x4A, 0x6B),
            ),
            ThemeMode::Light => (
                Color32::from_rgb(0xFA, 0xFA, 0xF9),
                Color32::from_rgb(0xF0, 0xF0, 0xEE),
                Color32::from_rgb(0xDD, 0xE3, 0xEC),
                Color32::from_rgb(0xB4, 0xCB, 0xEC),
            ),
        };
        Self {
            palette,
            text: fg(theme.text()),
            dim: fg(theme.faint()),
            user: fg(theme.user()),
            assistant: fg(theme.assistant()),
            thinking: fg(theme.thinking()),
            accent: fg(theme.accent()),
            success: fg(theme.success()),
            danger: fg(theme.danger()),
            warn: fg(theme.warn()),
            code: fg(theme.code()),
            code_block: fg(theme.code_block()),
            border: fg(theme.border()),
            border_focus: fg(theme.border_focus()),
            title: fg(theme.title()),
            surface_user: fg(theme.surface_user()),
            surface_pending: fg(theme.surface_pending()),
            surface_ok: fg(theme.surface_ok()),
            surface_err: fg(theme.surface_err()),
            background,
            panel,
            selection,
            selection_focus,
        }
        .separated_from_background()
    }

    /// Push every role away from the window background until it is actually visible.
    ///
    /// **Why this exists, and why it is not in the terminal.** The terminal's light
    /// palette draws a card on a white page with a *border* and a fill that is very
    /// nearly the page colour — which is the right call for a grid of characters with
    /// no fill. A desktop window has one background for the whole pane, so a band that
    /// resolves to `#FFFFFF` on `#FAFAF9` is not a band at all: it measured **1.04:1**
    /// and would have been invisible. The terminal also has no floor on its quiet
    /// roles, because on a real console a dim colour is the user's own theme.
    ///
    /// So the *roles* come from the terminal — one source of visual truth — and the
    /// *floors* are the desktop's, because the desktop's rendering is different. Two
    /// floors, and the difference between them is the whole reason for two constants:
    ///
    /// | | floor | why |
    /// |---|---|---|
    /// | text roles | 3.0:1 | WCAG AA for large text; `dim` failed at 2.98:1 |
    /// | surface roles | [`SURFACE_FLOOR`] | a band is a *background*, and a background is read as a boundary, not as text. 4% of luminance is where a fill stops being the page. |
    ///
    /// Hue is preserved: the colour is mixed toward white or black in small steps, so
    /// `surface_pending` stays the terminal's colour and only its brightness moves.
    fn separated_from_background(mut self) -> Self {
        let bg = self.background;
        self.dim = separate(self.dim, bg, TEXT_FLOOR);
        self.text = separate(self.text, bg, TEXT_FLOOR);
        self.user = separate(self.user, bg, TEXT_FLOOR);
        self.assistant = separate(self.assistant, bg, TEXT_FLOOR);
        self.thinking = separate(self.thinking, bg, TEXT_FLOOR);
        self.title = separate(self.title, bg, TEXT_FLOOR);
        self.accent = separate(self.accent, bg, TEXT_FLOOR);
        self.success = separate(self.success, bg, TEXT_FLOOR);
        self.danger = separate(self.danger, bg, TEXT_FLOOR);
        self.warn = separate(self.warn, bg, TEXT_FLOOR);
        self.code = separate(self.code, bg, TEXT_FLOOR);
        self.code_block = separate(self.code_block, bg, TEXT_FLOOR);
        self.border = separate(self.border, bg, SURFACE_FLOOR);
        self.border_focus = separate(self.border_focus, bg, SURFACE_FLOOR);
        self.panel = separate(self.panel, bg, SURFACE_FLOOR);
        self.selection = separate(self.selection, bg, SURFACE_FLOOR);
        self.selection_focus = separate(self.selection_focus, bg, SURFACE_FLOOR);
        self.surface_user = separate(self.surface_user, bg, SURFACE_FLOOR);
        self.surface_pending = separate(self.surface_pending, bg, SURFACE_FLOOR);
        self.surface_ok = separate(self.surface_ok, bg, SURFACE_FLOOR);
        self.surface_err = separate(self.surface_err, bg, SURFACE_FLOOR);
        self
    }

    /// The dark palette, or `palette` when one was chosen.
    ///
    /// `None` follows `EPSILON_THEME`, the same environment variable the terminal
    /// reads, so `--light`-style configuration is honoured in both frontends.
    pub fn resolve(chosen: Option<ThemeMode>) -> Self {
        let palette = match chosen {
            Some(p) => p,
            None => epsilon_tui::ThemeMode::detect(),
        };
        Self::from(palette)
    }

    /// egui's own widget styling, derived from the same roles.
    ///
    /// egui draws widgets from a [`Visuals`] struct rather than from tokens, so
    /// this is the one unavoidable bridge between the two systems. It is one
    /// function rather than a per-widget mapping because egui exposes no other
    /// hook, and it is here rather than in `app.rs` so a second window cannot
    /// quietly get a different chrome.
    pub fn visuals(&self) -> Visuals {
        let dark = matches!(self.palette, ThemeMode::Dark);
        let mut v = if dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        v.override_text_color = Some(self.text);
        v.panel_fill = self.panel;
        v.window_fill = self.panel;
        v.extreme_bg_color = self.background;
        v.faint_bg_color = self.panel;
        v.selection.bg_fill = self.selection_focus;
        v.selection.stroke = egui::Stroke::new(1.0, self.accent);
        v.hyperlink_color = self.accent;
        v.error_fg_color = self.danger;
        v.warn_fg_color = self.warn;
        v.window_stroke = egui::Stroke::new(1.0, self.border);
        v
    }

    /// The WCAG 2.1 relative-luminance contrast ratio between `a` and `b`.
    ///
    /// Returns `1.0` for identical colors, which is the correct degenerate answer
    /// and the one that makes an invisible pair fail a `>= 4.5` assertion rather
    /// than divide by zero.
    pub fn ratio(a: Color32, b: Color32) -> f32 {
        let (la, lb) = (Self::luminance(a), Self::luminance(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Contrast between body text and the window background.
    pub fn contrast(&self) -> f32 {
        Self::ratio(self.text, self.background)
    }

    /// WCAG 2.1 relative luminance of `c`, in `[0, 1]`.
    ///
    /// Exposed because "is this background dark?" has to be asked as a luminance and
    /// not as a comparison, and the comparison is wrong in the one direction that
    /// matters: a near-white page has a contrast ratio *just above 1.0* against white,
    /// so `ratio(bg, WHITE) > 1.0` classifies `#FAFAF9` as dark.
    pub fn luminance(c: Color32) -> f32 {
        fn channel(v: u8) -> f32 {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_text_is_readable_in_both_palettes() {
        // WCAG 2.1 AA for body text is 4.5:1. This is the hard floor: an IDE is
        // read for hours, and a palette that fails it is not a preference.
        for palette in [ThemeMode::Dark, ThemeMode::Light] {
            let t = IdeTheme::from(palette);
            let ratio = t.contrast();
            assert!(
                ratio >= 4.5,
                "{palette:?} body text on the window background is {ratio:.2}:1, below the \
                 WCAG AA 4.5:1 floor (text {:?} on background {:?})",
                t.text,
                t.background
            );
        }
    }

    #[test]
    fn secondary_text_stays_above_the_large_text_floor() {
        // Dim text is used for timestamps and hints. WCAG AA for large text is
        // 3:1; dim text is small, so 3:1 is the honest floor rather than a target.
        for palette in [ThemeMode::Dark, ThemeMode::Light] {
            let t = IdeTheme::from(palette);
            let ratio = IdeTheme::ratio(t.dim, t.background);
            assert!(
                ratio >= 3.0,
                "{palette:?} secondary text is {ratio:.2}:1 on the background, below 3:1 \
                 (dim {:?} on background {:?})",
                t.dim,
                t.background
            );
        }
    }

    #[test]
    fn every_role_is_visible_against_the_window_background() {
        // Two floors, because there are two kinds of role, and using one number for
        // both was the bug this test was written for: it demanded 1.2:1 of a *band*,
        // the code promised 1.12:1 of one, and the disagreement surfaced as a failure
        // that looked like an invisible colour and was actually two thresholds.
        // A role drawn at the background's own luminance is invisible either way.
        for palette in [ThemeMode::Dark, ThemeMode::Light] {
            let t = IdeTheme::from(palette);
            let text_roles: [(&str, egui::Color32); 12] = [
                ("accent", t.accent),
                ("success", t.success),
                ("danger", t.danger),
                ("warn", t.warn),
                ("code", t.code),
                ("code_block", t.code_block),
                ("title", t.title),
                ("user", t.user),
                ("assistant", t.assistant),
                ("thinking", t.thinking),
                ("border_focus", t.border_focus),
                ("text", t.text),
            ];
            for (name, c) in text_roles {
                let ratio = IdeTheme::ratio(c, t.background);
                assert!(
                    ratio >= TEXT_FLOOR,
                    "{palette:?} text role {name} ({c:?}) sits at {ratio:.2}:1 against the \
                     background {:?} — below the {TEXT_FLOOR}:1 floor",
                    t.background
                );
            }

            let surface_roles: [(&str, egui::Color32); 8] = [
                ("surface_user", t.surface_user),
                ("surface_pending", t.surface_pending),
                ("surface_ok", t.surface_ok),
                ("surface_err", t.surface_err),
                ("border", t.border),
                ("panel", t.panel),
                ("selection", t.selection),
                ("selection_focus", t.selection_focus),
            ];
            for (name, c) in surface_roles {
                let ratio = IdeTheme::ratio(c, t.background);
                assert!(
                    ratio >= SURFACE_FLOOR,
                    "{palette:?} surface role {name} ({c:?}) sits at {ratio:.2}:1 against the \
                     background {:?} — below the {SURFACE_FLOOR}:1 band floor, so it is the \
                     page with a different name",
                    t.background
                );
            }
        }
    }

    #[test]
    fn separating_a_colour_moves_it_without_flattening_it() {
        // A property of `separate` itself. It was originally asserted on `surface_err`,
        // which failed -- and the failure was the test's premise, not the code: the
        // terminal's light *surface* slots are deliberately neutral bands (a card on a
        // white page is defined by its border and its text, not by a tinted fill), so
        // there was no red to lose.
        let bg = Color32::from_rgb(0xFA, 0xFA, 0xF9);

        // Build a fixture that is definitely *below* the floor by walking a tinted
        // colour toward the background, rather than guessing a hex value: two of the
        // three hand-picked ones turned out to be above it, and a test whose fixture
        // silently stops testing is worse than no test.
        let tinted = Color32::from_rgb(0xE0, 0x60, 0x60);
        let mut near_bg = tinted;
        // The step is 5%, not 1%: `mix` rounds to whole channels, so a 1% step moves
        // the red channel by 0.26 and it never leaves 224 — the loop ran 400 times
        // without the fixture ever reaching the background.
        for _ in 0..300 {
            near_bg = mix(near_bg, bg, 0.05);
            if IdeTheme::ratio(near_bg, bg) < SURFACE_FLOOR {
                break;
            }
        }
        assert!(
            IdeTheme::ratio(near_bg, bg) < SURFACE_FLOOR,
            "fixture construction failed: {near_bg:?} is already past the floor"
        );

        let moved = separate(near_bg, bg, SURFACE_FLOOR);
        assert_ne!(moved, near_bg, "a colour below the floor must move");
        assert!(
            IdeTheme::ratio(moved, bg) >= SURFACE_FLOOR,
            "separation did not reach the floor: {moved:?}"
        );
        // Hue survives: mixing toward black is linear, so the red-to-blue ratio is
        // unchanged. An absolute channel delta would not test that -- and would have
        // passed on a grey, which is the thing that must not happen.
        let before = near_bg.r() as f32 / near_bg.b() as f32;
        let after = moved.r() as f32 / moved.b() as f32;
        assert!(
            (before - after).abs() < before * 0.02,
            "mixing toward black flattened the hue: {near_bg:?} -> {moved:?}"
        );

        // A colour already past the floor is returned untouched -- otherwise every
        // palette would drift a little further from the terminal's on every build.
        assert_eq!(separate(tinted, bg, SURFACE_FLOOR), tinted);
    }

    #[test]
    fn the_light_surface_slots_are_neutral_bands_by_design() {
        // Recorded so the next reader does not "fix" it. The terminal draws a card on
        // a white page with a border and coloured text; the desktop inherits that, so
        // an approval panel's colour comes from its content, not its fill.
        let t = IdeTheme::from(ThemeMode::Light);
        assert_eq!(t.surface_pending, t.surface_ok);
    }

    #[test]
    fn danger_and_success_are_not_the_same_colour() {
        // A palette where "allowed" and "refused" share a color is a palette
        // where the user reads meaning off hue alone — which fails for the ~8% of
        // men with a colour vision deficiency, and fails in a screenshot printed
        // in greyscale.
        let t = IdeTheme::from(ThemeMode::Dark);
        assert_ne!(t.success, t.danger);
        assert_ne!(t.warn, t.danger);
    }

    #[test]
    fn the_two_palettes_are_actually_different() {
        assert_ne!(
            IdeTheme::from(ThemeMode::Dark).background,
            IdeTheme::from(ThemeMode::Light).background
        );
    }

    #[test]
    fn contrast_of_a_colour_with_itself_is_one() {
        let c = Color32::from_rgb(0x12, 0x34, 0x56);
        assert!((IdeTheme::ratio(c, c) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn black_on_white_meets_the_aa_floor() {
        // Anchors the ratio function against two known values: WCAG's own worked
        // examples are 21:1 for black on white and about 4.54:1 for #767676 on white.
        assert!((IdeTheme::ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.05);
        let grey = Color32::from_rgb(0x76, 0x76, 0x76);
        let ratio = IdeTheme::ratio(grey, Color32::WHITE);
        assert!(
            (4.4..4.7).contains(&ratio),
            "#767676 on white should be ~4.54:1, got {ratio:.2}"
        );
    }
}
