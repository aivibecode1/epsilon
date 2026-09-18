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
}
