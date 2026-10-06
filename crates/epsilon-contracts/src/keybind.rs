//! The product's keyboard contract: what a chord is, and which chords each front end binds.
//!
//! # Why this is here and not in a front end
//!
//! `AGENTS.md` §2.4 says *"slash commands, palette entries and keybindings are declared once and
//! read by every front end, with per-front-end capability declared as data rather than as a
//! second hand-maintained list."* Commands were the half that held: `COMMAND_HEADS` lives here and
//! both front ends read it. **Keybindings were the half that did not.** The terminal had a table
//! in `epsilon-tui`; the desktop window had seventeen literal `consume_key` reads and a hand-written
//! `"ctrl+b"` constant in its hints — so the window's shortcuts could not be rebound at all, and a
//! reader of `AGENTS.md` would conclude something the tree did not contain.
//!
//! # What "declared once" means here, precisely
//!
//! **One file, one chord grammar, one place a binding is written down.** The *grammar* is
//! genuinely shared: a user who writes `{"app.interrupt": "ctrl+q"}` means the same thing in both
//! front ends, and [`Chord::parse`] is the one function that decides what that is.
//!
//! **The tables are per front end, and that is not a dodge.** A terminal and a window are two
//! different programs: `F6` cycles a focus ring in the window and means nothing in the terminal,
//! and `ctrl+b` moves a text cursor in the terminal's composer and folds the window's sidebar.
//! `AGENTS.md` asks for *per-front-end capability declared as data* — so [`TERMINAL`] and
//! [`WINDOW`] are that data, in one module, in one format, rather than two private tables in two
//! crates. A reader looking for "what does this product bind?" finds both in one place; a reader
//! looking for "what does *this* front end bind?" filters one list.
//!
//! # Why the types here are not crossterm's or egui's
//!
//! This crate depends on neither, and adding one to declare a keyboard would invert the whole
//! dependency direction the repository is built on (`design/epsilon-architecture.md` §2). So a
//! chord is [`Chord`] — a neutral [`Key`] plus neutral [`Mods`] — and each front end converts it
//! once, at its own input boundary. The terminal's conversion is `KeyCode`/`KeyModifiers`; the
//! window's is `egui::Key`/`egui::Modifiers`.

/// Modifier bits, neutral over any UI toolkit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    /// Control.
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
    /// Alt / Option.
    pub alt: bool,
    /// The platform's other "command" key: Windows or macOS Command.
    pub command: bool,
}

impl Mods {
    /// No modifiers.
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        command: false,
    };

    /// True when these modifiers are **exactly** the other set.
    ///
    /// **Exact, and deliberately not "logical".** egui's own `consume_key` matches *logically* —
    /// extra Shift and Alt are ignored — and that is right for a widget that has no better idea,
    /// but it is how `Shift+Enter` came to be swallowed by a plain `Enter` read in the desktop
    /// composer (M314). A binding is a **choice by a person**, so it means one specific set.
    pub fn is_exactly(self, other: Self) -> bool {
        self == other
    }
}

/// A key, named in a way that both toolkits can map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A printable character, already lowercased by [`Chord::parse`].
    Char(char),
    /// Escape.
    Esc,
    /// Enter / Return.
    Enter,
    /// Tab.
    Tab,
    /// The space bar, which is [`Key::Char`] with `' '` but named because `" "` is unreadable in a
    /// table of strings.
    Space,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Insert.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// A function key, 1 through 12.
    F(u8),
}

/// One chord: a key and the modifiers it must be pressed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    /// The key.
    pub key: Key,
    /// The modifiers, compared exactly.
    pub mods: Mods,
}

impl Chord {
    /// Parse a `KeyId` — `"ctrl+p"`, `"escape"`, `"shift+f2"`.
    ///
    /// **The one place the grammar lives.** It is a verbatim move of the terminal's
    /// `parse_key_id` (`crates/epsilon-tui/src/keybindings.rs`), with `KeyCode` and
    /// `KeyModifiers` replaced by [`Key`] and [`Mods`] so that this crate keeps no UI
    /// dependency. Moving it is the point: a binding the window cannot parse is a binding the
    /// window would silently ignore, and a second parser is a second answer.
    ///
    /// Returns `None` for an empty or unrecognised spelling, which callers treat as
    /// "no binding" — never as an error to show, because a keybinding file is user data.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim().to_ascii_lowercase();
        if raw.is_empty() {
            return None;
        }
        let parts: Vec<&str> = raw.split('+').map(str::trim).collect();
        let (mods_parts, key_part) = parts.split_at(parts.len().saturating_sub(1));
        let key_str = *key_part.first()?;
        let mut mods = Mods::NONE;
        for m in mods_parts {
            match *m {
                "ctrl" | "control" => mods.ctrl = true,
                "shift" => mods.shift = true,
                "alt" | "meta" => mods.alt = true,
                "super" | "win" | "cmd" => mods.command = true,
                "" => {}
                _ => return None,
            }
        }
        let key = match key_str {
            "escape" | "esc" => Key::Esc,
            "enter" | "return" => Key::Enter,
            "tab" => Key::Tab,
            "space" => Key::Space,
            "backspace" => Key::Backspace,
            "delete" | "del" => Key::Delete,
            "insert" | "ins" => Key::Insert,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "f1" => Key::F(1),
            "f2" => Key::F(2),
            "f3" => Key::F(3),
            "f4" => Key::F(4),
            "f5" => Key::F(5),
            "f6" => Key::F(6),
            "f7" => Key::F(7),
            "f8" => Key::F(8),
            "f9" => Key::F(9),
            "f10" => Key::F(10),
            "f11" => Key::F(11),
            "f12" => Key::F(12),
            s if s.len() == 1 => Key::Char(s.chars().next()?),
            // A literal `+` has to be named, because `"ctrl++"` splits into
            // `["ctrl", "", ""]` and the last part is empty. This was not a guess:
            // `every_declared_chord_parses` failed on `ctrl++` the first time it
            // ran, which is the whole reason that test exists.
            "plus" => Key::Char('+'),
            "." => Key::Char('.'),
            ">" => Key::Char('>'),
            "," => Key::Char(','),
            _ => return None,
        };
        Some(Self { key, mods })
    }

    /// The spelling a hint should print — `"ctrl+p"`, `"pgup"`, `"esc"`.
    ///
    /// **Printed from the chord, never typed by hand**, for the reason the terminal's own
    /// `key_label` gives: a footer that hardcodes `ctrl-p` names a key it only *hopes* is
    /// bound, and a rebound key must change its own hint.
    pub fn to_label(self) -> String {
        let mut out = String::new();
        if self.mods.ctrl {
            out.push_str("ctrl+");
        }
        if self.mods.alt {
            out.push_str("alt+");
        }
        if self.mods.shift {
            out.push_str("shift+");
        }
        if self.mods.command {
            out.push_str("super+");
        }
        out.push_str(&match self.key {
            Key::Char(' ') => "space".to_string(),
            // `+` is named, for the same reason it is parsed by name: `"ctrl++"`
            // does not split back into a chord. Caught by the round-trip test, which
            // is the test that exists to catch exactly this.
            Key::Char('+') => "plus".to_string(),
            Key::Char(c) => c.to_ascii_lowercase().to_string(),
            Key::Esc => "esc".to_string(),
            Key::Enter => "enter".to_string(),
            Key::Tab => "tab".to_string(),
            Key::Backspace => "backspace".to_string(),
            Key::Delete => "del".to_string(),
            Key::Insert => "ins".to_string(),
            Key::Home => "home".to_string(),
            Key::End => "end".to_string(),
            Key::PageUp => "pageup".to_string(),
            Key::PageDown => "pagedown".to_string(),
            Key::Up => "up".to_string(),
            Key::Down => "down".to_string(),
            Key::Left => "left".to_string(),
            Key::Right => "right".to_string(),
            Key::F(n) => format!("f{n}"),
            Key::Space => "space".to_string(),
        });
        out
    }
}

/// The terminal's default bindings, moved from `epsilon-tui` unchanged.
///
/// Ids are kept identical to pi's where they overlap, so a user migrating from pi can reuse the
/// same `keybindings.json`. The reasoning behind individual chords moved with them and is left
/// here rather than summarised: several of them record a chord that was tried and rejected
/// because it was already taken, and that is the part a future editor needs.
pub const TERMINAL: &[(&str, &[&str])] = &[
    ("app.interrupt", &["escape"]),
    // epsilon keeps ctrl+d for forward-delete (tui.editor.deleteCharForward), so
    // `app.exit` is not on ctrl+d as pi puts it.
    // `app.clear` owns ctrl+c and `app.exit` keeps ctrl+q, matching pi's separation
    // (`app.clear` = ctrl+c "Clear editor", `app.exit` = ctrl+d "Exit when editor is
    // empty"). Epsilon keeps ctrl+d for forward-delete, so exit is bound to ctrl+q
    // alone: two actions on one chord is not a remap, it is a coin toss decided by
    // handler order, and `/hotkeys` prints both. `app.clear` is matched first in
    // `app/keys.rs`, so ctrl+c empties the draft; ctrl+q is the way out.
    ("app.exit", &["ctrl+q"]),
    ("app.clear", &["ctrl+c"]),
    ("app.model.select", &["ctrl+m"]),
    ("app.palette", &["ctrl+p"]),
    ("app.thinking.cycle", &["ctrl+t"]),
    ("app.model.cycleForward", &["f2"]),
    ("app.model.cycleBackward", &["shift+f2"]),
    ("app.tools.expand", &["ctrl+x"]),
    // Transcript scrolling. These were the two hardcoded `KeyCode::PageUp/Down`
    // arms in the composer's key handler, which meant the footer could not name
    // them without hardcoding the same literal a second time — and a user who
    // wanted `ctrl+u`/`ctrl+d` had no way to ask. The handler matches these ids
    // now, so the key in the footer is the key that scrolls.
    ("app.transcript.pageUp", &["pageup"]),
    ("app.transcript.pageDown", &["pagedown"]),
    // Jump to the previous/next PROMPT rather than the previous/next ten rows.
    // pi's chords, read from `tui/src/keybindings.ts:184-192`:
    // `ctrl+up` / `ctrl+shift+up` and `ctrl+down` / `ctrl+shift+down`. The
    // one-modifier form is listed first because a terminal often cannot
    // distinguish the two, and a binding that only fires when a terminal
    // supports shift is a binding most users never discover.
    ("app.transcript.prevPrompt", &["ctrl+up", "ctrl+shift+up"]),
    (
        "app.transcript.nextPrompt",
        &["ctrl+down", "ctrl+shift+down"],
    ),
    // Jump to live: the other end of the same journey. `G` is vim's
    // end-of-buffer, but the reader pressing it here means "the newest rows",
    // and that is what "live" names — the view following the run again. Two
    // chords, primary first, because `g` is the one the footer can afford to
    // print and `G` is the one a vim user's hand reaches for; `matches` compares
    // characters case-insensitively, so the binding covers both.
    //
    // It fires only while the view is frozen. A bare letter that stopped typing
    // its own character would take `g` out of every draft, which is the M13
    // lesson (a bare `x` for expand made the letter untypable).
    ("app.transcript.live", &["G", "g"]),
    // Half-page and the two ends.
    //
    // pi gives half-page NO default (`tui/src/keybindings.ts:168-170` is
    // `defaultKeys: []`) and binds top/bottom to `home`/`end`.
    //
    // **Ctrl-modified, and on chords that are actually free.** Two attempts:
    //
    // 1. bare `u`/`d`/`H`/`L` -- correct about the keys, completely wrong about
    //    the editor. A bare letter is TEXT, and the composer is where a user is
    //    typing most of the time. Four unrelated tests failed.
    // 2. `ctrl+u`/`ctrl+d`, which read like readline's page keys -- and are
    //    **already bound to kill-to-start and delete-forward in the composer**
    //    (`keys.rs:1509`, `:1354`). A test caught that too, by asserting the
    //    scroll saturated at zero and finding it unmoved.
    //
    // 3. `ctrl+k`/`ctrl+j` -- free in the binding TABLE, and `ctrl+j` is
    //    **newline in the composer** (`tests.rs:1395`), so the table and the
    //    handler disagreed and the test caught it.
    //
    // **So the half page gets NO default key, which is exactly what pi does**:
    // `tui.altScreen.halfPageUp` is `defaultKeys: []`
    // (`tui/src/keybindings.ts:168-170`). Three chords were tried and each is
    // already the composer's: bare letters are text, `ctrl+u`/`ctrl+d` are
    // kill-line, `ctrl+j` is newline. A user who wants one sets it in
    // `keybindings.json`; a default that steals a key is worse than a default
    // that does not exist, and the whole point of the ids is that the footer can
    // name whatever the user chose.
    //
    // `top`/`bottom` do get defaults. `home`/`end` were rejected because they
    // already drive the composer cursor -- the same key meaning two things
    // depending on focus is worse than not having the binding -- so they take
    // `ctrl+home`/`ctrl+end`, which are free.
    //
    // A half page is the MEASURED viewport halved, not a fixed ten rows, which is
    // what makes it a half PAGE. The pageUp/pageDown bindings above stay a fixed
    // ten because they are also the footer hint.
    // **No half-page defaults, and that is the invariant rather than an
    // oversight.** Every id in this table must have a printable chord, because the
    // footer prints chords and a binding it cannot name is a binding a user
    // cannot find (`a_hint_label_parses_back_to_the_chord_it_names`). An empty
    // default fails that test -- correctly.
    //
    // Three chords were tried and each is the composer's: bare letters are TEXT,
    // `ctrl+u`/`ctrl+d` are kill-to-start and delete-forward, `ctrl+j` is newline.
    // pi leaves the same key unbound for the same reason
    // (`tui/src/keybindings.ts:168-170` is `defaultKeys: []`). The HANDLER still
    // exists, so a user who sets `app.transcript.halfUp` in `keybindings.json`
    // gets a half page; what is missing is only a default, which is the one thing
    // that cannot be had here.
    ("app.transcript.top", &["ctrl+home", "ctrl+shift+home"]),
    ("app.transcript.bottom", &["ctrl+end", "ctrl+shift+end"]),
    // Copy the last assistant message. pi binds this to `ctrl+x`
    // (`core/keybindings.ts:130-133`) -- but `ctrl+x` is `app.tools.expand` here,
    // and the two are not the same gesture: one shows more of a tool result, the
    // other takes text to the clipboard.
    //
    // **The chord was chosen by reading the composer's claims, not by taste.**
    // Every one of these is already bound in the key handler: ctrl+a home,
    // ctrl+b back-char, ctrl+d delete-forward, ctrl+e line-end, ctrl+f
    // move-forward, ctrl+k kill-to-line-end, ctrl+o (oauth), ctrl+u kill-to-start,
    // ctrl+v paste, ctrl+w kill-word, ctrl+y yank. ctrl+y was the obvious pick
    // and it is taken.
    //
    // What is left is the F-keys, and `grep -c 'KeyCode::F('` over the handler is
    // **0** -- nothing in the TUI claims one. `ctrl+y` was also tried first and
    // caught, the same way `ctrl+u`/`ctrl+j`/`ctrl+k` were for the half page.
    ("app.message.copy", &["f4"]),
    ("app.live.refresh", &["ctrl+g"]),
    ("tui.editor.undo", &["ctrl+z"]),
    ("tui.editor.redo", &["ctrl+."]),
    ("tui.input.newLine", &["ctrl+j"]),
    ("tui.input.submit", &["enter"]),
    ("app.clipboard.paste", &["ctrl+v", "shift+insert"]),
    // M59: pi's `app.editor.external` (pi defaults to ctrl+g, which this TUI
    // already uses for `app.live.refresh`) -- two defaults so either habit works.
    ("app.editor.external", &["ctrl+o", "alt+e"]),
];

/// The desktop window's default bindings.
///
/// **Every one of these was a literal `consume_key` read before M315.** Ids are per **action**,
/// not per key, because `Escape` means four different things in a window depending on which
/// surface holds the keyboard — and it is the *drawing order* that decides, not the table. That
/// is why a chord appears against more than one id here, and why the "no chord is bound to two
/// ids" rule is enforced for [`TERMINAL`] and not for this table: a shared chord in a terminal is
/// a coin toss decided by handler order, while a shared chord in a window is a priority the
/// drawing already states.
///
/// Note what is **not** shared: `ctrl+b` folds this window's sidebar and moves a text cursor
/// in the terminal's composer. That is not a contradiction of the table — they are two tables —
/// and an earlier comment in `app.rs` claiming the terminal bound `ctrl+b` to page-up was simply
/// wrong.
pub const WINDOW: &[(&str, &[&str])] = &[
    // --- the window's own chords, with no terminal counterpart ---
    ("window.sidebar.toggle", &["ctrl+b"]),
    ("window.sidebar.wider", &["right"]),
    ("window.sidebar.narrower", &["left"]),
    ("window.palette", &["ctrl+p"]),
    ("window.palette.close", &["escape"]),
    ("window.palette.submit", &["enter"]),
    ("window.palette.next", &["down"]),
    ("window.palette.previous", &["up"]),
    ("window.focus.next", &["f6"]),
    ("window.focus.previous", &["shift+f6"]),
    ("window.tree.next", &["down"]),
    ("window.tree.previous", &["up"]),
    ("window.tree.open", &["enter"]),
    ("window.pane.leave", &["escape"]),
    // **Two spellings, because a keyboard sends two keys.** `Ctrl`+`=` and
    // `Ctrl`+`Shift`+`=` reach the window as different keys (`Key::Equals` and
    // `Key::Plus`), and both mean "zoom in"; `"ctrl++"` is unspellable, so the
    // shifted form is named `plus`.
    ("window.zoom.in", &["ctrl+plus", "ctrl+="]),
    ("window.zoom.out", &["ctrl+-"]),
    ("window.zoom.reset", &["ctrl+0"]),
    // --- actions that exist in the terminal too, but under this window's own ids,
    //     because the surface that answers them is different ---
    ("window.composer.submit", &["enter"]),
    ("window.composer.escape", &["escape"]),
    ("window.approval.allow", &["enter"]),
    ("window.approval.deny", &["escape"]),
    // Session verbs with a keyboard route (S-103, S-107): the composer spells
    // them `/new` and `/fork`; these chords do the same through the same
    // dispatch. Rebindable like every other row in this table.
    ("window.session.new", &["ctrl+n"]),
    ("window.session.fork", &["ctrl+alt+f"]),
];

/// The default chords for `id` in `table`, or an empty slice.
pub fn chords<'a>(table: &'a [(&'a str, &'a [&'a str])], id: &str) -> &'a [&'a str] {
    table
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, chords)| *chords)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every chord in **both** tables parses. A table entry that does not parse is a
    /// binding that silently does nothing, which is the exact failure `AGENTS.md` §2.7
    /// names — and it is invisible until somebody presses the key.
    #[test]
    fn every_declared_chord_parses() {
        for (name, table) in [("TERMINAL", TERMINAL), ("WINDOW", WINDOW)] {
            for (id, cs) in table {
                for c in *cs {
                    assert!(
                        Chord::parse(c).is_some(),
                        "{name}:{id} declares {c:?}, which does not parse"
                    );
                }
            }
        }
    }

    /// **The round trip that keeps a hint honest.** A chord printed by
    /// [`Chord::to_label`] must parse back to itself, because the footer prints one and the
    /// handler matches the other. A label that did not round-trip would be a footer
    /// describing a key the product does not read.
    #[test]
    fn a_label_round_trips_to_the_chord_it_came_from() {
        for (name, table) in [("TERMINAL", TERMINAL), ("WINDOW", WINDOW)] {
            for (id, cs) in table {
                for c in *cs {
                    let parsed = Chord::parse(c).expect("declared chords parse");
                    let label = parsed.to_label();
                    assert_eq!(
                        Chord::parse(&label),
                        Some(parsed),
                        "{name}:{id} prints as {label:?}, which does not parse back"
                    );
                }
            }
        }
    }

    /// `ctrl+b` is the window's **and** the terminal's readline `back-char`, and the
    /// windows' comment about it was wrong. This records the truth so the next reader does
    /// not repeat the claim.
    #[test]
    fn ctrl_b_is_the_windows_sidebar_fold_and_nothing_else() {
        assert!(chords(WINDOW, "window.sidebar.toggle").contains(&"ctrl+b"));
        assert!(
            !TERMINAL.iter().any(|(_, cs)| cs.contains(&"ctrl+b")),
            "the terminal binds ctrl+b in its composer, but not in the table — a table entry \
             here would be a lie, because no handler reads it"
        );
    }

    /// Two ids in the **terminal** table sharing one chord is a coin toss decided by
    /// handler order. The window's table is exempt on purpose: a chord there is shared by
    /// *actions in different contexts*, and which one answers is decided by the drawing
    /// order, which is stated in code rather than by luck.
    #[test]
    fn no_chord_is_bound_to_two_ids_in_the_terminal_table() {
        let mut seen: Vec<(&str, &str)> = Vec::new();
        let mut clashes = Vec::new();
        for (id, cs) in TERMINAL {
            for c in *cs {
                match seen.iter().find(|(chord, _)| *chord == *c) {
                    Some((_, other)) => clashes.push(format!("TERMINAL: {c}: {other} and {id}")),
                    None => seen.push((*c, *id)),
                }
            }
        }
        assert!(
            clashes.is_empty(),
            "each chord must name one action: {}",
            clashes.join("; ")
        );
    }
}
