//! Keybinding manager — minimal pi parity (G6).
//!
//! pi: `packages/coding-agent/src/core/keybindings.ts` wraps
//! `packages/tui/src/keybindings.ts` (`TuiKeybindingsManager`) and
//! loads `~/.config/pi/agent/keybindings.json` (via `getAgentDir()`).
//! We mirror the shape with `~/.config/epsilon/keybindings.json`,
//! JSON-serialized as `{ "<binding-id>": "<KeyId>" | "<KeyId>[] }`
//! where `KeyId` is like `"ctrl+p"` / `"escape"` / `"shift+f2"`.
//! Only bindings listed in [`DEFAULTS`] are honoured; unknown keys are
//! retained verbatim so a round-trip never drops user data.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A parsed `KeyId` — one chord (modifiers + base key), in crossterm's vocabulary.
///
/// **The grammar is shared; only this conversion is not.**
/// `epsilon_contracts::keybind::Chord::parse` is the one function that decides what
/// `"ctrl+shift+f2"` means, so a binding a user writes means the same in both front ends.
/// Converting to `KeyCode`/`KeyModifiers` here, rather than parsing the string a second
/// time, is what keeps that true.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeySpec {
    code: KeyCode,
    modifiers: KeyModifiers,
}

/// The built-in defaults, **moved to `epsilon-contracts` (M315)**.
///
/// This table used to live here. It is now `epsilon_contracts::keybind::TERMINAL` because
/// `AGENTS.md` §2.4 requires keybindings to be declared once and read by every front end, and
/// the desktop window had seventeen literal `consume_key` reads with no table at all. The
/// reasons individual chords were chosen moved with the entries, unchanged.
use epsilon_contracts::keybind::TERMINAL as DEFAULTS;

/// Turn a shared [`Chord`] into crossterm's vocabulary.
///
/// **This is a conversion, not a parser.** The decision about what `"ctrl+shift+f2"` means
/// was made once, in `epsilon_contracts::keybind`; a second grammar here would be a second
/// answer to the same question, and the two would eventually disagree about something.
fn parse_key_id(raw: &str) -> Option<KeySpec> {
    let chord = epsilon_contracts::keybind::Chord::parse(raw)?;
    let code = match chord.key {
        epsilon_contracts::keybind::Key::Char(c) => KeyCode::Char(c),
        epsilon_contracts::keybind::Key::Esc => KeyCode::Esc,
        epsilon_contracts::keybind::Key::Enter => KeyCode::Enter,
        epsilon_contracts::keybind::Key::Tab => KeyCode::Tab,
        epsilon_contracts::keybind::Key::Space => KeyCode::Char(' '),
        epsilon_contracts::keybind::Key::Backspace => KeyCode::Backspace,
        epsilon_contracts::keybind::Key::Delete => KeyCode::Delete,
        epsilon_contracts::keybind::Key::Insert => KeyCode::Insert,
        epsilon_contracts::keybind::Key::Home => KeyCode::Home,
        epsilon_contracts::keybind::Key::End => KeyCode::End,
        epsilon_contracts::keybind::Key::PageUp => KeyCode::PageUp,
        epsilon_contracts::keybind::Key::PageDown => KeyCode::PageDown,
        epsilon_contracts::keybind::Key::Up => KeyCode::Up,
        epsilon_contracts::keybind::Key::Down => KeyCode::Down,
        epsilon_contracts::keybind::Key::Left => KeyCode::Left,
        epsilon_contracts::keybind::Key::Right => KeyCode::Right,
        epsilon_contracts::keybind::Key::F(n) => KeyCode::F(n),
    };
    let mut modifiers = KeyModifiers::empty();
    if chord.mods.ctrl {
        modifiers |= KeyModifiers::CONTROL;
    }
    if chord.mods.shift {
        modifiers |= KeyModifiers::SHIFT;
    }
    if chord.mods.alt {
        modifiers |= KeyModifiers::ALT;
    }
    if chord.mods.command {
        modifiers |= KeyModifiers::SUPER;
    }
    Some(KeySpec { code, modifiers })
}

fn specs_for(id: &str, user: &HashMap<String, Vec<String>>) -> Vec<KeySpec> {
    if let Some(v) = user.get(id) {
        let parsed: Vec<KeySpec> = v.iter().filter_map(|s| parse_key_id(s)).collect();
        if !parsed.is_empty() {
            return parsed;
        }
    }
    // fallback to defaults
    if let Some((_, def)) = DEFAULTS.iter().find(|(k, _)| *k == id) {
        def.iter().filter_map(|s| parse_key_id(s)).collect()
    } else {
        Vec::new()
    }
}

fn key_matches(spec: &KeySpec, ev: &KeyEvent) -> bool {
    if ev.modifiers != spec.modifiers {
        return false;
    }
    match (&spec.code, &ev.code) {
        (KeyCode::Char(a), KeyCode::Char(b)) => a.eq_ignore_ascii_case(b),
        (a, b) => a == b,
    }
}

/// A chord as a hint spells it. The inverse of [`parse_key_id`], and tested
/// against it rather than against a list of expected strings: a label that does
/// not parse back to the chord it came from is a label that names a dead key.
///
/// That test is why the spellings here look the way they do — `pageup`, not the
/// narrower `pgup` a footer would prefer, because `parse_key_id` accepts the
/// former and a label nothing can parse is exactly the defect this file is
/// fixing. Modifier order follows the parser (`ctrl+shift+x`).
fn key_label(spec: &KeySpec) -> String {
    let mut out = String::new();
    if spec.modifiers.contains(KeyModifiers::CONTROL) {
        out.push_str("ctrl+");
    }
    if spec.modifiers.contains(KeyModifiers::ALT) {
        out.push_str("alt+");
    }
    if spec.modifiers.contains(KeyModifiers::SHIFT) {
        out.push_str("shift+");
    }
    if spec.modifiers.contains(KeyModifiers::SUPER) {
        out.push_str("super+");
    }
    out.push_str(&match spec.code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) => c.to_ascii_lowercase().to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete => "del".to_string(),
        KeyCode::Insert => "ins".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::F(n) => format!("f{n}"),
        other => format!("{other:?}").to_ascii_lowercase(),
    });
    out
}

/// Minimal manager — loads once, `reload()` re-reads the file.
#[derive(Debug, Clone)]
pub struct KeybindingsManager {
    user: HashMap<String, Vec<String>>,
    config_path: Option<PathBuf>,
    /// Why the user file was ignored, if it was there to be read. `None`
    /// means absent (normal first run) or fine — only a present-but-unusable
    /// file names itself, because only that surprises.
    load_warning: Option<String>,
}

impl KeybindingsManager {
    /// Load from `dir/keybindings.json` if present (fail-open).
    pub fn load(dir: &Path) -> HashMap<String, Vec<String>> {
        Self::load_report(dir).0
    }

    /// [`Self::load`] plus why the file was ignored, if it was there to be
    /// read. Absent stays silent; broken JSON, non-objects and unreadable
    /// paths name the document with the reason and the fix.
    pub fn load_report(dir: &Path) -> (HashMap<String, Vec<String>>, Option<String>) {
        let path = dir.join("keybindings.json");
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (HashMap::new(), None),
            Err(e) => {
                return (
                    HashMap::new(),
                    Some(format!(
                        "keybindings ignored: {}: {e} — defaults apply; restart after fixing it",
                        path.display()
                    )),
                );
            }
        };
        let v: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                return (
                    HashMap::new(),
                    Some(format!(
                        "keybindings ignored: {}: not JSON ({}); defaults apply until it parses — restart after fixing it",
                        path.display(),
                        e
                    )),
                );
            }
        };
        let obj = match v.as_object() {
            Some(o) => o,
            None => {
                return (
                    HashMap::new(),
                    Some(format!(
                        "keybindings ignored: {}: not an object; defaults apply until it parses — restart after fixing it",
                        path.display()
                    )),
                );
            }
        };
        let mut out = HashMap::new();
        for (k, val) in obj {
            if let Some(s) = val.as_str() {
                out.insert(k.clone(), vec![s.to_string()]);
            } else if let Some(arr) = val.as_array() {
                let vs: Vec<String> = arr
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect();
                if !vs.is_empty() {
                    out.insert(k.clone(), vs);
                }
            }
        }
        (out, None)
    }

    /// Why [`Self::create`] ignored the user file, if it did.
    pub fn load_warning(&self) -> Option<&str> {
        self.load_warning.as_deref()
    }

    /// Create against the global config dir (`~/.config/epsilon`).
    pub fn create() -> Self {
        let dir = epsilon_core::config::global_dir();
        let (user, path, load_warning) = match dir {
            Some(d) => {
                let (u, w) = Self::load_report(&d);
                let p = Some(d.join("keybindings.json"));
                (u, p, w)
            }
            None => (HashMap::new(), None, None),
        };
        Self {
            user,
            config_path: path,
            load_warning,
        }
    }

    /// Test helper — inject a map directly.
    pub fn from_map(map: HashMap<String, Vec<String>>) -> Self {
        Self {
            user: map,
            config_path: None,
            load_warning: None,
        }
    }

    /// Re-read the file (pi `KeybindingsManager.reload()`).
    pub fn reload(&mut self) {
        if let Some(p) = &self.config_path.clone()
            && let Some(dir) = p.parent()
        {
            let (user, warning) = Self::load_report(dir);
            self.user = user;
            self.load_warning = warning;
        }
    }

    /// True when `event` matches any chord bound to `id`.
    pub fn matches(&self, id: &str, event: &KeyEvent) -> bool {
        let specs = specs_for(id, &self.user);
        specs.iter().any(|s| key_matches(s, event))
    }

    /// The chord to **print for** a binding, as the footer spells it — `ctrl+p`,
    /// `pgup`, `esc`.
    ///
    /// This is the way a hint stays true. A footer that hardcodes `ctrl-p` names
    /// a key it only *hopes* is bound: rebind `app.palette` and the hint keeps
    /// advertising the old chord, or worse, a chord nobody handles. Asking the
    /// table means a rebound key changes its own hint, and a binding that does
    /// not resolve produces no hint at all rather than a dead one.
    ///
    /// `None` when the id is unknown or every chord for it fails to parse — the
    /// caller drops the hint, which is the only honest thing left to do with it.
    ///
    /// The first chord is the one printed. A binding may list alternatives
    /// (`app.editor.external` is `ctrl+o` / `alt+e`); printing both would cost
    /// four columns on a one-row bar to describe a choice the reader can make
    /// later, and pi prints the primary chord for the same reason.
    pub fn key_text(&self, id: &str) -> Option<String> {
        specs_for(id, &self.user).first().map(key_label)
    }

    /// Whether a string is exactly the text [`Self::key_text`] prints for some
    /// binding — the check that a footer hint came from this table.
    ///
    /// Public because it is the assertion a hint needs and the one a test cannot
    /// make from the outside: "this key resolves to a binding" is a claim about
    /// the private user map, and a hint that fails it is a key that does nothing.
    pub fn key_text_round_trips(&self, text: &str) -> bool {
        let Some(spec) = parse_key_id(text) else {
            return false;
        };
        DEFAULTS
            .iter()
            .any(|(id, _)| specs_for(id, &self.user).contains(&spec))
    }

    /// Resolved (effective) bindings for diagnostics / `--help`.
    pub fn effective(&self) -> HashMap<String, Vec<String>> {
        let mut out = HashMap::new();
        for (id, _) in DEFAULTS {
            let specs = specs_for(id, &self.user);
            // serialize back to canonical lower-case form is just the stored strings;
            // fall back to defaults when user map empty.
            let strs: Vec<String> = if let Some(v) = self.user.get(*id) {
                v.clone()
            } else {
                // defaults
                DEFAULTS
                    .iter()
                    .find(|(k, _)| k == id)
                    .map(|(_, v)| v.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_default()
            };
            // only emit if we actually have specs (guards unknown ids)
            if !specs.is_empty() {
                out.insert((*id).to_string(), strs);
            }
        }
        // also surface any user-only ids (forward compat — pi keeps extras sorted)
        for (k, v) in &self.user {
            if !out.contains_key(k) {
                out.insert(k.clone(), v.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        }
    }

    #[test]
    fn defaults_match_without_file() {
        let m = KeybindingsManager::from_map(HashMap::new());
        // `app.exit` is ctrl+q alone (ctrl+d stays forward-delete, and pi puts exit
        // there). It used to be ctrl+c *and* ctrl+q — but ctrl+c is `app.clear`, and
        // two ids on one chord means the handler order decides which fires. `/hotkeys`
        // printed both, so the table advertised a binding table that could not be
        // honoured.
        assert!(m.matches("app.exit", &ev(KeyCode::Char('q'), KeyModifiers::CONTROL)));
        assert!(!m.matches("app.exit", &ev(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(!m.matches("app.exit", &ev(KeyCode::Char('d'), KeyModifiers::empty())));
        // And the chord ctrl+c now belongs to exactly one action — pi's default.
        assert!(m.matches("app.clear", &ev(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(m.matches("app.interrupt", &ev(KeyCode::Esc, KeyModifiers::empty())));
    }

    #[test]
    fn user_override_replaces_default() {
        let mut map = HashMap::new();
        map.insert("app.exit".into(), vec!["ctrl+q".into()]);
        let m = KeybindingsManager::from_map(map);
        assert!(m.matches("app.exit", &ev(KeyCode::Char('q'), KeyModifiers::CONTROL)));
        assert!(!m.matches("app.exit", &ev(KeyCode::Char('d'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn multi_chord_binding() {
        let m = KeybindingsManager::from_map(HashMap::new());
        assert!(m.matches(
            "app.clipboard.paste",
            &ev(KeyCode::Char('v'), KeyModifiers::CONTROL)
        ));
        assert!(m.matches(
            "app.clipboard.paste",
            &ev(KeyCode::Insert, KeyModifiers::SHIFT)
        ));
    }

    #[test]
    fn load_round_trips_json() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("keybindings.json");
        std::fs::write(&p, r#"{"app.exit":"ctrl+q","app.palette":["ctrl+p"]}"#).unwrap();
        let m = KeybindingsManager::load(tmp.path());
        assert_eq!(m.get("app.exit").unwrap(), &vec!["ctrl+q".to_string()]);
    }

    /// A present-but-unusable keybindings file names itself (S-126, terminal
    /// half): absent stays silent, broken JSON / non-object / unreadable
    /// paths warn naming the document. RED first: silence everywhere.
    #[test]
    fn a_broken_keybindings_file_names_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let file = dir.join("keybindings.json");

        // Absent: silent.
        assert!(
            KeybindingsManager::load_report(dir).1.is_none(),
            "no file, no warning"
        );

        // Broken JSON: warns naming the file.
        std::fs::write(&file, "{oops").unwrap();
        let warning = KeybindingsManager::load_report(dir)
            .1
            .expect("broken JSON must warn");
        assert!(
            warning.contains("keybindings.json"),
            "the warning names the document: {warning:?}"
        );

        // Valid JSON, not an object: warns.
        std::fs::write(&file, "[1, 2]").unwrap();
        assert!(
            KeybindingsManager::load_report(dir).1.is_some(),
            "a non-object must warn"
        );

        // Unreadable path: warns.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(
            KeybindingsManager::load_report(dir).1.is_some(),
            "an unreadable path must warn"
        );

        // Valid file: silent.
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, r#"{"app.exit": "ctrl+q"}"#).unwrap();
        assert!(
            KeybindingsManager::load_report(dir).1.is_none(),
            "a good file stays silent"
        );
    }

    #[test]
    fn a_hint_label_parses_back_to_the_chord_it_names() {
        // The property a footer depends on: every label it can print is a key id
        // that parses to the chord it was printed from. Checked against the
        // parser rather than against a list of strings, so a label that names a
        // key nobody can type cannot be added without this failing.
        let m = KeybindingsManager::from_map(HashMap::new());
        for (id, _) in DEFAULTS {
            let Some(label) = m.key_text(id) else {
                panic!("{id} has no printable chord despite being a default");
            };
            let spec = parse_key_id(&label)
                .unwrap_or_else(|| panic!("{id} printed {label:?}, which is not a key id"));
            assert_eq!(
                key_label(&spec),
                label,
                "{id}: {label:?} does not round-trip through the parser"
            );
        }
    }

    #[test]
    fn a_hint_names_the_users_chord_not_the_default_one() {
        // The defect this whole path exists to prevent: a footer that hardcodes
        // the chord it advertises keeps saying `ctrl+p` after the user binds
        // something else.
        let mut map = HashMap::new();
        map.insert("app.palette".into(), vec!["ctrl+k".into()]);
        let m = KeybindingsManager::from_map(map);
        assert_eq!(m.key_text("app.palette").as_deref(), Some("ctrl+k"));
        assert!(
            m.matches(
                "app.palette",
                &ev(KeyCode::Char('k'), KeyModifiers::CONTROL)
            ),
            "the chord the hint names must be the chord that fires"
        );
        assert!(
            !m.matches(
                "app.palette",
                &ev(KeyCode::Char('p'), KeyModifiers::CONTROL)
            ),
            "the old default must not still fire"
        );
    }

    #[test]
    fn an_unbindable_id_produces_no_hint() {
        // A promise with no key behind it must not be printable. An unknown id,
        // and a user overriding one to something that does not parse, are the
        // two ways a hint could otherwise name a dead key.
        let m = KeybindingsManager::from_map(HashMap::new());
        assert_eq!(m.key_text("app.no.such.binding"), None);
        assert_eq!(m.key_text("tui.input.submit"), Some("enter".to_string()));

        let mut map = HashMap::new();
        map.insert("tui.input.submit".into(), vec!["hyper+enter".into()]);
        let broken = KeybindingsManager::from_map(map);
        // The chord does not parse, so the binding falls back to its default
        // rather than to a key that cannot be pressed.
        assert_eq!(
            broken.key_text("tui.input.submit"),
            Some("enter".to_string())
        );
    }
}
