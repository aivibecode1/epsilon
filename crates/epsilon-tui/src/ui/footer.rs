//! The footer: the keys that work *right now*, and where the session lives.
//!
//! ```text
//! insert · enter send · ctrl+j newline · pgup scroll back    ~/work/api · main
//! insert ↑ 12/40 · g live · pgup scroll back                 ~/work/api · main
//! ```
//!
//! Four rules make a key hint useful rather than noise:
//!
//! * **Only live keys.** A footer that lists `esc stop` while nothing is running
//!   trains the reader to stop reading it. The set is chosen per state.
//! * **The key is whatever is bound to the action.** Every hint is built from
//!   [`KeybindingsManager::key_text`], never from a literal, so a rebound key
//!   changes its own hint and a binding with nothing behind it produces no hint
//!   at all. The footer used to advertise `? help`, which was not a keybinding:
//!   `?` did nothing until the user typed it *and* pressed Enter, and the keys a
//!   session actually needs — PageUp/PageDown, Ctrl+O, Ctrl+X — went unmentioned.
//! * **The location is not a key.** The workspace path and branch answer "which
//!   project am I about to let this thing edit", which is a safety question, so
//!   it gets the right-hand side where the eye lands after the hints.
//! * **A view that stopped following the run says so.** Scrolling up used to be
//!   silent: the transcript froze and nothing on screen admitted it, so a reader
//!   who paged up during a stream could not tell "the model is quiet" from "I am
//!   reading the past". The second line above is that state: a word, a position,
//!   and the key that comes back ([`View::badge`]).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;

use crate::keybindings::KeybindingsManager;
use crate::ui::layout::{Priority, clamp_line, side_by_side};
use crate::ui::theme::Theme;

/// Where the transcript view sits relative to the live tail.
///
/// One value instead of two flags, because the two facts are one state: a view is
/// either following the run or parked above it, and "parked" without a position
/// is what made scrolling silent in the first place.
///
/// The position is a **row count and a total** (`12/40`), not a scrollbar. The
/// transcript occupies the body region, whose width is decided by the layout
/// rather than by this widget, and a one-column scrollbar down the right edge
/// would have to be drawn by whoever owns that rectangle — the footer cannot
/// reach it. A fraction also answers the question the reader actually has
/// ("how much of this is below me?") in the same row as the key that resolves it,
/// which a thumb position on a 20-row viewport cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct View {
    /// True when the view has stopped following new rows. New rows then push the
    /// frozen window's *offset* rather than its contents, so this is the one bit
    /// the footer needs to explain the screen.
    pub frozen: bool,
    /// Transcript rows below the visible window: how far back the reader is.
    /// Zero means the last row of the transcript is on screen.
    pub rows_below: usize,
    /// Total transcript rows, including the part on screen.
    pub total_rows: usize,
}

impl View {
    /// The word that says the transcript is not following, plus the position —
    /// `↑ 12/40` — or `None` while the view is live.
    ///
    /// The count is part of the same badge rather than a separate field so a
    /// narrow terminal cannot show the word and lose the number: they are one
    /// answer to "where am I", and half of it is a worse lie than none.
    ///
    /// Suppressed when the reader is at the bottom (`rows_below == 0`) even if
    /// the frozen flag is set: a transcript that fits on one screen has nothing
    /// to scroll to, and "frozen 0/6 rows back" would be true and useless.
    pub fn badge(&self, theme: &Theme) -> Option<String> {
        if !self.frozen || self.rows_below == 0 {
            return None;
        }
        Some(format!(
            "{} {}/{}",
            theme.glyphs.back, self.rows_below, self.total_rows
        ))
    }
}

/// What the footer should say, chosen by the caller from the live app state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hints {
    /// The mode the keys belong to, shown first: `insert`, `run`, `approval`.
    ///
    /// Without it `esc stop` and `esc dismiss` read the same, and the reader has
    /// to work out from the rest of the row which screen they are looking at.
    pub mode: Option<String>,
    /// Pairs of `(key, action)`, in the order they should read.
    pub keys: Vec<(String, String)>,
    /// The workspace path, already shortened for display.
    pub location: Option<String>,
    /// The git branch, when the workspace is a repository.
    pub branch: Option<String>,
    /// Set while a run is in flight, so the footer can say how to stop it, and
    /// how many messages are waiting behind it.
    pub queued: usize,
    /// Where the transcript view sits: live, or frozen above the tail.
    pub view: View,
}

impl Hints {
    /// The hints for an idle session at the composer.
    ///
    /// `bindings` is the live table: every key here is resolved through it, so
    /// this function cannot name a key the session would not act on.
    ///
    /// Order is priority, because a narrow row keeps the tail: the location
    /// outranks the hints ([`Priority::Right`]) and the trim removes the *near*
    /// end of what is left, so the last hint written is the one that survives
    /// 80 columns. Scrolling goes early for that reason — nothing else on screen
    /// mentions it, while `enter send` is stated by the composer's own
    /// placeholder and stated again by the row itself.
    pub fn idle(bindings: &KeybindingsManager) -> Self {
        Self::idle_at(bindings, View::default())
    }

    /// The idle hints for a view that may be frozen.
    ///
    /// A frozen view is a different keyboard: the reader is looking at the past
    /// and the one key they cannot guess is the one that comes back, so it leads
    /// the set and `send` — which would queue a message they cannot see the
    /// bottom of — steps aside for it. The exit is stated first and the way
    /// further back is stated next; both are live in this state, and both are
    /// resolved from the table like every other hint.
    pub fn idle_at(bindings: &KeybindingsManager, view: View) -> Self {
        if view.frozen {
            return Self::frozen(bindings, view);
        }
        let mut hints = Self::at_composer(bindings, "insert");
        Self::push_transcript_keys(&mut hints, bindings);
        hints.push(bindings, "tui.input.newLine", "newline");
        hints.push(bindings, "app.palette", "palette");
        hints
    }

    /// The hints for a running session.
    pub fn running(bindings: &KeybindingsManager, queued: usize) -> Self {
        Self::running_at(bindings, queued, View::default())
    }

    /// The running hints for a view that may be frozen.
    pub fn running_at(bindings: &KeybindingsManager, queued: usize, view: View) -> Self {
        if view.frozen {
            let mut hints = Self::frozen(bindings, view);
            hints.push(bindings, "app.interrupt", "stop");
            hints.queued = queued;
            if queued > 0 {
                hints.keys.push(("queued".to_string(), queued.to_string()));
            }
            return hints;
        }
        let mut hints = Self::at_composer(bindings, "run");
        // The same chord, a different promise: while a run is in flight Enter
        // queues the message rather than sending it.
        hints.keys[0].1 = "queue".to_string();
        // While the model is talking, scrolling is what a reader reaches for:
        // the tail is streaming and the part they want to re-read is above it.
        Self::push_transcript_keys(&mut hints, bindings);
        hints.push(bindings, "app.interrupt", "stop");
        if queued > 0 {
            hints.keys.push(("queued".to_string(), queued.to_string()));
        }
        hints.push(bindings, "app.palette", "palette");
        hints.queued = queued;
        hints
    }

    /// The hints while the view is frozen: go further back, or come back.
    ///
    /// `mode` is deliberately unchanged from the live row. The reader is still
    /// typing into a live composer — only the transcript below it stopped
    /// following — and a mode word that changed would claim a keyboard state the
    /// app does not have. What changed is stated by the position badge, which
    /// lives on this row and nowhere else ([`View::badge`]).
    ///
    /// **Order is survival.** The existing narrow-row test in this module pinned
    /// the rule the hard way: the hint pushed *first* is the one a 60-column row
    /// beside a workspace path keeps. The exit is pushed first for that reason —
    /// it is the one key on this row the reader cannot infer from the screen,
    /// because a frozen view looks exactly like a slow model. At 80 columns both
    /// hints print; at 60 the position and the exit survive and "further back" is
    /// what gives way.
    ///
    /// PageDown (toward live) is deliberately not listed while frozen: it is the
    /// same journey as the exit taken incrementally, and it is the first thing a
    /// reader who has scrolled up has already discovered.
    fn frozen(bindings: &KeybindingsManager, view: View) -> Self {
        let mut hints = Self {
            mode: Some("insert".to_string()),
            view,
            ..Self::default()
        };
        hints.push(bindings, "app.transcript.live", "jump to live");
        hints.push(bindings, "app.transcript.pageUp", "further back");
        hints
    }

    /// The two paging keys, in the order a narrow row keeps them.
    ///
    /// Shared by both live rows so PageUp cannot be advertised in one state and
    /// not the other: they are the same binding and the same promise.
    fn push_transcript_keys(hints: &mut Self, bindings: &KeybindingsManager) {
        hints.push(bindings, "app.transcript.pageUp", "scroll back");
        hints.push(bindings, "app.transcript.pageDown", "scroll forward");
    }

    /// The hints while a permission ask owns the keyboard.
    ///
    /// The decision keys are literals on purpose: `handle_approval_key` matches
    /// `y`/`a`/`n`/`e` by character and there is no binding id for them, so a
    /// resolved chord here would be a claim about the table that the table
    /// cannot make. Naming the letter that is actually handled is the honest
    /// version — and if these ever move into [`crate::keybindings::DEFAULTS`],
    /// this is the function that changes with them.
    pub fn approval() -> Self {
        Self {
            mode: Some("approval".into()),
            keys: vec![
                ("y".into(), "allow once".into()),
                ("a".into(), "always".into()),
                ("n".into(), "deny".into()),
                ("e".into(), "deny + guidance".into()),
            ],
            ..Self::default()
        }
    }

    /// The chord that sends, plus the mode word. The mode is not decoration:
    /// `esc stop` in the footer of an idle session is a lie the reader only
    /// discovers by pressing it.
    fn at_composer(bindings: &KeybindingsManager, mode: &str) -> Self {
        let mut hints = Self {
            mode: Some(mode.to_string()),
            ..Self::default()
        };
        hints.push(bindings, "tui.input.submit", "send");
        hints
    }

    /// Append one resolved hint, or nothing when the binding does not resolve.
    ///
    /// This is what makes a dead hint unrepresentable rather than merely
    /// discouraged: there is no branch here that pushes a literal key.
    fn push(&mut self, bindings: &KeybindingsManager, id: &str, action: &str) {
        if let Some(key) = bindings.key_text(id) {
            self.keys.push((key, action.to_string()));
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
    // The mode leads the row: the same chord means different things per mode
    // (`enter send` / `enter queue`), and the word is what tells them apart.
    if let Some(mode) = &hints.mode {
        left.push(Span::styled(mode.clone(), theme.accent()));
    }
    // Where the view is, immediately after the mode and before the keys: it
    // qualifies both. `warn` rather than `accent` because a view that stopped
    // following the run is a state to notice, not a brand colour — and on a
    // no-colour terminal it is still the word's glyph and digits that carry it.
    if let Some(badge) = hints.view.badge(theme) {
        if !left.is_empty() {
            left.push(Span::raw(" "));
        }
        left.push(Span::styled(badge, theme.warn()));
    }
    for (key, action) in hints.keys.iter() {
        if !left.is_empty() {
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
    use std::collections::HashMap;

    /// The live table with no user overrides: what a session with no
    /// `keybindings.json` runs on.
    fn defaults() -> KeybindingsManager {
        KeybindingsManager::from_map(HashMap::new())
    }

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

    /// Everything the row actually says, with nothing cut off.
    ///
    /// The hints are trimmed from their far end when the location needs the
    /// room — that is [`side_by_side`]'s job and it is asserted separately — so
    /// this renders at a width where the whole set fits and the assertions are
    /// about the strings rather than about the trim.
    fn full_line(hints: &Hints) -> String {
        render(200, hints)
    }

    #[test]
    fn idle_hints_name_only_keys_that_do_something() {
        let hints = Hints::idle(&defaults());
        let text = hints.keys_text();
        assert!(text.contains("enter send"), "{text}");
        assert!(text.contains("ctrl+j newline"), "{text}");
        assert!(text.contains("ctrl+p palette"), "{text}");
        assert!(!text.contains("stop"), "nothing is running: {text}");
        // The scroll keys a real session needs. They were reachable and
        // unmentioned before this: `KeyCode::PageUp` was handled in the composer
        // and no row of the UI ever said so.
        assert!(text.contains("pageup scroll back"), "{text}");
        assert!(text.contains("pagedown scroll forward"), "{text}");
    }

    #[test]
    fn the_footer_names_the_resolved_chord_and_never_a_dead_key() {
        // The defect this replaces: `? help` was advertised and `?` was not a
        // binding at all — it did nothing until the user typed it *and* pressed
        // Enter. A hint the session would not act on is worse than no hint.
        let bindings = defaults();
        let hints = Hints::idle(&bindings);
        let line = full_line(&hints);

        // Every hint on the row is the chord the binding manager reports for
        // some real binding — resolve it back and check it fires.
        for (key, action) in &hints.keys {
            assert!(
                line.contains(&format!("{key} {action}")),
                "the row must carry `{key} {action}`: {line:?}"
            );
            assert!(
                bindings.key_text_round_trips(key),
                "`{key} {action}` names a key that resolves to no binding: {line:?}"
            );
        }
        // And the specific promise that was false.
        assert!(
            !line.contains("? help"),
            "`?` is not a keybinding and must not be advertised: {line:?}"
        );
        assert!(
            !hints.keys.iter().any(|(key, _)| key == "?"),
            "no hint may be built from a literal key again"
        );
    }

    #[test]
    fn a_rebound_key_changes_its_own_hint() {
        // The property that makes a hint honest rather than merely correct
        // today: the row is built from the table, so rebinding an action
        // rebinds the text. Hardcoded, this test cannot pass.
        let mut map = HashMap::new();
        map.insert("tui.input.submit".into(), vec!["ctrl+s".into()]);
        map.insert("app.palette".into(), vec!["ctrl+k".into()]);
        map.insert("app.transcript.pageUp".into(), vec!["ctrl+b".into()]);
        let rebound = KeybindingsManager::from_map(map);

        let line = full_line(&Hints::idle(&rebound));
        assert!(line.contains("ctrl+s send"), "{line:?}");
        assert!(line.contains("ctrl+k palette"), "{line:?}");
        assert!(line.contains("ctrl+b scroll back"), "{line:?}");
        assert!(
            !line.contains("ctrl+p"),
            "the default chord survived a rebinding: {line:?}"
        );

        // The same table drives the running row, so the stop key moves too.
        let run = full_line(&Hints::running(&rebound, 0));
        assert!(run.contains("esc stop"), "{run:?}");
    }

    #[test]
    fn the_mode_leads_the_row() {
        // `enter send` and `enter queue` are the same chord and different
        // promises; the mode word is what tells them apart, and without it the
        // reader has to infer the state from the rest of the row.
        let bindings = defaults();
        assert!(full_line(&Hints::idle(&bindings)).starts_with("insert · "));
        let run = full_line(&Hints::running(&bindings, 0));
        assert!(run.starts_with("run · "), "{run:?}");
        assert!(run.contains("enter queue"), "{run:?}");
        assert!(run.contains("esc stop"), "{run:?}");
    }

    #[test]
    fn running_hints_offer_the_stop_key_and_the_queue_count() {
        let bindings = defaults();
        let text = Hints::running(&bindings, 0).keys_text();
        assert!(text.contains("esc stop"), "{text}");
        let queued = Hints::running(&bindings, 2).keys_text();
        assert!(queued.contains('2'), "{queued}");
    }

    #[test]
    fn approval_hints_are_the_approval_keys() {
        let text = Hints::approval().keys_text();
        for key in ["y allow once", "a always", "n deny", "e deny + guidance"] {
            assert!(text.contains(key), "missing {key} in {text}");
        }
        assert_eq!(Hints::approval().mode.as_deref(), Some("approval"));
    }

    #[test]
    fn the_location_survives_a_narrow_terminal() {
        let hints = Hints {
            location: Some("~/work/api".into()),
            branch: Some("main".into()),
            ..Hints::idle(&defaults())
        };
        for width in [30u16, 50, 80, 120] {
            let line = render(width, &hints);
            assert_eq!(
                crate::ui::layout::display_width(&line),
                width as usize,
                "width {width}"
            );
            assert!(line.contains("main"), "width {width}: {line}");
        }
        // Narrower than the location: show the location, drop the hints.
        let narrow = render(12, &hints);
        assert!(narrow.contains("api"), "{narrow}");
    }

    #[test]
    fn what_survives_a_narrow_row_is_the_hint_that_cannot_be_guessed() {
        // The trim yields from the near end (the location outranks the hints),
        // so the *tail* of the hint set is what a narrow terminal keeps. The
        // order therefore encodes priority: the scroll keys, which nothing else
        // on screen mentions, come before `newline` and `palette`.
        let hints = Hints {
            location: Some("~/work/eps-checkout".into()),
            branch: Some("main".into()),
            ..Hints::idle(&defaults())
        };
        let narrow = render(80, &hints);
        assert!(
            narrow.contains("scroll back"),
            "80 columns dropped the scroll hints, which no other row mentions: {narrow:?}"
        );
    }

    #[test]
    fn nothing_is_drawn_past_the_row_width() {
        let hints = Hints::running(&defaults(), 0);
        for width in [24u16, 40, 80] {
            let line = render(width, &hints);
            assert!(
                crate::ui::layout::display_width(&line) <= width as usize,
                "width {width} overflowed to {} cells: {line:?}",
                crate::ui::layout::display_width(&line)
            );
            assert!(
                !line.ends_with(' '),
                "trailing padding looks like a bug: {line:?}"
            );
        }
        // The frozen row is the widest state the footer has (badge + exit +
        // paging); it must not wrap at any supported width either.
        let frozen = Hints::idle_at(
            &defaults(),
            View {
                frozen: true,
                rows_below: 120,
                total_rows: 340,
            },
        );
        for width in [44u16, 60, 80] {
            let line = render(width, &frozen);
            assert!(
                crate::ui::layout::display_width(&line) <= width as usize,
                "frozen footer overflowed at {width}: {line:?}"
            );
        }
    }

    // ---- the frozen view ------------------------------------------------------
    //
    // The audit's D11: scrolling up used to be silent, `G` did nothing, and
    // nothing on screen said where the reader was. These are the three promises
    // that replaced it.

    fn frozen_view() -> View {
        View {
            frozen: true,
            rows_below: 12,
            total_rows: 40,
        }
    }

    /// The row's position badge as the ASCII palette draws it.
    ///
    /// Taken from the theme's glyph table rather than written as `^` here: the
    /// badge's marker is a repertoire decision, and a test that hardcoded one
    /// would pass on a console the UI cannot actually draw on.
    fn ascii_badge(view: &View) -> String {
        view.badge(&Theme::new(ColorMode::Ansi16, ASCII))
            .expect("a frozen view with rows below it has a position")
    }

    #[test]
    fn a_frozen_view_says_so_and_names_the_way_back() {
        let bindings = defaults();
        let hints = Hints::idle_at(&bindings, frozen_view());
        let line = full_line(&hints);

        // The state, and where in the transcript it sits.
        assert!(
            line.contains(&ascii_badge(&frozen_view())),
            "no position badge: {line:?}"
        );
        // The exit, resolved from the table like every other hint.
        let live = bindings.key_text("app.transcript.live").expect("bound");
        assert!(
            line.contains(&format!("{live} jump to live")),
            "the frozen row must name the way back (`{live}`): {line:?}"
        );
        // And the frozen row is not the live row with extra words on it: `send`
        // would queue a message the reader cannot see the end of.
        assert!(!line.contains("enter send"), "{line:?}");
    }

    #[test]
    fn the_position_badge_is_the_only_thing_that_distinguishes_frozen_from_live() {
        // Why the badge exists at all: the mode word stays `insert` — the reader
        // is still typing into a live composer, only the transcript stopped
        // following — so if the badge vanished there would be nothing on the row
        // to say the view is parked.
        let bindings = defaults();
        let live = full_line(&Hints::idle_at(&bindings, View::default()));
        let frozen = full_line(&Hints::idle_at(&bindings, frozen_view()));
        let badge = ascii_badge(&frozen_view());
        assert!(
            !live.contains(&badge),
            "a live view shows no position: {live:?}"
        );
        assert!(frozen.contains(&badge), "{frozen:?}");
        assert!(
            live.starts_with("insert · ") && frozen.starts_with("insert ^ "),
            "the mode word is stable across both: {live:?} / {frozen:?}"
        );
        // A view that is frozen but has nothing below it (a transcript shorter
        // than the screen) prints no badge: `0/6 rows back` is true and useless.
        let nothing_below = View {
            frozen: true,
            rows_below: 0,
            total_rows: 6,
        };
        assert_eq!(
            nothing_below.badge(&Theme::new(ColorMode::Ansi16, ASCII)),
            None
        );
    }

    #[test]
    fn the_frozen_hint_set_is_honest_about_the_keys_it_names() {
        // The same property the live row is held to, on the row that has the most
        // to lose from a lie: every key printed resolves back to a binding, so a
        // hint cannot outlive a rebinding.
        let bindings = defaults();
        let hints = Hints::idle_at(&bindings, frozen_view());
        assert!(hints.keys.iter().any(|(_, a)| a == "jump to live"));
        for (key, action) in &hints.keys {
            assert!(
                bindings.key_text_round_trips(key),
                "`{key} {action}` names a key that resolves to no binding"
            );
        }
        // ...and a rebinding moves the hint, which is the whole reason it is
        // resolved rather than written down.
        let mut map = HashMap::new();
        map.insert("app.transcript.live".into(), vec!["ctrl+l".into()]);
        let rebound = KeybindingsManager::from_map(map);
        let line = full_line(&Hints::idle_at(&rebound, frozen_view()));
        assert!(line.contains("ctrl+l jump to live"), "{line:?}");
        assert!(!line.contains("g jump to live"), "{line:?}");
    }

    #[test]
    fn the_position_counts_what_is_below_the_window_not_what_is_above() {
        // "12/40" has to mean something a reader can act on. Rows *below* is that
        // number: `g` returns to live, and every one of those rows is what the
        // journey back passes through. A count of rows scrolled past would read
        // as progress through the transcript, which it is not.
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let mid = View {
            frozen: true,
            rows_below: 12,
            total_rows: 40,
        };
        assert_eq!(mid.badge(&theme).as_deref(), Some("^ 12/40"));
        let top = View {
            frozen: true,
            rows_below: 40,
            total_rows: 40,
        };
        assert_eq!(top.badge(&theme).as_deref(), Some("^ 40/40"));
        // ASCII repertoire: the badge's glyph comes from the theme's table, so a
        // legacy code page gets a character it can actually draw.
        assert!(theme.glyphs.back.is_ascii());
    }
}
