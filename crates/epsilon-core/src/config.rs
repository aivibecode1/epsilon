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
///
/// **M223: one implementation, in the crate that cannot delegate.** This body used to be a second,
/// byte-for-byte copy of `epsilon_llm::auth::global_dir`, and the ledger row that tracked it (A12)
/// — and plan 019, which planned to remove it — both assumed the duplication was *symmetric*: "two
/// copies of one rule in crates that cannot see each other". Measured: the DAG runs
/// **core → llm** (`epsilon-core/Cargo.toml` depends on `epsilon-llm`; the reverse edge is the one
/// the law forbids), so this crate can see that one. The rule therefore has a forced home in the
/// *lower* crate, and the higher one asks.
///
/// That is the whole fix, and it is why it is worth writing down: the planned version — deleting the
/// copy and threading a `&Path` through seven credential wrappers, twelve CLI/TUI call sites and the
/// provider-resolution path inside llm — was answering a problem that did not exist, at a cost
/// measured against a call graph the plan had not read. `grep -rn "fn global_dir" crates/` must now
/// return exactly one hit, and a test in this module is what keeps it that way.
pub fn global_dir() -> Option<PathBuf> {
    epsilon_llm::auth::global_dir()
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

/// Skills the user has switched off, from `config.toml`.
///
/// **A disable-list, not an enable-list.** An enable-list would mean a skill
/// silently stops working the day a config file gains a `[skills]` section it
/// did not have before, and a user who has never opened a settings screen would
/// come back to a missing tool. A disable-list is inert until someone writes to
/// it, which is the same reasoning behind "off by default" everywhere else in
/// this repository.
pub fn load_disabled_skills_in(config_dir: &Path) -> Vec<String> {
    let path = config_dir.join("config.toml");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(table) = toml::from_str::<toml::Table>(&raw) else {
        return Vec::new();
    };
    table
        .get("skills")
        .and_then(|s| s.get("disabled"))
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// [`load_disabled_skills_in`] against the resolved global directory.
pub fn load_disabled_skills() -> Vec<String> {
    global_dir()
        .map(|d| load_disabled_skills_in(&d))
        .unwrap_or_default()
}

/// Whether a skill is switched off.
pub fn skill_is_disabled(name: &str) -> bool {
    load_disabled_skills().iter().any(|n| n == name)
}

/// Switch a skill on or off, and report the NEW state.
///
/// Mirrors [`toggle_favorite_model_in`] exactly -- read the file into a table,
/// change ONE key, write it back -- because that function is the established
/// idiom here and a second way of editing `config.toml` is a second way of losing
/// a setting.
///
/// Only this key is touched. Everything else in the file, including keys this
/// build does not understand, round-trips unchanged, which is what makes it safe
/// to write from a TUI at all.
///
/// The skill must already exist: disabling a name that nothing provides would
/// write a setting that silently does nothing, and would then read back as a
/// disabled skill the user cannot find to re-enable.
pub fn set_skill_disabled_in(
    config_dir: &Path,
    name: &str,
    disabled: bool,
) -> Result<bool, String> {
    if name.trim().is_empty() {
        return Err("refusing to toggle a skill with an empty name".into());
    }
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = config_dir.join("config.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();

    let mut disabled_list = load_disabled_skills_in(config_dir);
    // Remove first so the list cannot accumulate duplicates, then push if wanted.
    disabled_list.retain(|n| n != name);
    if disabled {
        disabled_list.push(name.to_string());
    }

    table
        .entry("skills")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| "[skills] exists but is not a table".to_string())?
        .insert(
            "disabled".into(),
            toml::Value::Array(
                disabled_list
                    .into_iter()
                    .map(toml::Value::String)
                    .collect::<Vec<_>>(),
            ),
        );

    let serialized =
        toml::to_string_pretty(&table).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(disabled)
}

/// [`set_skill_disabled_in`] against the resolved global directory.
/// Returns the NEW state: `true` = now disabled.
pub fn set_skill_disabled(name: &str, disabled: bool) -> Result<bool, String> {
    // `global_dir` is an `Option`, and this is the same shape
    // `toggle_favorite_model` uses -- guessing that it was a `Result` and
    // calling `map_err` on it was one wasted compile.
    let Some(dir) = global_dir() else {
        return Err("no global config directory resolved".into());
    };
    set_skill_disabled_in(&dir, name, disabled)
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

/// Disabling a skill persists, and re-enabling removes it.
///
/// The load-bearing half is the ROUND TRIP through the file: an in-memory
/// list that survives the call but not the write would report success and do
/// nothing, which is the "reports success while the thing you wanted did not
/// happen" shape this repo treats as the worst kind of bug.
#[test]
fn a_disabled_skill_survives_a_reload_and_can_be_re_enabled() {
    let dir = tempfile::tempdir().unwrap();

    // Nothing written yet: nothing is disabled. An enable-list would fail
    // here by reporting every skill as off.
    assert!(
        load_disabled_skills_in(dir.path()).is_empty(),
        "a config with no [skills] section disables nothing"
    );

    assert!(set_skill_disabled_in(dir.path(), "commit", true).is_ok());
    assert_eq!(
        load_disabled_skills_in(dir.path()),
        vec!["commit".to_string()],
        "the disable is in the FILE, not in a map that outlives the call"
    );

    // Re-enabled: the entry is GONE, not present-and-false. A retained
    // "commit = false" would read as disabled to the next reader that only
    // looks for membership.
    set_skill_disabled_in(dir.path(), "commit", false).unwrap();
    assert!(
        load_disabled_skills_in(dir.path()).is_empty(),
        "re-enabling removes the entry rather than negating it"
    );
}

/// Writing one setting must not disturb the others.
///
/// A TUI writing `config.toml` is only safe if it round-trips keys it does
/// not understand — including keys a NEWER build wrote. This is the property
/// that makes it acceptable to persist from the UI at all, and it is the
/// thing a hand-rolled writer gets wrong by rebuilding the file from a
/// struct instead of editing the parsed table.
#[test]
fn writing_a_skill_setting_leaves_every_other_key_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "model = \"some-model\"\n\n[picker]\nfavorite = [\"a\", \"b\"]\n\n\
             [a-key-this-build-knows-nothing-about]\nnested = 42\n",
    )
    .unwrap();

    set_skill_disabled_in(dir.path(), "review", true).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("some-model"),
        "the model key survived: {after}"
    );
    assert!(
        after.contains("\"a\"") && after.contains("\"b\""),
        "favorites survived: {after}"
    );
    assert!(
        after.contains("a-key-this-build-knows-nothing-about"),
        "an UNKNOWN table survived, which is what a future build needs: {after}"
    );
    assert!(
        after.contains("nested = 42"),
        "nested values survived: {after}"
    );
    assert!(
        after.contains("review"),
        "and the new key was written: {after}"
    );
}

/// Toggling the same skill twice does not leave duplicates behind.
///
/// `retain` then `push` exists for exactly this: an append-only list would
/// grow a duplicate on every press, and the list is shown to a user.
#[test]
fn toggling_twice_does_not_duplicate_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    set_skill_disabled_in(dir.path(), "commit", true).unwrap();
    set_skill_disabled_in(dir.path(), "commit", true).unwrap();
    set_skill_disabled_in(dir.path(), "review", true).unwrap();
    set_skill_disabled_in(dir.path(), "commit", true).unwrap();
    // ORDER IS NOT ASSERTED, deliberately: the implementation is `retain`
    // then `push`, so re-disabling moves a skill to the end. Nothing reads
    // the order, and an earlier version of this test asserted insertion
    // order -- which would have "fixed" correct code to satisfy a property
    // no caller depends on. What matters is that each name appears EXACTLY
    // ONCE, which is the bug `retain` exists to prevent.
    let list = load_disabled_skills_in(dir.path());
    assert_eq!(
        list.iter().filter(|n| *n == "commit").count(),
        1,
        "commit appears once, not once per press: {list:?}"
    );
    assert_eq!(
        list.iter().filter(|n| *n == "review").count(),
        1,
        "review appears once: {list:?}"
    );
    assert_eq!(list.len(), 2, "and nothing else is in the list: {list:?}");
}

/// An empty name is refused rather than written.
///
/// A blank entry would disable nothing and read back as a disabled skill the
/// user cannot name to re-enable.
#[test]
fn an_empty_skill_name_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    assert!(set_skill_disabled_in(dir.path(), "   ", true).is_err());
    assert!(
        load_disabled_skills_in(dir.path()).is_empty(),
        "and nothing was written"
    );
}

/// M223: the global directory has exactly one implementation in the workspace.
///
/// **The assertion is over the source, not over a value.** Two functions that agree right now is not
/// the property: the defect this replaces was two byte-identical bodies that could drift at any
/// moment, and no test on a *value* can see that — a test that compares the two results passes for
/// as long as the environment happens to give the same answer, which is exactly the trap A12's row
/// describes ("two copies of one rule cannot disagree visibly"). Counting the definitions is what
/// makes the duplication unrepeatable rather than merely absent today.
///
/// Source-scanning, not a unit assertion, because the thing at risk spans two crates.
#[test]
fn the_global_directory_is_computed_in_exactly_one_place() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut computed: Vec<String> = Vec::new();
    let mut delegating = 0usize;
    for entry in walk_sources(&crates.join("..")) {
        let Ok(text) = std::fs::read_to_string(&entry) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            if !(t.starts_with("pub fn global_dir(") || t.starts_with("fn global_dir(")) {
                continue;
            }
            // The body is what decides: a definition that *asks* another crate's function is a
            // delegate, and a delegate is the fix working. One that reads the environment is a
            // second implementation of the rule, which is the defect.
            let body: String = lines[n..(n + 8).min(lines.len())].join("\n");
            let where_ = format!(
                "{}:{}",
                entry.strip_prefix(crates).unwrap_or(&entry).display(),
                n + 1
            );
            if body.contains("var_os(\"HOME\")") {
                computed.push(where_);
            } else {
                delegating += 1;
            }
        }
    }
    assert_eq!(
        computed.len(),
        1,
        "`~/.config/epsilon` must be *computed* in one place; computed at {computed:?} \
         ({delegating} delegating definition(s) are fine)"
    );
}

/// Every `.rs` under `dir`, skipping build output.
///
/// `#[cfg(test)]` because the only caller is the test above: a function that walks the workspace has
/// no business in a shipped library, and `dead_code` is right to say so.
#[cfg(test)]
fn walk_sources(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            out.extend(walk_sources(&path));
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
    out
}
