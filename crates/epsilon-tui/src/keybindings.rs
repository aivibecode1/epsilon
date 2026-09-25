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

/// A parsed `KeyId` — one chord (modifiers + base key).
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeySpec {
    code: KeyCode,
    modifiers: KeyModifiers,
}

/// Built-in defaults (subset of pi's `KEYBINDINGS` that epsilon actually
/// handles). The id strings are kept identical to pi where they overlap so
/// a user migrating from pi can reuse the same file.
const DEFAULTS: &[(&str, &[&str])] = &[
    ("app.interrupt", &["escape"]),
    // epsilon keeps ctrl+d for forward-delete (tui.editor.deleteCharForward),
    // so app.exit defaults to ctrl+c / ctrl+q (pi uses ctrl+d there).
    ("app.exit", &["ctrl+c", "ctrl+q"]),
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
    ("app.live.refresh", &["ctrl+g"]),
    ("tui.editor.undo", &["ctrl+z"]),
    ("tui.editor.redo", &["ctrl+."]),
    ("tui.input.newLine", &["ctrl+j"]),
    ("tui.input.submit", &["enter"]),
    ("app.clipboard.paste", &["ctrl+v", "shift+insert"]),
    // M59: pi's `app.editor.external` (pi defaults to ctrl+g, which this TUI
    // already uses for `app.live.refresh`) — two defaults so either habit works.
    ("app.editor.external", &["ctrl+o", "alt+e"]),
];

fn parse_key_id(raw: &str) -> Option<KeySpec> {
    let raw = raw.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }
    let parts: Vec<&str> = raw.split('+').map(str::trim).collect();
    let (mods_parts, key_part) = parts.split_at(parts.len().saturating_sub(1));
    let key_str = *key_part.first()?;
    let mut mods = KeyModifiers::empty();
    for m in mods_parts {
        match *m {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "shift" => mods |= KeyModifiers::SHIFT,
            "alt" | "meta" => mods |= KeyModifiers::ALT,
            "super" | "win" | "cmd" => mods |= KeyModifiers::SUPER,
            "" => {}
            _ => return None,
        }
    }
    let code = match key_str {
        "escape" | "esc" => KeyCode::Esc,
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        s if s.len() == 1 => {
            let ch = s.chars().next().unwrap();
            KeyCode::Char(ch)
        }
        "." => KeyCode::Char('.'),
        ">" => KeyCode::Char('>'),
        "," => KeyCode::Char(','),
        _ => return None,
    };
    Some(KeySpec {
        code,
        modifiers: mods,
    })
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
}

impl KeybindingsManager {
    /// Load from `dir/keybindings.json` if present (fail-open).
    pub fn load(dir: &Path) -> HashMap<String, Vec<String>> {
        let path = dir.join("keybindings.json");
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return HashMap::new(),
        };
        let v: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => return HashMap::new(),
        };
        let obj = match v.as_object() {
            Some(o) => o,
            None => return HashMap::new(),
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
        out
    }

    /// Create against the global config dir (`~/.config/epsilon`).
    pub fn create() -> Self {
        let dir = epsilon_core::config::global_dir();
        let (user, path) = match dir {
            Some(d) => {
                let u = Self::load(&d);
                let p = Some(d.join("keybindings.json"));
                (u, p)
            }
            None => (HashMap::new(), None),
        };
        Self {
            user,
            config_path: path,
        }
    }

    /// Test helper — inject a map directly.
    pub fn from_map(map: HashMap<String, Vec<String>>) -> Self {
        Self {
            user: map,
            config_path: None,
        }
    }

    /// Re-read the file (pi `KeybindingsManager.reload()`).
    pub fn reload(&mut self) {
        if let Some(p) = &self.config_path.clone()
            && let Some(dir) = p.parent()
        {
            self.user = Self::load(dir);
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
        // app.exit now defaults to ctrl+c / ctrl+q (ctrl+d kept as forward-delete).
        assert!(m.matches("app.exit", &ev(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(m.matches("app.exit", &ev(KeyCode::Char('q'), KeyModifiers::CONTROL)));
        assert!(!m.matches("app.exit", &ev(KeyCode::Char('d'), KeyModifiers::empty())));
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
