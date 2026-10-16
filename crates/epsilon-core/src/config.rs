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

/// `[provider_custom.<name>]` from `config_dir/config.toml`, sorted by name (M225).
///
/// Rows without a usable `base_url` are skipped — a half-written entry must not take the whole
/// catalog down. A missing or unparseable file is an empty set, never an error, which is the rule
/// every reader here follows.
///
/// **This is the last reader of `config.toml` outside this module**, which is what lets `epsilon-llm`
/// drop its `toml` dependency (`cargo tree -p epsilon-llm -i toml` is the proof). The rows are
/// `epsilon_llm::custom::CustomProvider` because `epsilon-core` depends on `epsilon-llm` and not the
/// other way round — M223's asymmetry deciding where the type may be named.
#[must_use]
pub fn custom_providers_in(config_dir: &Path) -> Vec<epsilon_llm::custom::CustomProvider> {
    let Ok(raw) = std::fs::read_to_string(config_dir.join("config.toml")) else {
        return Vec::new();
    };
    let Ok(value) = raw.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut out: Vec<epsilon_llm::custom::CustomProvider> = value
        .get("provider_custom")
        .and_then(toml::Value::as_table)
        .map(|t| {
            t.iter()
                .filter_map(|(name, body)| {
                    let body = body.as_table()?;
                    let text = |k: &str| body.get(k).and_then(toml::Value::as_str).unwrap_or("");
                    epsilon_llm::custom::CustomProvider::from_config(
                        name,
                        text("base_url"),
                        text("env_key"),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `[oauth.<provider>]` from `config_dir/config.toml`, **one parse for both keys** (M224).
///
/// The two keys used to be read by two functions in `epsilon-llm`, each parsing the file for one
/// field: two reads per login, two places that could disagree about the same table, and the reason
/// that crate carried a `toml` dependency. This is the one reader, and the value type lives in
/// `epsilon-llm` because `epsilon-core` depends on it (M223's asymmetry decides which crate may name
/// the type and which may fill it).
///
/// A missing file, a missing table, a missing key, or a blank value is `None` — never an error.
/// The same rule `load_config_string_in` has: a typo in user configuration must not break a login.
#[must_use]
pub fn oauth_config_in(config_dir: &Path, provider: &str) -> epsilon_llm::oauth_login::OAuthConfig {
    use epsilon_llm::oauth_login::OAuthConfig;
    let Some(raw) = std::fs::read_to_string(config_dir.join("config.toml")).ok() else {
        return OAuthConfig::default();
    };
    let Ok(table) = raw.parse::<toml::Table>() else {
        return OAuthConfig::default();
    };
    let Some(entry) = table
        .get("oauth")
        .and_then(toml::Value::as_table)
        .and_then(|t| t.get(provider))
        .and_then(toml::Value::as_table)
    else {
        return OAuthConfig::default();
    };
    let text = |key: &str| {
        entry
            .get(key)
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    OAuthConfig {
        client_id: text("client_id"),
        enterprise_domain: text("enterprise_domain"),
    }
}

/// [`oauth_config_in`] against the resolved global directory.
#[must_use]
pub fn oauth_config(provider: &str) -> epsilon_llm::oauth_login::OAuthConfig {
    match global_dir() {
        Some(dir) => oauth_config_in(&dir, provider),
        None => epsilon_llm::oauth_login::OAuthConfig::default(),
    }
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

/// The user's home directory (`$HOME`, falling back to `$USERPROFILE`).
///
/// Same M223 shape as [`global_dir`]: the rule is computed in `epsilon-llm` and asked
/// for here. It exists because the skill-root table in `epsilon_core::skills` reads
/// several cross-harness conventions out of the home directory — `~/.claude/skills`,
/// `~/.codex/skills`, `~/.gemini/skills`, `~/.agents/skills` — and a second reader of
/// `HOME` is the duplication this delegate exists to prevent.
pub fn home_dir() -> Option<PathBuf> {
    epsilon_llm::auth::home_dir()
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

/// Replace the whole favorites list at once (P-047 `/scoped-models`
/// checklist: Space stages in memory, Ctrl+S lands the staged scope here).
/// Same table key and cap as [`toggle_favorite_model_in`]; over the cap is
/// refused with the file untouched, so a checklist that outgrew the cap
/// cannot truncate the saved scope by saving.
pub fn save_favorite_models_in(config_dir: &Path, models: &[String]) -> Result<(), String> {
    let favs: Vec<String> = models
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if favs.len() > FAVORITE_MAX {
        return Err(format!(
            "favorites are full ({FAVORITE_MAX}); unfavorite one first"
        ));
    }
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = config_dir.join("config.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();

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
    Ok(())
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

/// Extra skill directories from `[skills] extra_dirs`, global config only.
///
/// DSH's `customSkillDirs` and Pi's `skillPaths`, as a config key rather than
/// a CLI flag (both references configure it, neither passes it per-run).
/// Relative entries resolve against the config dir holding them; blank entries
/// are dropped. Missing or malformed values read as empty (fail-open reads):
/// an absent directory is already silent-empty at discovery, so inventing an
/// error here would refuse startups over a typo the scanner tolerates.
///
/// Global-only by design: these roots skip the project trust gate because the
/// user wrote them into their own file. A repository cannot declare skill
/// roots outside itself through this key (there is no project-level twin).
pub fn load_extra_skill_dirs_in(config_dir: &Path) -> Vec<PathBuf> {
    let path = config_dir.join("config.toml");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(table) = toml::from_str::<toml::Table>(&raw) else {
        return Vec::new();
    };
    table
        .get("skills")
        .and_then(|s| s.get("extra_dirs"))
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    let p = PathBuf::from(s);
                    if p.is_absolute() {
                        p
                    } else {
                        config_dir.join(p)
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// [`load_extra_skill_dirs_in`] against the resolved global directory.
pub fn load_extra_skill_dirs() -> Vec<PathBuf> {
    global_dir()
        .map(|d| load_extra_skill_dirs_in(&d))
        .unwrap_or_default()
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

// ---- display preferences (P-040; thinking visibility) ---------------------

/// Whether thinking blocks are hidden from the transcript (`[display]
/// hide_thinking`).
///
/// Off (visible) when unset or unreadable — hiding is the choice that must be
/// made, not the default, and a corrupt config must not blank the transcript.
pub fn load_hide_thinking_in(config_dir: &Path) -> bool {
    let path = config_dir.join("config.toml");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(table) = toml::from_str::<toml::Table>(&raw) else {
        return false;
    };
    table
        .get("display")
        .and_then(|d| d.get("hide_thinking"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Set `[display] hide_thinking`, preserving every other key. Persists
/// immediately; returns the file written.
pub fn save_hide_thinking_in(config_dir: &Path, hide: bool) -> Result<PathBuf, String> {
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = config_dir.join("config.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();

    table
        .entry("display")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| "[display] exists but is not a table".to_string())?
        .insert("hide_thinking".into(), toml::Value::Boolean(hide));

    let serialized =
        toml::to_string_pretty(&table).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// [`load_hide_thinking_in`] against the resolved global directory.
pub fn load_hide_thinking() -> bool {
    global_dir()
        .map(|d| load_hide_thinking_in(&d))
        .unwrap_or(false)
}

/// [`save_hide_thinking_in`] against the resolved global directory.
pub fn save_hide_thinking(hide: bool) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    save_hide_thinking_in(&dir, hide)
}

// ---- reasoning effort default (criterion 7) -------------------------------------

/// Default reasoning effort (`[thinking] level`) from `config_dir/config.toml`.
///
/// The standing default a launch starts from — **not** the per-favourite
/// memories in `[picker.favorite_thinking]`, which apply on F2 cycling and stay
/// exactly as they are. Same fail-open contract as every reader here: a missing
/// file, table or key, or a value this build does not know, is `None` — never an
/// error, and never a silent clamp, because a typo must not change how hard the
/// model thinks.
pub fn load_thinking_level_in(config_dir: &Path) -> Option<epsilon_contracts::ThinkingLevel> {
    let raw = load_config_string_in(config_dir, "thinking", "level")?;
    epsilon_contracts::ThinkingLevel::parse(&raw)
}

/// [`load_thinking_level_in`] against the resolved global directory.
pub fn load_thinking_level() -> Option<epsilon_contracts::ThinkingLevel> {
    global_dir().and_then(|d| load_thinking_level_in(&d))
}

/// Set `[thinking] level`, preserving every other key. Persists immediately;
/// returns the file written.
///
/// Same read-modify-write idiom as [`save_hide_thinking_in`]: a second way of
/// editing `config.toml` is a second way of losing a setting.
pub fn save_thinking_level_in(
    config_dir: &Path,
    level: epsilon_contracts::ThinkingLevel,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = config_dir.join("config.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();

    table
        .entry("thinking")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| "[thinking] exists but is not a table".to_string())?
        .insert(
            "level".into(),
            toml::Value::String(level.as_str().to_string()),
        );

    let serialized =
        toml::to_string_pretty(&table).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// [`save_thinking_level_in`] against the resolved global directory.
pub fn save_thinking_level(level: epsilon_contracts::ThinkingLevel) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    save_thinking_level_in(&dir, level)
}

// ---- permission preset default (Wave 20) -----------------------------------

/// Persisted permission preset name (`[permissions] preset`) from
/// `config_dir/config.toml`.
///
/// A string, not a `Preset`: `epsilon-safety` (where `Preset` lives) depends
/// on this crate, so naming the type here would be a dependency cycle. Name
/// validity is decided at the one place that can name it —
/// `epsilon_runtime::resolve_effective_preset` — and unknown names fall
/// through there, never reaching the gate.
///
/// Same fail-open contract as every reader here: a missing file, table or
/// key, a non-string value, or a blank value is `None` — never an error, and
/// never a silent clamp, because a typo must not change the policy a launch
/// runs under.
pub fn load_preset_in(config_dir: &Path) -> Option<String> {
    load_config_string_in(config_dir, "permissions", "preset")
}

/// [`load_preset_in`] against the resolved global directory.
pub fn load_preset() -> Option<String> {
    global_dir().and_then(|d| load_preset_in(&d))
}

/// Set `[permissions] preset`, preserving every other key. Persists
/// immediately; returns the file written.
///
/// Same read-modify-write idiom as [`save_thinking_level_in`]: a second way
/// of editing `config.toml` is a second way of losing a setting. Production
/// callers pass `Preset::name()`; the empty string is refused, like every
/// other saver here.
pub fn save_preset_in(config_dir: &Path, preset: &str) -> Result<PathBuf, String> {
    if preset.trim().is_empty() {
        return Err("refusing to persist an empty preset".into());
    }
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = config_dir.join("config.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();

    table
        .entry("permissions")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| "[permissions] exists but is not a table".to_string())?
        .insert(
            "preset".into(),
            toml::Value::String(preset.trim().to_string()),
        );

    let serialized =
        toml::to_string_pretty(&table).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// [`save_preset_in`] against the resolved global directory.
pub fn save_preset(preset: &str) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    save_preset_in(&dir, preset)
}

/// Persist `[tui] theme`, preserving every other key.
///
/// The settings panel writes this. Bare `/theme` still does not: that key is a
/// brightness toggle for the current run, and a toggle that rewrites a file on
/// every press is a different action. `name` is the spelling the theme already
/// uses (`dark`, `light`, `high-contrast`).
pub fn save_tui_theme_in(config_dir: &Path, name: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = config_dir.join("config.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();
    table
        .entry("tui")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| "[tui] exists but is not a table".to_string())?
        .insert("theme".into(), toml::Value::String(name.to_string()));
    let serialized =
        toml::to_string_pretty(&table).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
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

// ---- persisted live listings (criterion 6) -----------------------------------

/// One provider's live listing as persisted in `<dir>/live-models.json`:
/// the ids plus when they were fetched. No etag — a fetch replaces the whole
/// entry on success, so there is nothing to revalidate against.
///
/// A plain struct rather than a TOML key: listings are fetched data with
/// their own freshness, not user configuration, and mixing them into
/// `config.toml` would make every fetch rewrite the user's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveModelCacheEntry {
    /// Provider key, as in [`epsilon_llm::catalog`](`live_listing_available`).
    pub provider: String,
    /// Model ids from the last successful fetch.
    pub ids: Vec<String>,
    /// `SystemTime` millis when the fetch landed (see `now_ms` in the TUI).
    pub fetched_at_ms: u64,
}

/// Path of the live-listing cache under `config_dir` (`live-models.json`).
fn live_cache_path(config_dir: &Path) -> PathBuf {
    config_dir.join("live-models.json")
}

/// Parse a persisted cache file; `None` when it is corrupt or an old shape
/// (fail-open reads, matching [`load_model_id_in`]).
fn parse_live_cache(raw: &str) -> Option<Vec<LiveModelCacheEntry>> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let rows = value.as_array()?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(LiveModelCacheEntry {
            provider: row.get("provider")?.as_str()?.to_string(),
            ids: row
                .get("ids")?
                .as_array()?
                .iter()
                .map(|id| id.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()?,
            fetched_at_ms: row.get("fetched_at_ms")?.as_u64()?,
        });
    }
    Some(out)
}

/// Persisted live listings from `config_dir/live-models.json` (criterion 6).
///
/// Missing/unreadable file yields an empty list. A corrupt or old-shape file
/// is DELETED and reads as empty (delete-and-continue): the next successful
/// fetch rebuilds it, and startup never fails over fetched data. Never an
/// error, for the same reason every other reader here is fail-open.
pub fn load_live_cache_in(config_dir: &Path) -> Vec<LiveModelCacheEntry> {
    let path = live_cache_path(config_dir);
    match std::fs::read_to_string(&path) {
        Ok(raw) => match parse_live_cache(&raw) {
            Some(entries) => entries,
            None => {
                let _ = std::fs::remove_file(&path);
                Vec::new()
            }
        },
        Err(_) => Vec::new(),
    }
}

/// Replace the persisted live listings wholesale (criterion 6: save on every
/// successful fetch). Best-effort like every other writer here — errors are
/// returned for callers that want to stay silent, never fatal.
///
/// Written to a temp file and renamed, not written in place: two TUI
/// processes sharing a global dir interleave whole-file replaces, and a
/// torn in-place write would read as corrupt — deleting the other
/// process's good cache along with the torn half. Rename is atomic on one
/// filesystem, so readers see the old file or the new file, never half of
/// either. Last-writer-wins on content (no merge) is accepted: entries are
/// refetchable fetched data, and merging would invent a union neither
/// process observed.
pub fn save_live_cache_in(
    config_dir: &Path,
    entries: &[LiveModelCacheEntry],
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = live_cache_path(config_dir);
    let rows: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "provider": e.provider,
                "ids": e.ids,
                "fetched_at_ms": e.fetched_at_ms,
            })
        })
        .collect();
    let serialized =
        serde_json::to_string(&rows).map_err(|e| format!("serialize live cache: {e}"))?;
    let staging = path.with_extension("json.tmp");
    std::fs::write(&staging, serialized)
        .map_err(|e| format!("write {}: {e}", staging.display()))?;
    std::fs::rename(&staging, &path).map_err(|e| format!("rename {}: {e}", staging.display()))?;
    Ok(path)
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

    /// **Criterion 7: a project's own model choice survives a restart.**
    ///
    /// Written as a failing test first. The write site for a per-project default
    /// resolves `project_dir.or_else(global_dir)` (`epsilon-tui/src/app.rs:824`)
    /// and the only startup read is `default_model: load_model_id()`
    /// (`epsilon-tui/src/app/session.rs:93`), which reads the **global** dir. So the
    /// two disagree about *where* the value lives. This test says the read must
    /// answer for the project's own directory — the rule the fix has to satisfy,
    /// written before the fix exists.
    #[test]
    fn a_projects_model_choice_survives_a_restart() {
        let workspace = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let project_dir = workspace.path().join(".epsilon");
        std::fs::create_dir_all(&project_dir).unwrap();

        // What the "set default" path writes for this project.
        save_model_id_in(&project_dir, "anthropic/claude-opus-4-6").unwrap();

        // The startup rule under test: no flag, no explicit request — the project's
        // own setting is what must be resolved.
        let restored = load_model_id_in(&project_dir);
        assert_eq!(
            restored.as_deref(),
            Some("anthropic/claude-opus-4-6"),
            "the project's choice must be readable back from its own directory"
        );

        // **The control.** A second workspace with no own setting must not inherit
        // the first one's — the `.or_else(load_model_id)` chain makes the global dir
        // a shared fallback, and this assertion is what stops the fix becoming
        // cross-project contamination.
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            load_model_id_in(other.path()),
            None,
            "one project's model choice must not leak into another's"
        );

        // And isolation runs both ways: the global dir holding a value must not
        // override a project that set its own.
        assert_eq!(load_model_id_in(global.path()), None);
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
    fn favorites_bulk_save_round_trips_and_enforces_the_cap() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();

        // Whole-list save replaces, order preserved, rereadable.
        save_favorite_models_in(dir, &["b-model".to_string(), "a-model".to_string()])
            .expect("bulk save");
        assert_eq!(
            load_favorite_models_in(dir),
            vec!["b-model".to_string(), "a-model".to_string()]
        );

        // Over the cap refused, file untouched.
        let too_many: Vec<String> = (0..=FAVORITE_MAX).map(|i| format!("m-{i}")).collect();
        assert!(save_favorite_models_in(dir, &too_many).is_err());
        assert_eq!(
            load_favorite_models_in(dir),
            vec!["b-model".to_string(), "a-model".to_string()]
        );
    }

    /// **Criterion 7: the thinking default survives a restart, and a write
    /// leaves every other key alone.**
    ///
    /// `/think` used to write three things — the agent, the mirror field, the
    /// notice — and nothing to disk, so every launch started at `Off` and a user
    /// re-set the level by hand every time. The round trip is the whole fix on
    /// the config side, and the coexistence assertion is what keeps it from
    /// becoming a second way of losing a setting.
    #[test]
    fn a_thinking_default_round_trips_and_leaves_other_keys_alone() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();

        // Unset reads None; a value this build does not know is None too — a
        // typo must not silently change how hard the model thinks.
        assert_eq!(load_thinking_level_in(dir), None);
        std::fs::write(dir.join("config.toml"), "[thinking]\nlevel = \"banana\"\n").unwrap();
        assert_eq!(load_thinking_level_in(dir), None);

        // Set, read back; then re-set beside another key and read both.
        save_thinking_level_in(dir, epsilon_contracts::ThinkingLevel::XHigh).unwrap();
        assert_eq!(
            load_thinking_level_in(dir),
            Some(epsilon_contracts::ThinkingLevel::XHigh)
        );
        toggle_favorite_model_in(dir, "deepseek-chat").unwrap();
        save_thinking_level_in(dir, epsilon_contracts::ThinkingLevel::High).unwrap();
        assert_eq!(
            load_thinking_level_in(dir),
            Some(epsilon_contracts::ThinkingLevel::High)
        );
        assert_eq!(
            load_favorite_models_in(dir),
            vec!["deepseek-chat".to_string()],
            "the [picker] table survives a thinking write"
        );
    }

    /// **Wave 20: `[permissions] preset` round-trips, and a write leaves
    /// every other key alone.**
    ///
    /// `/preset` used to swap only the in-memory gate, so every restart came
    /// back `auto` with no notice. The round trip is the whole fix on the
    /// config side; the coexistence assertion keeps it from becoming a second
    /// way of losing a setting.
    ///
    /// Name validity is NOT decided here: this layer persists the string and
    /// reads it back, and `epsilon_runtime::resolve_effective_preset` (the one
    /// place that can name `Preset`) decides what an unknown name means.
    #[test]
    fn preset_round_trips_and_leaves_other_keys_alone() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();

        // Unset reads None; a blank value is not configuration either.
        assert_eq!(load_preset_in(dir), None);
        std::fs::write(dir.join("config.toml"), "[permissions]\npreset = \"   \"\n").unwrap();
        assert_eq!(load_preset_in(dir), None);

        // Set, read back; then re-set beside another key and read both.
        save_preset_in(dir, "read-only").unwrap();
        assert_eq!(load_preset_in(dir).as_deref(), Some("read-only"));
        toggle_favorite_model_in(dir, "deepseek-chat").unwrap();
        save_preset_in(dir, "workspace-write").unwrap();
        assert_eq!(load_preset_in(dir).as_deref(), Some("workspace-write"));
        assert_eq!(
            load_favorite_models_in(dir),
            vec!["deepseek-chat".to_string()],
            "the [picker] table survives a preset write"
        );
    }

    #[test]
    fn preset_missing_blank_or_malformed_loads_as_none() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();

        assert_eq!(load_preset_in(dir), None, "no file yet");

        std::fs::write(dir.join("config.toml"), "not [ valid toml {{{{").unwrap();
        assert_eq!(load_preset_in(dir), None);

        std::fs::write(dir.join("config.toml"), "[permissions]\npreset = 7\n").unwrap();
        assert_eq!(load_preset_in(dir), None, "a non-string is not our key");

        std::fs::write(dir.join("config.toml"), "[other]\nkey = 1\n").unwrap();
        assert_eq!(load_preset_in(dir), None);
    }

    #[test]
    fn preset_empty_refuses_to_persist_and_non_table_section_errors() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        assert!(save_preset_in(dir, "  ").is_err());

        // A pre-existing scalar `permissions = ...` must error rather than be
        // silently clobbered — same shape as the `[model]` guard above.
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("config.toml"), "permissions = \"scalar\"\n").unwrap();
        assert!(
            save_preset_in(dir, "read-only").is_err(),
            "must refuse to overwrite a non-table [permissions]"
        );
    }

    /// **Criterion 7: a project's own thinking choice is read from its own
    /// directory, and does not leak into a workspace that has none.**
    ///
    /// Same control shape as `a_projects_model_choice_survives_a_restart`: the
    /// fix is only correct if the second workspace still answers `None`.
    #[test]
    fn a_projects_thinking_choice_does_not_leak_into_another_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let project_dir = workspace.path().join(".epsilon");
        std::fs::create_dir_all(&project_dir).unwrap();
        save_thinking_level_in(&project_dir, epsilon_contracts::ThinkingLevel::High).unwrap();

        assert_eq!(
            load_thinking_level_in(&project_dir),
            Some(epsilon_contracts::ThinkingLevel::High),
            "the project's choice must be readable back from its own directory"
        );

        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            load_thinking_level_in(other.path()),
            None,
            "one workspace's default must not leak into another's"
        );
        assert_eq!(
            load_thinking_level_in(workspace.path()),
            None,
            "...not even into the workspace root that owns the .epsilon directory"
        );
    }

    #[test]
    fn hide_thinking_round_trips_and_leaves_other_keys_alone() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();

        // Unset reads visible; corrupt reads visible.
        assert!(!load_hide_thinking_in(dir));
        std::fs::write(dir.join("config.toml"), "not = [valid").unwrap();
        assert!(!load_hide_thinking_in(dir));

        // Set true, read back true; set false, read back false.
        save_hide_thinking_in(dir, true).unwrap();
        assert!(load_hide_thinking_in(dir));
        save_hide_thinking_in(dir, false).unwrap();
        assert!(!load_hide_thinking_in(dir));

        // Coexists with other tables: favorites survive a hide write and back.
        toggle_favorite_model_in(dir, "deepseek-chat").unwrap();
        save_hide_thinking_in(dir, true).unwrap();
        assert!(load_hide_thinking_in(dir));
        assert_eq!(
            load_favorite_models_in(dir),
            vec!["deepseek-chat".to_string()]
        );
        save_hide_thinking_in(dir, false).unwrap();
        assert_eq!(
            load_favorite_models_in(dir),
            vec!["deepseek-chat".to_string()],
            "unhiding must not clobber favorites either"
        );
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

/// M223: the home directory has exactly one implementation in the workspace.
///
/// **The assertion is over the source, not over a value.** Two functions that agree right now is not
/// the property: the defect this replaces was two byte-identical bodies that could drift at any
/// moment, and no test on a *value* can see that — a test that compares the two results passes for
/// as long as the environment happens to give the same answer, which is exactly the trap A12's row
/// describes ("two copies of one rule cannot disagree visibly"). Counting the definitions is what
/// makes the duplication unrepeatable rather than merely absent today.
///
/// Source-scanning, not a unit assertion, because the thing at risk spans two crates.
///
/// The scanned names widened when the skill-root table grew: `global_dir` used to be the only
/// consumer of the home directory, so it was the only name worth counting. Reading
/// `~/.claude/skills` and its siblings made `home_dir` a second entry point to the same
/// environment variables, and a gate that counted one of the two would have let the other
/// grow its own reader. The invariant is unchanged and is now stated at the level it actually
/// holds: **`$HOME` is read in exactly one place.**
#[test]
fn the_home_directory_is_computed_in_exactly_one_place() {
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
            if !(t.starts_with("pub fn home_dir(")
                || t.starts_with("fn home_dir(")
                || t.starts_with("pub fn global_dir(")
                || t.starts_with("fn global_dir("))
            {
                continue;
            }
            // The body is what decides: a definition that *asks* another function is a
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
        "`$HOME` must be *computed* in one place; computed at {computed:?} \
         ({delegating} delegating definition(s) are fine)"
    );
}

/// `[provider_custom.<name>]` rows, sorted, with unusable rows skipped (M225).
///
/// The name is the one this test had in `epsilon-llm`: the read moved here, the assertion did not,
/// and the repository's guard against silently-lost coverage (M196's five tests) keys on names.
#[test]
fn load_from_toml_dir_sorted_and_tolerant() {
    let tmp = tempfile::tempdir().expect("tmp");
    std::fs::write(
        tmp.path().join("config.toml"),
        "[provider_custom.zserver]\n\
         base_url = \"http://z.example/v1/\"\n\
         env_key = \"Z_KEY\"\n\n\
         [provider_custom.aserver]\n\
         base_url = \"http://a.example/v1\"\n\n\
         [provider_custom.broken]\n\
         env_key = \"NO_BASE_URL\"\n\n\
         [picker]\n\
         favorite = [\"faux-1\"]\n",
    )
    .unwrap();

    let customs = custom_providers_in(tmp.path());
    assert_eq!(customs.len(), 2, "broken row skipped, valid rows kept");
    assert_eq!(customs[0].name, "aserver");
    assert_eq!(customs[0].env_key, "");
    assert_eq!(customs[1].name, "zserver");
    assert_eq!(customs[1].env_key, "Z_KEY");
    assert_eq!(customs[1].base_url, "http://z.example/v1");

    // Other config keys survive untouched (we only read).
    let raw = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
    let value: toml::Table = raw.parse().unwrap();
    assert!(
        value.get("picker").is_some(),
        "[picker] still present: this reader must not rewrite the file"
    );
}

/// A workspace with no custom providers has none — a missing file, and a file without the table, are
/// both "none" rather than an error.
#[test]
fn missing_file_reads_empty() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(custom_providers_in(tmp.path()).is_empty());
    std::fs::write(tmp.path().join("config.toml"), "[picker]\n").unwrap();
    assert!(
        custom_providers_in(tmp.path()).is_empty(),
        "a config with no `[provider_custom]` table yields no rows"
    );
    std::fs::write(tmp.path().join("config.toml"), "not = [valid").unwrap();
    assert!(
        custom_providers_in(tmp.path()).is_empty(),
        "an unparseable file is an empty set, never a panic"
    );
}

/// M224: `[oauth.<provider>].client_id` comes from the config table — read here, once, for both keys.
///
/// The two functions this replaces (`client_id_from_config`, `enterprise_domain_from_config`) each
/// parsed the file for one field, so their tests could only ever cover one key at a time — and the
/// case below, **a provider with both**, was not expressible. A reader that cannot be asked "what
/// does this provider's OAuth table say?" is a reader with a seam in it.
///
/// The name is the one this test had in `epsilon-llm`: the read moved, the assertion did not, and the
/// repository's guard against silently-lost coverage (M196's five tests) keys on names for exactly
/// this reason.
#[test]
fn client_id_resolution_reads_the_oauth_config_table() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[oauth.anthropic]\nclient_id = \"cid-anthropic\"\n\
         [oauth.github-copilot]\nclient_id = \"cid-copilot\"\nenterprise_domain = \"ghe.example.com\"\n",
    )
    .unwrap();

    let copilot = oauth_config_in(dir.path(), "github-copilot");
    assert_eq!(
        copilot.client_id.as_deref(),
        Some("cid-copilot"),
        "and the same reading brought the enterprise domain along"
    );
    assert_eq!(
        copilot.enterprise_domain.as_deref(),
        Some("ghe.example.com")
    );

    // A provider with only a client id, and one with neither, stay distinguishable.
    let anthropic = oauth_config_in(dir.path(), "anthropic");
    assert_eq!(anthropic.client_id.as_deref(), Some("cid-anthropic"));
    assert_eq!(anthropic.enterprise_domain, None);
    assert_eq!(oauth_config_in(dir.path(), "xai").client_id, None);

    // Malformed config, a missing file, and a blank value all fall through rather than panic: a
    // typo in user configuration must not break a login.
    std::fs::write(dir.path().join("config.toml"), "not = [valid").unwrap();
    assert_eq!(oauth_config_in(dir.path(), "anthropic").client_id, None);
    std::fs::write(
        dir.path().join("config.toml"),
        "[oauth.anthropic]\nclient_id = \"  \"\n",
    )
    .unwrap();
    assert_eq!(
        oauth_config_in(dir.path(), "anthropic").client_id,
        None,
        "a blank value is absent, not a blank credential"
    );
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(oauth_config_in(empty.path(), "anthropic").client_id, None);
}

/// `[oauth.<provider>] enterprise_domain` (M55) — GitHub Copilot Business/Enterprise accounts run
/// the ceremony on the customer's host. `None` for individual accounts and every other provider, and
/// a missing config never panics.
///
/// Also the name this test had in `epsilon-llm`; only the reader moved (M224).
#[test]
fn enterprise_domain_comes_from_the_config_table() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[oauth.github-copilot]\nenterprise_domain = \"ghe.example.com\"\n",
    )
    .unwrap();
    assert_eq!(
        oauth_config_in(dir.path(), "github-copilot").enterprise_domain,
        Some("ghe.example.com".to_string())
    );
    // Other providers have no enterprise domain; missing config never panics.
    assert_eq!(oauth_config_in(dir.path(), "xai").enterprise_domain, None);
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        oauth_config_in(empty.path(), "github-copilot").enterprise_domain,
        None
    );
}

/// Values are trimmed on the way in, because a pasted YAML-ish `"  cid  "` is a typo the user should
/// not have to debug at the token endpoint.
#[test]
fn an_oauth_value_is_trimmed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[oauth.anthropic]\nclient_id = \"  cid  \"\nenterprise_domain = \" ghe.example.com \"\n",
    )
    .unwrap();
    let cfg = oauth_config_in(dir.path(), "anthropic");
    assert_eq!(cfg.client_id.as_deref(), Some("cid"));
    assert_eq!(cfg.enterprise_domain.as_deref(), Some("ghe.example.com"));
}

/// `[skills] extra_dirs`: absolute kept, relative joined to the config dir,
/// blanks dropped, missing/malformed reads as empty (fail-open, like every
/// reader here — discovery already treats an absent root as empty).
#[test]
fn extra_skill_dirs_read_absolute_relative_and_blank_shapes() {
    let dir = tempfile::tempdir().unwrap();
    // Absolute on every platform (a `/opt/...` literal is absolute on POSIX
    // but root-relative on Windows, so the absolute case is built, not written).
    let absolute = dir.path().join("abs-skills");
    let toml = format!(
        "[skills]\nextra_dirs = [{}, \"team/skills\", \"   \", 7]\n",
        toml::Value::String(absolute.to_string_lossy().into_owned())
    );
    std::fs::write(dir.path().join("config.toml"), toml).unwrap();
    assert_eq!(
        load_extra_skill_dirs_in(dir.path()),
        vec![absolute, dir.path().join("team/skills")],
        "absolute kept, relative joined, blank and non-string dropped"
    );

    let empty = tempfile::tempdir().unwrap();
    assert!(load_extra_skill_dirs_in(empty.path()).is_empty());
    std::fs::write(dir.path().join("config.toml"), "not [ valid toml {{{{").unwrap();
    assert!(load_extra_skill_dirs_in(dir.path()).is_empty());
}

/// Criterion 6: live listings persist as one small JSON file and round-trip
/// (ids + fetch timestamp, no etag).
#[test]
fn live_cache_round_trips_ids_and_timestamps() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        load_live_cache_in(dir.path()).is_empty(),
        "no file yet reads as empty, never an error"
    );
    let entries = vec![
        LiveModelCacheEntry {
            provider: "deepseek".to_string(),
            ids: vec!["deepseek-v4-pro".to_string()],
            fetched_at_ms: 1_700_000_000_000,
        },
        LiveModelCacheEntry {
            provider: "openai".to_string(),
            ids: vec![],
            fetched_at_ms: 1_700_000_060_000,
        },
    ];
    save_live_cache_in(dir.path(), &entries).expect("save");
    assert_eq!(load_live_cache_in(dir.path()), entries);
    // Atomic write: no staging file survives a save, so a concurrent
    // reader never meets half a write.
    assert!(
        std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .all(|n| n != "live-models.json.tmp"),
        "the staging file is renamed, never left behind"
    );
}

/// Criterion 6: a corrupt (or old-shape) cache file deletes-and-continues —
/// the next fetch rebuilds it, and startup never fails over it.
#[test]
fn corrupt_live_cache_deletes_and_continues() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live-models.json");

    std::fs::write(&path, "not json at all {{{").unwrap();
    assert!(
        load_live_cache_in(dir.path()).is_empty(),
        "garbage reads as empty"
    );
    assert!(!path.exists(), "a corrupt file is deleted so it cannot rot");

    // Old shape (an object, not the array this version writes).
    std::fs::write(&path, r#"{"providers": {"deepseek": ["a"]}}"#).unwrap();
    assert!(load_live_cache_in(dir.path()).is_empty());
    assert!(!path.exists(), "an old shape is deleted the same way");

    // A row missing its timestamp is old too, not half-loaded.
    std::fs::write(&path, r#"[{"provider": "deepseek", "ids": ["a"]}]"#).unwrap();
    assert!(load_live_cache_in(dir.path()).is_empty());
    assert!(!path.exists());
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
