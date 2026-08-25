//! Provider credential store — opencode's `auth.json` pattern (M22).
//!
//! opencode lets a user run `opencode auth login`, pick a provider, paste a key
//! once, and every later session just works; the key lives in
//! `~/.local/share/opencode/auth.json`. Epsilon ports that experience: the file
//! is `<global config dir>/auth.json` (`~/.config/epsilon/auth.json`), holding
//! `{ "provider-id": { "type": "api", "key": "..." } }` — the same shape as
//! opencode's `Auth.Info` (`api` variant, the only kind our direct-HTTP wires
//! need).
//!
//! Precedence lives in [`crate::catalog::resolve_api_key`]: **stored key wins**
//! over the environment variable (the login flow is the documented path), env
//! vars remain the fallback for CI and users who prefer them.
//!
//! Placement note: this store lives in epsilon-llm (not core) because pi keeps
//! provider auth inside its `packages/ai` brick and Epsilon's dependency law
//! (`contracts <- llm <- core`) forbids llm from reaching core.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One stored provider credential (opencode `Auth.Info`, `api` variant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCredential {
    /// Literal `"api"` (discriminator matching opencode's schema).
    #[serde(rename = "type")]
    pub kind: String,
    /// The secret API key.
    pub key: String,
}

/// `auth.json` inside a config dir (opencode's file name and location).
fn auth_path_in(config_dir: &Path) -> PathBuf {
    config_dir.join("auth.json")
}

/// `~/.config/epsilon` — resolved from `$HOME`, falling back to `$USERPROFILE`.
/// Lives here (not core) because the credential store must be reachable from
/// epsilon-llm and the dependency law forbids llm → core.
pub fn global_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".config").join("epsilon"))
}

fn parse_auth_map(raw: &str) -> std::collections::BTreeMap<String, StoredCredential> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// All stored credentials from `config_dir/auth.json` keyed by provider id
/// (`anthropic`, `openai`, ...). Corrupt/missing files yield an empty map —
/// credentials are advisory at read time exactly like the rest of config;
/// env vars remain the fallback path in [`crate::catalog::resolve_api_key`].
pub fn load_credentials_in(
    config_dir: &Path,
) -> std::collections::BTreeMap<String, StoredCredential> {
    match std::fs::read_to_string(auth_path_in(config_dir)) {
        Ok(raw) => parse_auth_map(&raw),
        Err(_) => Default::default(),
    }
}

/// Store `key` for `provider_id`, preserving other entries. Creates the dir.
/// Writes 0600 permissions on unix (opencode writes 0o600); Windows relies on
/// the user-profile ACL, which is already user-scoped.
pub fn save_credential_in(
    config_dir: &Path,
    provider_id: &str,
    key: &str,
) -> Result<PathBuf, String> {
    if provider_id.trim().is_empty() {
        return Err("refusing to store a credential under an empty provider id".into());
    }
    if key.trim().is_empty() {
        return Err("refusing to store an empty API key".into());
    }
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("create {}: {e}", config_dir.display()))?;
    let path = auth_path_in(config_dir);

    let mut map = load_credentials_in(config_dir);
    map.insert(
        provider_id.to_string(),
        StoredCredential {
            kind: "api".to_string(),
            key: key.to_string(),
        },
    );
    write_auth_file(&path, &map)?;
    Ok(path)
}

/// Remove the credential for `provider_id`. Reports whether one existed.
pub fn remove_credential_in(config_dir: &Path, provider_id: &str) -> Result<bool, String> {
    let path = auth_path_in(config_dir);
    let mut map = load_credentials_in(config_dir);
    let existed = map.remove(provider_id).is_some();
    if existed || path.exists() {
        write_auth_file(&path, &map)?;
    }
    Ok(existed)
}

fn write_auth_file(
    path: &Path,
    map: &std::collections::BTreeMap<String, StoredCredential>,
) -> Result<(), String> {
    let serialized =
        serde_json::to_string_pretty(map).map_err(|e| format!("serialize auth: {e}"))?;
    std::fs::write(path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// [`load_credentials_in`] against the resolved global directory.
pub fn load_credentials() -> std::collections::BTreeMap<String, StoredCredential> {
    global_dir()
        .map(|d| load_credentials_in(&d))
        .unwrap_or_default()
}

/// [`save_credential_in`] against the resolved global directory.
pub fn save_credential(provider_id: &str, key: &str) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    save_credential_in(&dir, provider_id, key)
}

/// [`remove_credential_in`] against the resolved global directory.
pub fn remove_credential(provider_id: &str) -> Result<bool, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    remove_credential_in(&dir, provider_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_remove_round_trip_preserving_other_providers() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();

        save_credential_in(dir, "anthropic", "sk-ant-test").expect("save");
        // Second entry must survive the first one.
        save_credential_in(dir, "openai", "sk-oauth").expect("save");

        let creds = load_credentials_in(dir);
        assert_eq!(creds.len(), 2);
        assert_eq!(creds["anthropic"].kind, "api");
        assert_eq!(creds["anthropic"].key, "sk-ant-test");
        assert_eq!(creds["openai"].key, "sk-oauth");

        // Overwrite semantics: same provider id replaces its key only.
        save_credential_in(dir, "anthropic", "sk-rotated").expect("rotate");
        let creds = load_credentials_in(dir);
        assert_eq!(creds["anthropic"].key, "sk-rotated");
        assert_eq!(creds["openai"].key, "sk-oauth");

        // Remove reports existence accurately and prunes the entry.
        assert!(remove_credential_in(dir, "anthropic").expect("remove"));
        assert!(!remove_credential_in(dir, "anthropic").expect("remove again"));
        let creds = load_credentials_in(dir);
        assert_eq!(creds.len(), 1);
        assert!(creds.contains_key("openai"));
    }

    #[test]
    fn missing_and_corrupt_files_fail_open_to_empty() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        assert!(
            load_credentials_in(dir).is_empty(),
            "missing auth.json is empty, not an error"
        );

        std::fs::write(auth_path_in(dir), "{ not json !!!").unwrap();
        assert!(
            load_credentials_in(dir).is_empty(),
            "corrupt auth.json fails open (advisory read contract)"
        );
    }

    #[test]
    fn empty_provider_or_key_is_rejected_before_any_write() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        assert!(save_credential_in(dir, "  ", "k").is_err());
        assert!(save_credential_in(dir, "p", "").is_err());
        assert!(save_credential_in(dir, "p", "   ").is_err());
        assert!(
            !auth_path_in(dir).exists(),
            "rejections must not create the file"
        );
    }

    #[test]
    fn stored_shape_matches_opencode_schema() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        save_credential_in(dir, "groq", "gsk_test").expect("save");
        let raw = std::fs::read_to_string(auth_path_in(dir)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            v["groq"]["type"], "api",
            "discriminator matches opencode Auth.Info"
        );
        assert_eq!(v["groq"]["key"], "gsk_test");
    }

    #[test]
    fn blank_stored_key_is_ignored_by_resolution() {
        // resolve_api_key trims a stored key and treats blank as absent, so the
        // store itself refuses to persist one (save_credential_in rejects).
        // This test pins that guard: nothing blank can enter the file.
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        assert!(
            save_credential_in(dir, "faux-provider-x", "   ").is_err(),
            "blank keys are rejected at write time"
        );
        assert!(load_credentials_in(dir).is_empty(), "nothing was persisted");
    }
}
