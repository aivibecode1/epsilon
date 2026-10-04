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
use crate::ui::layout::{Priority, clamp_line, display_width, room_beside, side_by_side};
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
    /// The session's goal, when one is set: `● goal 2/256`.
    ///
    /// Immediately after the mode and before the key hints, and emitted whole or not at all —
    /// the M201 rule, and it matters more here than anywhere: a goal badge cut in half reads
    /// `● goal 2/2`, which is a goal one round from its cap rather than one round into a long
    /// one. It outranks every hint because it is the only fragment on the row that says what
    /// the session is *for*.
    pub goal: Option<String>,
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
    /// **Order is survival, and what survives on a realistic row is the exit —
    /// not the second hint.** A narrow row is decided in three steps, in this
    /// order: the location and branch take what they need ([`Priority::Right`],
    /// so the hints are the half that yields); the mode word is pushed first and
    /// budgets nothing away, which is why the row still says which keyboard it
    /// belongs to; and the badge is emitted whole or not at all, because a
    /// truncated `^ 12/40` is a wrong total rather than a shorter badge.
    ///
    /// The exit is pushed before "further back" because it is the one key on this
    /// row the reader cannot infer from the screen: a frozen view looks exactly
    /// like a slow model. The measured consequence is worth stating, because the
    /// obvious reading of "the first one survives" is not what happens at every
    /// width: with a realistic location — `~/Desktop/My Agent Space/Epsilon · main`
    /// is 39 columns — a 60-column row prints `insert ^ 12/40 · g`, so the exit's
    /// key is what the reader keeps and its words are what they lose, while
    /// `further back` never appears at all. The badge is the part of the row that
    /// is never half-shown. The test below pins this at 60 columns rather than at
    /// 80, where everything fits and the rule cannot be observed at all.
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

    /// The approval footer for an ask that **cannot be approved as shown**.
    ///
    /// The approve keys are withdrawn in `handle_approval_key` when the request carries
    /// display-steering characters, and a footer that still advertised them would be the
    /// defect this repository tracks by name: a key the window does not act on, printed as
    /// though it would. Every `y` in this build once led somewhere; it must be the same
    /// everywhere or the footer is decoration.
    pub fn approval_deny_only() -> Self {
        Self {
            mode: Some("approval — cannot approve as shown".into()),
            keys: vec![
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
///
/// **The hints are budgeted here, before the layout runs, because a hint is a
/// unit and the layout cannot know that.** `side_by_side` cuts at a character, so
/// a row one column too long for its hints renders the last one as `pageup sc` —
/// which is not a shorter hint, it is a *key that does not exist*, and it was
/// measured on a 60-column frame (`audit/evidence/frames/m199-baseline/`). The
/// same argument the header makes for `R0 W0 CH n/a` one row up, one row down: at
/// 60 columns `pageup` on its own is honest, `pageup sc` is not, and the action
/// can go without the key.
///
/// The budget is `room_beside` — the same subtraction and the same one-column gap
/// the layout reserves — so the row a caller budgets and the row the layout emits
/// cannot disagree. What survives is whatever fits in priority order, and a hint
/// that does not fit is skipped rather than ending the run, so the short
/// `ctrl+p palette` still appears on a row where the long `pageup scroll back`
/// did not.
pub fn draw(f: &mut Frame<'_>, area: Rect, theme: &Theme, hints: &Hints) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width as usize;

    // The right half is built and measured first because it is the half that
    // cannot yield.
    let mut right: Vec<Span<'static>> = Vec::new();
    if let Some(location) = &hints.location {
        right.push(Span::styled(location.clone(), theme.faint()));
    }
    if let Some(branch) = &hints.branch {
        right.push(Span::styled(" · ", theme.faint()));
        right.push(Span::styled(branch.clone(), theme.faint()));
    }
    let budget = if right.is_empty() {
        // No location: nothing to be separated from, and the whole row is the
        // hints'. Reserving the gap here would leave one column permanently blank
        // on a row that has no second half to put in it.
        width
    } else {
        room_beside(&right, width)
    };
    let mut used = 0usize;

    let mut left: Vec<Span<'static>> = Vec::new();
    // The mode leads the row: the same chord means different things per mode
    // (`enter send` / `enter queue`), and the word is what tells them apart. It is
    // the highest-priority part of the left half and the last to go — but it goes
    // *whole*, like everything else here: `insert` cut to `in` is not a shorter
    // mode, it is a word the UI does not use. (Below 44 columns the layout's
    // clamp is still the backstop, and below 44 nothing is drawn at all — the
    // frame reports itself too small.)
    if let Some(mode) = &hints.mode {
        let span = Span::styled(mode.clone(), theme.accent());
        let w = display_width(&span.content);
        if w <= budget {
            used += w;
            left.push(span);
        }
    }
    // Where the view is, immediately after the mode and before the keys: it
    // qualifies both. `warn` rather than `accent` because a view that stopped
    // following the run is a state to notice, not a brand colour — and on a
    // no-colour terminal it is still the word's glyph and digits that carry it.
    //
    // It is emitted whole or not at all: `^ 12/40` is a position and `^ 12` is a
    // lie about one, while no badge at all is merely less information. Budgeting it
    // rather than protecting it after the fact is what makes that structural — the
    // room a dropped badge does not use goes to the hints, which is the order the
    // reader wants anyway.
    // The goal comes first among the optional segments: it is the only one that says what the
    // session is for, and `goal 2/256` cut to `goal 2/2` is the worst lie on the row.
    if let Some(goal) = &hints.goal {
        let span = Span::styled(goal.clone(), theme.warn());
        let w = display_width(&span.content);
        let sep = usize::from(!left.is_empty()) * 3; // " · "
        if used + sep + w <= budget {
            if sep > 0 {
                left.push(Span::styled(" · ", theme.faint()));
            }
            used += sep + w;
            left.push(span);
        }
    }
    if let Some(badge) = hints.view.badge(theme) {
        let span = Span::styled(badge, theme.warn());
        let w = display_width(&span.content);
        let sep = usize::from(!left.is_empty());
        if used + sep + w <= budget {
            if sep == 1 {
                left.push(Span::raw(" "));
            }
            used += sep + w;
            left.push(span);
        }
    }
    for (key, action) in hints.keys.iter() {
        let sep = if left.is_empty() { 0 } else { 3 }; // " · "
        let key_w = display_width(key);
        let full_w = sep + key_w + 1 + display_width(action);
        if used + full_w <= budget {
            if sep > 0 {
                left.push(Span::styled(" · ", theme.faint()));
            }
            // The key is the affordance and gets the brighter treatment; the
            // action is the explanation and stays quiet. Reading down the keys
            // alone should still tell an experienced user what is available.
            left.push(Span::styled(key.clone(), theme.dim_strong()));
            left.push(Span::styled(format!(" {action}"), theme.faint()));
            used += full_w;
        } else if used + sep + key_w <= budget {
            // The key without its action: a real key, honestly named, one word
            // short of an explanation. Never a fragment of either.
            if sep > 0 {
                left.push(Span::styled(" · ", theme.faint()));
            }
            left.push(Span::styled(key.clone(), theme.dim_strong()));
            used += sep + key_w;
        }
    }

    let line = side_by_side(left, right, width, Priority::Right);
    let line = clamp_line(line, width);
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
    fn the_position_badge_is_never_half_written_at_any_width() {
        // The finding this answers was a comment claiming the *position* survives
        // 60 columns. Measured, the 60-column claim turned out to be the easy
        // half — the badge fits there even with no protection at all — and the
        // defect was one width-band lower, where the trim cuts *inside* the
        // fraction and the row says `^ 12` or `^ 1` for a 40-row transcript.
        // A wrong total is worse than a missing badge: absent information is
        // noticed, and a wrong number is not.
        //
        // So the assertion is the invariant rather than one width: across every
        // width down to where the row stops being a row, the badge is either
        // whole or gone. The arm that proves it discriminates is a `None` in
        // `draw`'s `whole_span`, and it produces `^`, `^ 1`, `^ 12`, `^ 12/` and
        // `^ 12/4` in exactly this sweep.
        let hints = Hints {
            location: Some("~/work/api".into()),
            branch: Some("main".into()),
            ..Hints::idle_at(&defaults(), frozen_view())
        };
        let badge = ascii_badge(&frozen_view());
        let (marker, count) = badge.split_once(' ').expect("badge is `marker count`");
        let mut half_written = Vec::new();
        let mut saw_badge = 0usize;
        let mut saw_it_dropped = 0usize;
        for width in 26u16..=60 {
            let line = render(width, &hints);
            assert!(
                crate::ui::layout::display_width(&line) <= width as usize,
                "width {width} overflowed: {line:?}"
            );
            if let Some(at) = line.find(marker) {
                saw_badge += 1;
                let tail = &line[at + marker.len()..];
                if !tail.trim_start().starts_with(count) {
                    half_written.push(format!("{width}: {line:?}"));
                }
            } else {
                saw_it_dropped += 1;
            }
        }
        assert!(
            half_written.is_empty(),
            "the badge states a position, so part of it is a wrong one: {half_written:#?}"
        );
        // Both arms of the sweep have to actually run, or a `draw` that never
        // emitted a badge at all would pass it — the failure mode this whole
        // section exists for (a control that passes proves nothing).
        assert!(saw_badge > 0, "the sweep never saw a badge");
        assert!(
            saw_it_dropped > 0,
            "the sweep never exercised the drop: the badge fits at every width"
        );
    }

    /// **No footer row ever prints half a hint.** The measured defect was
    /// `… pagedown scroll forward · ct …\Epsilon\crates\epsilon-tui` at 60
    /// columns: the layout cut `ctrl+enter` mid-word, so the row advertised a key
    /// that does not exist, and it abutted the location so the two read as one
    /// run. Both halves are asserted here, at every width from 20 to the point
    /// where the row is nothing but the location.
    ///
    /// The assertion is on the *frame*, not on `Hints`, because every part of this
    /// row was correct as data: `Hints::idle` named real bindings, `keys_text`
    /// joined them properly, and the loss happened in the cut.
    #[test]
    fn no_footer_row_ever_prints_half_a_hint_or_touches_the_location() {
        let hints = Hints {
            location: Some("~/work/api".into()),
            branch: Some("main".into()),
            ..Hints::idle(&defaults())
        };
        // Every legal rendering of a hint, longest first so a prefix of a full
        // hint cannot match the short form and pass.
        let mut legal: Vec<String> = Vec::new();
        for (key, action) in &hints.keys {
            legal.push(format!("{key} {action}"));
            legal.push(key.clone());
        }
        for width in 20u16..=120 {
            let line = render(width, &hints);
            assert!(
                crate::ui::layout::display_width(&line) <= width as usize,
                "width {width} overflowed: {line:?}"
            );
            // Each `key [action]` the row carries must be one of the legal forms.
            let head = line.split(" ~/work/api").next().unwrap_or(&line);
            for part in head.split(" · ").skip(1) {
                let part = part.trim_end();
                if part.is_empty() {
                    continue;
                }
                assert!(
                    legal.iter().any(|l| l == part),
                    "width {width}: `{part}` is half a hint: {line:?}"
                );
            }
            // And the location never touches the run before it. Skips the widths
            // where the location itself is trimmed, which is a different question.
            if let Some(at) = line.find("~/work/api")
                && at > 0
            {
                assert_eq!(
                    &line[at - 1..at],
                    " ",
                    "width {width}: the hints abut the location: {line:?}"
                );
            }
        }
    }

    /// **The same property as the test above, over the footer states that test does not reach.**
    ///
    /// The test above is named for *"no footer row"* but exercises **one** state —
    /// `Hints::idle`. The two approval footers are a different hint list, a different `mode`
    /// string, and the widest rows in the UI, and nothing asserted the same property over
    /// them. That is the gap this closes.
    ///
    /// **And a correction, because the first version of this comment claimed a defect that is
    /// not one.** A frame dump read on 2026-10-04 shows the approval row at **80×24** as:
    ///
    /// ```text
    /// |approval · y allow once · a always · n deny · e           ...\crates\epsilon-tui|
    /// ```
    ///
    /// and I read the trailing `e` as `deny + guidance` cut mid-label. It is — but that is
    /// **not** the defect this file names. `footer.rs:308-314` draws the distinction exactly:
    /// `pageup sc` is a defect because it advertises *a key that does not exist*, while
    /// `pageup` on its own is honest — *"the action can go without the key."* A bare `e` names
    /// a key that is really bound and really handled; only its label is missing. **Degraded,
    /// and honest.** This test's `legal` set encodes that distinction by accepting both
    /// `"{key} {action}"` and a bare `key`, and it passes.
    ///
    /// What the frame did establish is worth keeping: the budget loop emits the key and the
    /// action **together or not at all** (`footer.rs:400-412`), so the bare `e` came from
    /// `side_by_side` cutting at a character *after* the budget had approved the pair. The
    /// pre-budget is therefore not what produced it. Nothing here is broken today; the
    /// property is now asserted over the states where a break would show first.
    #[test]
    fn the_approval_footer_never_prints_half_a_hint_either() {
        const LOCATION: &str = r"...\crates\epsilon-tui";
        for (name, base) in [
            ("approval", Hints::approval()),
            ("approval_deny_only", Hints::approval_deny_only()),
        ] {
            let hints = Hints {
                location: Some(LOCATION.into()),
                ..base
            };
            // Every legal rendering of a hint, longest first so a prefix of the full
            // form cannot match the short one and pass.
            let mut legal: Vec<String> = Vec::new();
            for (key, action) in &hints.keys {
                legal.push(format!("{key} {action}"));
                legal.push(key.clone());
            }
            for width in 20u16..=120 {
                let line = render(width, &hints);
                assert!(
                    crate::ui::layout::display_width(&line) <= width as usize,
                    "{name} at width {width} overflowed: {line:?}"
                );
                let head = line.split(LOCATION).next().unwrap_or(&line);
                for part in head.split(" · ").skip(1) {
                    let part = part.trim_end();
                    if part.is_empty() {
                        continue;
                    }
                    assert!(
                        legal.iter().any(|l| l == part),
                        "{name} at width {width}: `{part}` is half a hint — the key without \
                         its action. That is a key the reader cannot look up: {line:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_frozen_row_at_sixty_columns_keeps_the_position_and_the_exit() {
        // The finding this replaces promised 60-column survival from an *idle* row
        // rendered at 80: both halves were wrong, because the row a narrow
        // terminal degrades is the frozen one — the widest footer state there is
        // (badge + exit + paging) — and 80 columns is a width at which every hint
        // already fits, so the test could not see the rule it named.
        //
        // What it pins now is the row a reader at 60 columns actually gets, and
        // the division of labour is worth stating: that the badge is never *half*
        // written is the sweep above; that it is here at all, beside a realistic
        // location, is this test.
        let bindings = defaults();
        let hints = Hints {
            // Realistic on purpose: 39 columns with the branch. A short
            // placeholder path would leave the hints room they do not have on a
            // real screen and make this pass for the wrong reason.
            location: Some("~/Desktop/My Agent Space/Epsilon".into()),
            branch: Some("main".into()),
            ..Hints::idle_at(&bindings, frozen_view())
        };
        let line = render(60, &hints);

        // The position survives, whole, which is what a `whole_span` buys here: on
        // this row it fits beside the location.
        assert!(
            line.contains(&ascii_badge(&frozen_view())),
            "60 columns lost the position the frozen row exists to state: {line:?}"
        );
        // The whole row may not wrap, which is the property the fix must not
        // trade away for the badge.
        assert!(
            crate::ui::layout::display_width(&line) <= 60,
            "the frozen row overflowed: {line:?}"
        );
        // What actually gives way at this width, asserted rather than asserted
        // *about*: the last hint is gone from the row. This is the half of the old
        // comment that was simply false — it said the position and the exit
        // survive, and measured, the exit's own text does not.
        assert!(
            !line.contains("scroll back"),
            "60 columns kept both hints, so this test is not exercising the trim: {line:?}"
        );
        // And the exit is the hint that outlived the other one — the order in
        // `Hints::frozen` is the reason, and this is the assertion that fails if
        // the two pushes are ever swapped. Its *key* is asserted rather than its
        // whole action text, because the exit's own words are cut too and by a
        // number this test would otherwise be pinning twice over.
        let live = bindings.key_text("app.transcript.live").expect("bound");
        assert!(
            line.contains(&format!("· {live} ")),
            "the exit key must be the one on the row: {line:?}"
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
    fn the_badge_is_drawn_onto_the_row_with_the_warn_colour() {
        // The pixels, not the string: `Hints` carrying a view and `View::badge`
        // returning text are both satisfiable while nothing draws either. This is
        // the assertion that fails if the badge is dropped from `draw`.
        let hints = Hints::idle_at(&defaults(), frozen_view());
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).expect("test terminal");
        terminal
            .draw(|f| draw(f, f.area(), &theme, &hints))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let row = crate::ui::layout::row_text(buffer, 0, 60);
        let badge = ascii_badge(&frozen_view());
        let x = row
            .find(&badge)
            .unwrap_or_else(|| panic!("the badge {badge:?} is not on the row: {row:?}"));
        let cell = &buffer[(x as u16, 0)];
        assert_eq!(
            cell.style().fg,
            Some(ratatui::style::Color::Yellow),
            "the badge must not be styled like the hints beside it"
        );
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
