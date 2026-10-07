//! The window's keyboard: one table, read from [`epsilon_contracts::keybind::WINDOW`].
//!
//! # What this replaces
//!
//! Seventeen literal `consume_key` reads in `app.rs` and one hand-written `"ctrl+b"` constant
//! used in four hint strings. `AGENTS.md` §2.4 requires keybindings to be "declared once and read
//! by every front end, with per-front-end capability declared as data"; the window was the half
//! that did not, so **its shortcuts could not be rebound at all** while the terminal's could.
//!
//! # Why ids are per *action* and not per *key*
//!
//! `Escape` means four different things in this window depending on what has the keyboard:
//! it closes a palette, leaves a focused pane, clears the draft, and refuses a permission ask.
//! So there is no single `app.interrupt` here — there are four ids, and **which one answers is
//! decided by the code**, in the order the surfaces are drawn. That is the difference between a
//! terminal and a window: a terminal has one composer, and a window stacks surfaces.
//!
//! It also means a chord can appear against more than one id, which is why the "no chord is bound
//! to two ids" rule is enforced for the terminal's table and not for this one. A shared chord in
//! a terminal is a coin toss decided by handler order; a shared chord in a window is a
//! **priority** the drawing order already states.

use std::collections::HashMap;
use std::path::Path;

use epsilon_contracts::keybind::{Chord, Mods, WINDOW};

/// The window's resolved bindings: the user's file if it has one, the shared table otherwise.
#[derive(Debug, Clone, Default)]
pub struct KeyBindings {
    user: HashMap<String, Vec<String>>,
    /// Why the user file was ignored, if it was there to be read. `None`
    /// means absent (normal first run) or fine — only a present-but-unusable
    /// file names itself, because only that surprises: every rebind in it is
    /// silently not applied.
    load_warning: Option<String>,
}

impl KeyBindings {
    /// The shared defaults, with no user file read.
    pub fn defaults() -> Self {
        Self {
            user: HashMap::new(),
            load_warning: None,
        }
    }

    /// Defaults overlaid with `<config dir>/keybindings.json`, exactly as the terminal reads it.
    ///
    /// **The same file, the same shape, the same grammar.** A user who writes
    /// `{"window.sidebar.toggle": "ctrl+alt+b"}` means it here and means it in the terminal,
    /// because `Chord::parse` is one function in `epsilon-contracts`. Reads fail open: a
    /// missing or unreadable file leaves the defaults, because a preferences file must never
    /// stop a window opening — but a present file that cannot be used says so
    /// through [`Self::load_warning`], because silent defaults are how a
    /// carefully written rebind goes missing without a trace.
    pub fn load(config_dir: &Path) -> Self {
        let (user, load_warning) = load_user_file(config_dir);
        Self { user, load_warning }
    }

    /// Why [`Self::load`] ignored the user file, if it did.
    pub fn load_warning(&self) -> Option<&str> {
        self.load_warning.as_deref()
    }

    /// How many binding ids the user file overrides. The settings browser
    /// shows the count rather than the table.
    pub fn user_count(&self) -> usize {
        self.user.len()
    }

    /// Test seam: inject a user map directly.
    pub fn from_map(user: HashMap<String, Vec<String>>) -> Self {
        Self {
            user,
            load_warning: None,
        }
    }

    /// The chords bound to `id`, parsed. An unknown id yields nothing, and a chord that does
    /// not parse is dropped rather than guessed at — a binding the product cannot read is
    /// not a binding it should pretend to have.
    pub fn chords(&self, id: &str) -> Vec<Chord> {
        self.raw(id)
            .iter()
            .filter_map(|c| Chord::parse(c))
            .collect()
    }

    /// The chord to **print** for `id`, or `None` when there is none.
    ///
    /// `None` is the honest answer and the caller drops the hint: a footer that hardcodes
    /// `ctrl-b` names a key it only *hopes* is bound, and a rebound key must change its own
    /// hint. That is the rule the terminal's `key_text` states, and it is why this lives beside
    /// the table rather than in a string at each call site.
    pub fn label(&self, id: &str) -> Option<String> {
        self.chords(id).first().map(|c| c.to_label())
    }

    fn raw(&self, id: &str) -> Vec<String> {
        if let Some(v) = self.user.get(id) {
            let parsed: Vec<String> = v
                .iter()
                .filter(|c| Chord::parse(c).is_some())
                .cloned()
                .collect();
            if !parsed.is_empty() {
                return parsed;
            }
        }
        epsilon_contracts::keybind::chords(WINDOW, id)
            .iter()
            .map(|c| c.to_string())
            .collect()
    }

    /// Consume `id`'s chord from the event stream if the stream carries one, and say so.
    ///
    /// **The replacement for `consume_key`, and it matches *exactly*.** egui's own
    /// `consume_key` matches modifiers *logically* — extra Shift and Alt are ignored — which
    /// is how `Shift+Enter` came to be swallowed by a plain `Enter` read in the composer
    /// (M314). A binding is a choice by a person, so it means one specific set of modifiers.
    ///
    /// The event is **removed from the stream** when it matches, which is what `consume_key`
    /// does and what the drawing order below depends on: the composer takes `Enter` before the
    /// `TextEdit` is drawn so that Enter is a send rather than a newline.
    pub fn take(&self, ctx: &egui::Context, id: &str) -> bool {
        let chords = self.chords(id);
        if chords.is_empty() {
            return false;
        }
        ctx.input_mut(|i| {
            let before = i.events.len();
            i.events.retain(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => !chords.iter().any(|c| {
                    c.key == neutral_key(*key) && c.mods.is_exactly(neutral_mods(*modifiers))
                }),
                _ => true,
            });
            before != i.events.len()
        })
    }
}

/// Read `<config dir>/keybindings.json` into a map, plus why it was ignored.
///
/// Fails open either way — a preferences file must never stop a window
/// opening — but only an *absent* file stays silent. A present file that
/// cannot be used names itself with the reason and the fix, because every
/// rebind in it is silently not applied otherwise.
fn load_user_file(config_dir: &Path) -> (HashMap<String, Vec<String>>, Option<String>) {
    let path = config_dir.join("keybindings.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (HashMap::new(), None),
        Err(e) => {
            return (
                HashMap::new(),
                Some(format!(
                    "keybindings ignored: {}: {e} — defaults apply; reopen the window after fixing it",
                    path.display()
                )),
            );
        }
    };
    let value = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(v) => v,
        Err(e) => {
            return (
                HashMap::new(),
                Some(format!(
                    "keybindings ignored: {}: not JSON ({}); defaults apply until it parses — reopen the window after fixing it",
                    path.display(),
                    e
                )),
            );
        }
    };
    let Some(object) = value.as_object() else {
        return (
            HashMap::new(),
            Some(format!(
                "keybindings ignored: {}: not an object; defaults apply until it parses — reopen the window after fixing it",
                path.display()
            )),
        );
    };
    let mut out = HashMap::new();
    for (id, value) in object {
        if let Some(s) = value.as_str() {
            out.insert(id.clone(), vec![s.to_string()]);
        } else if let Some(array) = value.as_array() {
            let vs: Vec<String> = array
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
            if !vs.is_empty() {
                out.insert(id.clone(), vs);
            }
        }
    }
    (out, None)
}

/// egui's `Key` in the shared vocabulary.
///
/// **Letters and digits are absent on purpose, and the reason is egui's, not ours.** egui
/// delivers printable characters as `Event::Text`, not as a `Key`, so a Ctrl+letter chord is
/// not expressible here — which is why this window has no such binding. A `Key` that has no
/// neutral counterpart is given the neutral `Char` of the character egui prints for it, so a
/// mismatch is a comparison that fails rather than a panic.
pub fn neutral_key(key: egui::Key) -> epsilon_contracts::keybind::Key {
    use epsilon_contracts::keybind::Key as K;
    match key {
        egui::Key::ArrowDown => K::Down,
        egui::Key::ArrowUp => K::Up,
        egui::Key::ArrowLeft => K::Left,
        egui::Key::ArrowRight => K::Right,
        egui::Key::Escape => K::Esc,
        egui::Key::Tab => K::Tab,
        egui::Key::Backspace => K::Backspace,
        egui::Key::Enter => K::Enter,
        egui::Key::Space => K::Space,
        egui::Key::Insert => K::Insert,
        egui::Key::Delete => K::Delete,
        egui::Key::Home => K::Home,
        egui::Key::End => K::End,
        egui::Key::PageUp => K::PageUp,
        egui::Key::PageDown => K::PageDown,
        egui::Key::Plus => K::Char('+'),
        egui::Key::Equals => K::Char('='),
        egui::Key::Minus => K::Char('-'),
        // Punctuation the tables bind (`ctrl+/` for the shortcuts reference):
        // egui names these words (`Slash`), and the word is not a character the
        // grammar could name — so they are mapped explicitly rather than left
        // to the name recovery below, which would make them unboundable.
        egui::Key::Slash => K::Char('/'),
        egui::Key::Period => K::Char('.'),
        egui::Key::Comma => K::Char(','),
        egui::Key::Num0 => K::Char('0'),
        other => {
            // egui names letters `A`…`Z` and digits `Num0`…`Num9`, and prints them as the
            // variant name, so the neutral form is recovered from that name. **Only a
            // single character or a `NumN` is accepted**, and anything else becomes a
            // character no chord can name: mapping `Key::Colon` onto `'c'` would let a
            // `ctrl+c` binding fire on a colon, which is the kind of near-miss that is
            // invisible until somebody binds the chord.
            //
            // A first version of this function assumed egui had **no** letter keys and
            // returned a placeholder for all of them. Four palette tests failed, because
            // `Ctrl+P` — the one chord every IDE uses for a command palette — never
            // matched.
            let name = format!("{other:?}");
            // F-keys first: `F6` is two characters, so it must not be mistaken for a
            // letter, and `window.focus.next` is bound to it.
            if let Some(n) = name.strip_prefix('F').and_then(|d| d.parse::<u8>().ok()) {
                return K::F(n);
            }
            if name.chars().count() == 1 {
                return K::Char(name.to_lowercase().chars().next().unwrap_or('\u{0}'));
            }
            if let Some(d) = name
                .strip_prefix("Num")
                .and_then(|d| d.parse::<char>().ok())
            {
                return K::Char(d);
            }
            K::Char('\u{0}')
        }
    }
}

/// egui's `Modifiers` in the shared vocabulary.
pub fn neutral_mods(mods: egui::Modifiers) -> Mods {
    Mods {
        ctrl: mods.ctrl,
        shift: mods.shift,
        alt: mods.alt,
        command: mods.mac_cmd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A present-but-unusable keybindings file names itself (S-126).
    ///
    /// Absent stays silent (normal first run); broken JSON, a non-object, and
    /// an unreadable path warn naming the file — because every rebind in them
    /// is silently not applied otherwise. RED first: silence everywhere.
    #[test]
    fn a_broken_keybindings_file_names_itself() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let file = dir.join("keybindings.json");

        // Absent: silent, defaults.
        assert!(
            KeyBindings::load(dir).load_warning().is_none(),
            "no file, no warning"
        );

        // Broken JSON: warns naming the file.
        std::fs::write(&file, "{oops").unwrap();
        let warning = KeyBindings::load(dir)
            .load_warning()
            .expect("broken JSON must warn")
            .to_string();
        assert!(
            warning.contains("keybindings.json"),
            "the warning names the document: {warning:?}"
        );

        // Valid JSON, not an object: warns.
        std::fs::write(&file, "[1, 2]").unwrap();
        assert!(
            KeyBindings::load(dir).load_warning().is_some(),
            "a non-object must warn"
        );

        // Unreadable path (a directory where the file should be): warns.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(
            KeyBindings::load(dir).load_warning().is_some(),
            "an unreadable path must warn"
        );

        // Valid file: silent.
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, r#"{"window.palette": "ctrl+alt+p"}"#).unwrap();
        assert!(
            KeyBindings::load(dir).load_warning().is_none(),
            "a good file stays silent"
        );
    }

    #[test]
    fn every_window_id_resolves_to_a_printable_chord() {
        // A hint that cannot be printed is a binding a user cannot find — the same rule the
        // terminal enforces with `a_hint_label_parses_back_to_the_chord_it_names`.
        for (id, _) in WINDOW {
            let keys = KeyBindings::defaults();
            let label = keys
                .label(id)
                .unwrap_or_else(|| panic!("{id} has no printable chord"));
            assert!(
                Chord::parse(&label).is_some(),
                "{id} prints as {label:?}, which does not parse back"
            );
        }
    }

    /// Feed one key event into a fresh context and run an empty frame.
    ///
    /// **The `textures_delta` is cleared**, because `epaint` panics in `Drop` on an
    /// unapplied delta and this draws nothing. The in-crate `Harness` clears it for the same
    /// reason and says why.
    fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Context {
        let ctx = egui::Context::default();
        let mut full = ctx.run_ui(
            egui::RawInput {
                // **`focused: true`, and it is load-bearing.** `RawInput` defaults it to
                // false and egui drops keyboard events for an unfocused window — the same
                // trap the in-crate `Harness` fell into, and the reason these two tests
                // reported a chord that "did not act" when it had never been delivered.
                focused: true,
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |_| {},
        );
        full.textures_delta.clear();
        ctx
    }

    /// A rebound window key is the key that acts, and the old chord says so.
    ///
    /// **The capability that did not exist at all before this module**: the window's
    /// shortcuts were literals, so `keybindings.json` could not touch them.
    #[test]
    fn a_rebound_window_key_is_the_key_that_acts() {
        let mut user = HashMap::new();
        user.insert(
            "window.sidebar.toggle".to_string(),
            vec!["ctrl+alt+b".to_string()],
        );
        let keys = KeyBindings::from_map(user);
        assert_eq!(
            keys.label("window.sidebar.toggle").as_deref(),
            Some("ctrl+alt+b")
        );

        let ctx = press(egui::Key::B, egui::Modifiers::CTRL);
        assert!(
            !keys.take(&ctx, "window.sidebar.toggle"),
            "ctrl+b must not still act"
        );

        let ctx = press(egui::Key::B, egui::Modifiers::CTRL | egui::Modifiers::ALT);
        assert!(
            keys.take(&ctx, "window.sidebar.toggle"),
            "the rebound chord must act"
        );
    }

    /// `take` matches **exactly**, which is the whole point of it existing.
    ///
    /// egui's `consume_key` would match the `Shift+Enter` here, because it ignores extra
    /// modifiers. That is how the composer lost its `Shift+Enter` (M314); the fix there was a
    /// hand-written guard, and the fix here is the matcher.
    #[test]
    fn an_extra_modifier_does_not_satisfy_a_binding() {
        let keys = KeyBindings::defaults();
        let ctx = press(egui::Key::Enter, egui::Modifiers::SHIFT);
        assert!(
            !keys.take(&ctx, "window.composer.submit"),
            "shift+enter must not be read as a plain submit"
        );
    }

    /// An id nobody declared matches nothing — no panic, no silent default chord.
    #[test]
    fn an_unknown_id_matches_nothing_and_prints_nothing() {
        let keys = KeyBindings::defaults();
        let ctx = egui::Context::default();
        assert!(!keys.take(&ctx, "window.does.not.exist"));
        assert_eq!(keys.label("window.does.not.exist"), None);
    }
}
