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
}
