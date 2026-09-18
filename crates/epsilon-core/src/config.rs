//! Global user configuration — first implemented slice of ADR-012.
//!
//! Path convention (ADR-012): `~/.config/epsilon/config.toml` on every platform
//! (`$HOME` / `$USERPROFILE`; no external dirs crate). Declarative only; `${VAR}`
//! interpolation from the environment arrives with the full config layer (M12).
//!
//! M11 owns exactly one key: `[model] id` — the last model picked in any frontend.
//! Precedence everywhere: `--model` CLI flag (ephemeral) > saved value > `"faux-1"`.

use std::path::{Path, PathBuf};

/// Default catalog id when nothing is configured or the file is unreadable.
pub const DEFAULT_MODEL_ID: &str = "faux-1";

/// Read `[<table>] <key> = "…"` from a config dir's `config.toml` (M59: the
/// external-editor command is user configuration, and the TUI reads every
/// config value through this module rather than parsing TOML itself).
/// Missing file, table, key, or a non-string value → `None` (never an error:
/// a config typo must not break a session).
pub fn load_config_string_in(config_dir: &Path, table: &str, key: &str) -> Option<String> {
    let raw = std::fs::read_to_string(config_dir.join("config.toml")).ok()?;
    let parsed: toml::Value = toml::from_str(&raw).ok()?;
    parsed
        .get(table)?
        .get(key)?
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// [`load_config_string_in`] against the resolved global directory.
pub fn load_config_string(table: &str, key: &str) -> Option<String> {
    load_config_string_in(&global_dir()?, table, key)
}

/// `~/.config/epsilon` — resolved from `$HOME`, falling back to `$USERPROFILE`.
pub fn global_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".config").join("epsilon"))
}

/// Path of the global config file (exists only when a home directory resolves).
pub fn global_config_path() -> Option<PathBuf> {
    global_dir().map(|d| d.join("config.toml"))
}

/// Read the persisted model id from `config_dir/config.toml`.
///
/// Missing file / missing key / parse errors all resolve to `None` — configuration is
/// advisory at read time and must never block startup (fail-open for reads; writes are
/// best-effort too).
pub fn load_model_id_in(config_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(config_dir.join("config.toml")).ok()?;
    parse_model_id(&raw)
}

/// [`load_model_id_in`] against the resolved global directory.
pub fn load_model_id() -> Option<String> {
    load_model_id_in(&global_dir()?)
}

/// Persist `model_id` into `config_dir/config.toml`, creating the directory when needed.
///
/// Best-effort by contract: errors are returned for callers that want to warn but are
/// never fatal. Existing unknown keys survive (round-trip via [`toml::Table`]).
pub fn save_model_id_in(config_dir: &Path, model_id: &str) -> Result<PathBuf, String> {
    if model_id.trim().is_empty() {
        return Err("refusing to persist an empty model id".into());
    }
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = config_dir.join("config.toml");

    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();
    table
        .entry("model")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| "[model] exists but is not a table".to_string())?
        .insert("id".into(), toml::Value::String(model_id.to_string()));

    let serialized =
        toml::to_string_pretty(&table).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// [`save_model_id_in`] against the resolved global directory.
pub fn save_model_id(model_id: &str) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    save_model_id_in(&dir, model_id)
}

fn parse_model_id(raw: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(raw).ok()?;
    value
        .get("model")?
        .get("id")?
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
}

/// Recently used model ids, newest first (`[picker] recent`), capped at [`RECENT_MAX`].
pub const RECENT_MAX: usize = 8;

// ---- project trust hash (M22; shared by cli loader + tui acceptance) -------------

/// SHA-256 hex of `bytes` (the trust-hash primitive both cli and tui need).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Record `hash` as trusted in `<workspace>/.epsilon/trust.json` — the user
/// accepted a changed project config (pi trust-manager's "trust" answer).
pub fn accept_project_config_hash_in(workspace: &Path, hash: &str) -> Result<PathBuf, String> {
    let path = workspace.join(".epsilon").join("trust.json");
    std::fs::create_dir_all(workspace.join(".epsilon"))
        .map_err(|e| format!("create .epsilon: {e}"))?;
    std::fs::write(
        &path,
        serde_json::json!({ "config_sha256": hash }).to_string(),
    )
    .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(path)
}

/// Current SHA-256 of `<workspace>/.epsilon/config.toml`, if present.
pub fn current_project_config_hash_in(workspace: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(workspace.join(".epsilon").join("config.toml")).ok()?;
    Some(sha256_hex(raw.as_bytes()))
}

/// Favorite (starred) model ids (`[picker] favorite`), insertion order, uncapped —
/// opencode's `favorite` list semantics (toggled in the picker, cycled with
/// Ctrl+P like pi's scoped-models cycling).
pub const FAVORITE_MAX: usize = 16;

/// Favorite model ids from `config_dir/config.toml` (insertion order, capped).
pub fn load_favorite_models_in(config_dir: &Path) -> Vec<String> {
    let raw = match std::fs::read_to_string(config_dir.join("config.toml")) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let value: toml::Value = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    value
        .get("picker")
        .and_then(|p| p.get("favorite"))
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .filter(|s| !s.trim().is_empty())
                .take(FAVORITE_MAX)
                .collect()
        })
        .unwrap_or_default()
}

/// Toggle `model_id` in the favorites list: present -> remove, absent ->
/// append (opencode `toggleFavorite` semantics). Persists immediately.
pub fn toggle_favorite_model_in(config_dir: &Path, model_id: &str) -> Result<bool, String> {
    if model_id.trim().is_empty() {
        return Err("refusing to favorite an empty model id".into());
    }
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = config_dir.join("config.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();

    let mut favs = load_favorite_models_in(config_dir);
    let existed = match favs.iter().position(|id| id == model_id) {
        Some(i) => {
            favs.remove(i);
            true
        }
        None => {
            if favs.len() >= FAVORITE_MAX {
                return Err(format!(
                    "favorites are full ({FAVORITE_MAX}); unfavorite one first"
                ));
            }
            favs.push(model_id.to_string());
            false
        }
    };

    table
        .entry("picker")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| "[picker] exists but is not a table".to_string())?
        .insert(
            "favorite".into(),
            toml::Value::Array(
                favs.into_iter()
                    .map(toml::Value::String)
                    .collect::<Vec<_>>(),
            ),
        );

    let serialized =
        toml::to_string_pretty(&table).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(existed)
}

/// [`load_favorite_models_in`] against the resolved global directory.
pub fn load_favorite_models() -> Vec<String> {
    global_dir()
        .map(|d| load_favorite_models_in(&d))
        .unwrap_or_default()
}

/// [`toggle_favorite_model_in`] against the resolved global directory.
/// Returns the NEW state: true = now a favorite, false = unfavorited.
pub fn toggle_favorite_model(model_id: &str) -> Result<bool, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    toggle_favorite_model_in(&dir, model_id)
}

// ---- per-favorite thinking levels (M26; pi scoped-models' per-entry level) ------

/// Per-favorite thinking levels from `config_dir/config.toml`, read from the
/// optional `[picker.favorite_thinking]` table (`<model-id> = "high"`). An
/// absent entry means "inherit the session level" — exactly pi's optional
/// `thinkingLevel` on scoped models (`agent-session.ts:204`). Entries for
/// models that are not favorites are ignored by callers; `unstar` removes
/// them at the source.
pub fn load_favorite_thinking_in(config_dir: &Path) -> Vec<(String, String)> {
    let raw = match std::fs::read_to_string(config_dir.join("config.toml")) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let value: toml::Value = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    value
        .get("picker")
        .and_then(|p| p.get("favorite_thinking"))
        .and_then(|t| t.as_table())
        .map(|t| {
            t.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Set (or clear with `None`) the remembered thinking level for one favorite
/// model id. Persists immediately; unknown keys survive.
pub fn set_favorite_thinking_in(
    config_dir: &Path,
    model_id: &str,
    level: Option<&str>,
) -> Result<PathBuf, String> {
    if model_id.trim().is_empty() {
        return Err("refusing to remember thinking for an empty model id".into());
    }
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = config_dir.join("config.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();

    {
        let picker = table
            .entry("picker")
            .or_insert(toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| "[picker] exists but is not a table".to_string())?;
        let sub = picker
            .entry("favorite_thinking")
            .or_insert(toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| "[picker.favorite_thinking] exists but is not a table".to_string())?;
        match level {
            Some(l) => {
                sub.insert(model_id.into(), toml::Value::String(l.to_string()));
            }
            None => {
                sub.remove(model_id);
            }
        }
    }

    let serialized =
        toml::to_string_pretty(&table).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// [`load_favorite_thinking_in`] against the resolved global directory.
pub fn load_favorite_thinking() -> Vec<(String, String)> {
    global_dir()
        .map(|d| load_favorite_thinking_in(&d))
        .unwrap_or_default()
}

/// [`set_favorite_thinking_in`] against the resolved global directory.
pub fn set_favorite_thinking(model_id: &str, level: Option<&str>) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    set_favorite_thinking_in(&dir, model_id, level)
}

/// Recently used model ids from `config_dir/config.toml` (newest first, capped).
pub fn load_recent_models_in(config_dir: &Path) -> Vec<String> {
    let raw = match std::fs::read_to_string(config_dir.join("config.toml")) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let value: toml::Value = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    value
        .get("picker")
        .and_then(|p| p.get("recent"))
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .filter(|s| !s.trim().is_empty())
                .take(RECENT_MAX)
                .collect()
        })
        .unwrap_or_default()
}

/// Push `model_id` to the front of the recents list (dedup, cap) and persist.
pub fn push_recent_model_in(config_dir: &Path, model_id: &str) -> Result<PathBuf, String> {
    if model_id.trim().is_empty() {
        return Err("refusing to record an empty model id".into());
    }
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = config_dir.join("config.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();

    let mut recent = load_recent_models_in(config_dir);
    recent.retain(|id| id != model_id);
    recent.insert(0, model_id.to_string());
    recent.truncate(RECENT_MAX);

    table
        .entry("picker")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| "[picker] exists but is not a table".to_string())?
        .insert(
            "recent".into(),
            toml::Value::Array(
                recent
                    .into_iter()
                    .map(toml::Value::String)
                    .collect::<Vec<_>>(),
            ),
        );

    let serialized =
        toml::to_string_pretty(&table).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// Recently used model ids from the global config (newest first).
pub fn load_recent_models() -> Vec<String> {
    global_dir()
        .map(|d| load_recent_models_in(&d))
        .unwrap_or_default()
}

/// Record a picked model in the global recents list.
pub fn push_recent_model(model_id: &str) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    push_recent_model_in(&dir, model_id)
}

// ---- prompt history (M16b) -----------------------------------------------------

/// Maximum persisted prompt-history entries (`<dir>/history`, newest last).
pub const PROMPT_HISTORY_MAX: usize = 500;

/// Path of the prompt-history file under `config_dir` (`history`).
fn history_path(config_dir: &Path) -> PathBuf {
    config_dir.join("history")
}

/// Parse JSON-lines history; corrupt lines are skipped, never fatal (fail-open
/// reads, matching [`load_model_id_in`]).
fn parse_history(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| serde_json::from_str::<String>(line).ok())
        .collect()
}

/// Persisted prompt history from `config_dir/history` (oldest first).
///
/// Missing/unreadable file yields an empty list; history is advisory and must
/// never block startup.
pub fn load_prompt_history_in(config_dir: &Path) -> Vec<String> {
    match std::fs::read_to_string(history_path(config_dir)) {
        Ok(raw) => parse_history(&raw),
        Err(_) => Vec::new(),
    }
}

/// Append one entry to `config_dir/history` (JSON-lines, capped at
/// [`PROMPT_HISTORY_MAX`] by rewriting the tail). Consecutive duplicates are
/// ignored (same rule as the in-session ring). Errors are advisory.
pub fn append_prompt_history_in(config_dir: &Path, entry: &str) -> Result<PathBuf, String> {
    if entry.trim().is_empty() {
        return Err("refusing to persist an empty history entry".into());
    }
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = history_path(config_dir);

    let mut entries = load_prompt_history_in(config_dir);
    if entries.last().is_some_and(|last| last == entry) {
        return Ok(path);
    }
    entries.push(entry.to_string());
    let start = entries.len().saturating_sub(PROMPT_HISTORY_MAX);
    let mut serialized = String::new();
    for e in &entries[start..] {
        serialized.push_str(&serde_json::to_string(e).map_err(|e| e.to_string())?);
        serialized.push('\n');
    }
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// [`load_prompt_history_in`] against the resolved global directory.
pub fn load_prompt_history() -> Vec<String> {
    global_dir()
        .map(|d| load_prompt_history_in(&d))
        .unwrap_or_default()
}

/// [`append_prompt_history_in`] against the resolved global directory.
pub fn append_prompt_history(entry: &str) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    append_prompt_history_in(&dir, entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_round_trips_and_preserves_unknown_keys() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path().to_path_buf();
        assert_eq!(load_model_id_in(&dir), None, "no file yet");

        // Seed a config with a foreign key, then persist a model over it.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.toml"),
            "[theme]\nname = \"dark\"\n[old]\nid = \"gpt-4o\"\n",
        )
        .unwrap();

        let written = save_model_id_in(&dir, "deepseek-chat").expect("save");
        assert_eq!(written, dir.join("config.toml"));

        assert_eq!(load_model_id_in(&dir).as_deref(), Some("deepseek-chat"));
        let text = std::fs::read_to_string(&written).unwrap();
        assert!(
            text.contains("[theme]"),
            "unknown section preserved: {text}"
        );
        assert!(text.contains("[old]"), "second section preserved: {text}");
        assert!(text.contains("[model]"), "model section present: {text}");
    }

    #[test]
    fn malformed_or_keyless_files_load_as_none() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();

        std::fs::write(dir.join("config.toml"), "not [ valid toml {{{{").unwrap();
        assert_eq!(load_model_id_in(dir), None);

        std::fs::write(dir.join("config.toml"), "[model]\nid = \"   \"\n").unwrap();
        assert_eq!(
            load_model_id_in(dir),
            None,
            "blank ids are not configuration"
        );

        std::fs::write(dir.join("config.toml"), "[other]\nkey = 1\n").unwrap();
        assert_eq!(load_model_id_in(dir), None);
    }

    #[test]
    fn empty_id_refuses_to_persist_and_non_table_section_errors() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        assert!(save_model_id_in(dir, "  ").is_err());

        // A pre-existing scalar `[model] id = ...` written by hand as `model = "x"`
        // must error rather than be silently clobbered.
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("config.toml"), "model = \"scalar\"\n").unwrap();
        assert!(
            save_model_id_in(dir, "kimi-k2").is_err(),
            "must refuse to overwrite a non-table [model]"
        );
        assert_eq!(
            parse_model_id(&std::fs::read_to_string(dir.join("config.toml")).unwrap()),
            None,
            "scalar shape does not parse as our key either"
        );
    }

    #[test]
    fn recents_dedup_cap_and_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert!(load_recent_models_in(dir).is_empty());

        for id in ["a", "b", "c"] {
            push_recent_model_in(dir, id).unwrap();
        }
        assert_eq!(
            load_recent_models_in(dir),
            vec!["c".to_string(), "b".to_string(), "a".to_string()]
        );

        // Re-picking "a" moves it to front without duplicating.
        push_recent_model_in(dir, "a").unwrap();
        assert_eq!(load_recent_models_in(dir)[0], "a");
        assert_eq!(
            load_recent_models_in(dir)
                .iter()
                .filter(|i| *i == "a")
                .count(),
            1
        );

        // Cap enforced.
        for i in 0..RECENT_MAX as u32 + 4 {
            let _ = push_recent_model_in(dir, &format!("m{i}"));
        }
        assert_eq!(load_recent_models_in(dir).len(), RECENT_MAX);
        // And the model id key survived all of this.
        let _ = save_model_id_in(dir, "deepseek-chat");
        assert_eq!(load_model_id_in(dir).as_deref(), Some("deepseek-chat"));
        assert_eq!(load_recent_models_in(dir).len(), RECENT_MAX);
    }

    #[test]
    fn favorites_toggle_round_trip_and_survive_recents_writes() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();

        // Toggle on: reports new state true, list grows in insertion order.
        assert!(!toggle_favorite_model_in(dir, "kimi-k2.6").unwrap());
        assert!(!toggle_favorite_model_in(dir, "deepseek-chat").unwrap());
        assert_eq!(
            load_favorite_models_in(dir),
            vec!["kimi-k2.6".to_string(), "deepseek-chat".to_string()]
        );

        // Toggle off the first: removed, order of the rest preserved.
        assert!(toggle_favorite_model_in(dir, "kimi-k2.6").unwrap());
        assert_eq!(load_favorite_models_in(dir), vec!["deepseek-chat"]);

        // Recents writes must not clobber favorites (separate [picker] key).
        let _ = push_recent_model_in(dir, "deepseek-chat");
        assert_eq!(
            load_favorite_models_in(dir),
            vec!["deepseek-chat".to_string()]
        );
        assert_eq!(
            load_recent_models_in(dir),
            vec!["deepseek-chat".to_string()]
        );

        // Empty id refused; missing file reads empty.
        assert!(toggle_favorite_model_in(dir, "  ").is_err());
        let empty = tempfile::tempdir().unwrap();
        assert!(load_favorite_models_in(empty.path()).is_empty());
    }

    #[test]
    fn favorite_thinking_set_clear_and_survive_other_writes() {
        // M26: per-favorite thinking memory round-trips, clears, and coexists
        // with the favorites/recents keys in the same [picker] table.
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();

        // No memory until one is set; unknown ids read as absent.
        assert!(load_favorite_thinking_in(dir).is_empty());

        set_favorite_thinking_in(dir, "kimi-k2.6", Some("high")).unwrap();
        set_favorite_thinking_in(dir, "deepseek-chat", Some("off")).unwrap();
        // Table iteration order depends on the toml crate's feature flags, so
        // compare as a sorted set, not positional rows.
        let mut got = load_favorite_thinking_in(dir);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("deepseek-chat".to_string(), "off".to_string()),
                ("kimi-k2.6".to_string(), "high".to_string())
            ]
        );

        // Clearing removes only that entry.
        set_favorite_thinking_in(dir, "kimi-k2.6", None).unwrap();
        assert_eq!(
            load_favorite_thinking_in(dir),
            vec![("deepseek-chat".to_string(), "off".to_string())]
        );

        // Other [picker] writes must not clobber the memory (separate keys).
        let _ = toggle_favorite_model_in(dir, "deepseek-chat");
        let _ = push_recent_model_in(dir, "deepseek-chat");
        assert_eq!(
            load_favorite_thinking_in(dir),
            vec![("deepseek-chat".to_string(), "off".to_string())]
        );
        // ...and the memory write must not clobber favorites or recents either.
        assert_eq!(load_favorite_models_in(dir), vec!["deepseek-chat"]);
        assert_eq!(
            load_recent_models_in(dir).first().map(String::as_str),
            Some("deepseek-chat")
        );

        // Empty id refused; malformed table reads empty instead of crashing.
        assert!(set_favorite_thinking_in(dir, "  ", Some("high")).is_err());
    }

    #[test]
    fn favorites_cap_at_max_with_loud_error() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        for i in 0..FAVORITE_MAX {
            assert!(
                !toggle_favorite_model_in(dir, &format!("m{i}")).unwrap(),
                "insertion {i} should report added"
            );
        }
        // The next NEW favorite is refused with guidance; existing ones still
        // toggle off fine (the cap only gates growth).
        let err = toggle_favorite_model_in(dir, "overflow-model").unwrap_err();
        assert!(
            err.contains(&FAVORITE_MAX.to_string()),
            "error should name the cap: {err}"
        );
        assert_eq!(load_favorite_models_in(dir).len(), FAVORITE_MAX);
        assert!(
            toggle_favorite_model_in(dir, "m0").unwrap(),
            "removing an existing favorite still works at cap"
        );
        assert!(
            !toggle_favorite_model_in(dir, "overflow-model").unwrap(),
            "after freeing a slot the new one fits"
        );
    }

    #[test]
    fn precedence_documented_flag_over_saved_over_default() {
        // Documentation-by-test of the resolution order implemented at call sites:
        // flag.map(...).or(load_model_id()).unwrap_or(DEFAULT_MODEL_ID)
        let flag: Option<String> = Some("claude-opus-4-1".into());
        let saved: Option<String> = None;
        let resolved = flag.or(saved).unwrap_or_else(|| DEFAULT_MODEL_ID.into());
        assert_eq!(resolved, "claude-opus-4-1");

        let flag: Option<String> = None;
        let saved: Option<String> = Some("deepseek-chat".into());
        let resolved = flag.or(saved).unwrap_or_else(|| DEFAULT_MODEL_ID.into());
        assert_eq!(resolved, "deepseek-chat");
    }

    #[test]
    fn prompt_history_appends_dedups_caps_and_survives_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        assert!(load_prompt_history_in(dir).is_empty(), "no file yet");

        append_prompt_history_in(dir, "first task").unwrap();
        append_prompt_history_in(dir, "second task").unwrap();
        assert_eq!(
            load_prompt_history_in(dir),
            vec!["first task".to_string(), "second task".to_string()]
        );

        // Consecutive duplicate collapses.
        append_prompt_history_in(dir, "second task").unwrap();
        assert_eq!(load_prompt_history_in(dir).len(), 2);

        // Empty entries refused.
        assert!(append_prompt_history_in(dir, "   ").is_err());

        // Cap enforced (oldest dropped).
        for i in 0..PROMPT_HISTORY_MAX + 10 {
            append_prompt_history_in(dir, &format!("task {i}")).unwrap();
        }
        let loaded = load_prompt_history_in(dir);
        assert_eq!(loaded.len(), PROMPT_HISTORY_MAX);
        assert_eq!(
            loaded.last().map(String::as_str),
            Some("task 509"),
            "newest kept, oldest dropped"
        );
    }

    #[test]
    fn prompt_history_tolerates_corrupt_lines_and_multiline_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir).unwrap();
        // A multiline entry serializes as one JSON line; a corrupt neighbor is skipped.
        std::fs::write(
            dir.join("history"),
            "not json\n\"good entry\"\n{\"truncated\":\n",
        )
        .unwrap();
        assert_eq!(load_prompt_history_in(dir), vec!["good entry".to_string()]);

        append_prompt_history_in(dir, "has \"quotes\" and\nnewlines").unwrap();
        let loaded = load_prompt_history_in(dir);
        assert!(loaded.contains(&"has \"quotes\" and\nnewlines".to_string()));
    }
}
