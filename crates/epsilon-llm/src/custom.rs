//! Custom provider declarations (M28) — opencode's `provider.options.baseURL`
//! and Hermes' `base_url_override`, ported onto our catalog seam.
//!
//! Users declare OpenAI-compatible endpoints as `[provider_custom.<name>]`
//! tables in the global config (`~/.config/epsilon/config.toml`):
//!
//! ```toml
//! [provider_custom.lmstudio]
//! base_url = "http://localhost:1234/v1"
//! # env_key = "LMSTUDIO_API_KEY"   # optional; omit for keyless local servers
//! ```
//!
//! The model id `lmstudio/foo` then streams to `<base_url>/chat/completions`,
//! with the API key resolved for provider name `lmstudio` (stored
//! `auth.json` entry first, then the declared env var). This module owns both
//! the TOML parsing and the [`CustomProvider`] type so wire routing and its
//! data live in one crate; nothing else in the workspace re-parses it.

use std::path::Path;

/// One user-declared OpenAI-compatible endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomProvider {
    /// Provider prefix before `/` in a model id (`lmstudio` of
    /// `lmstudio/qwen2.5-7b-instruct`). Lowercase.
    pub name: String,
    /// Endpoint root without trailing slash (`http://localhost:1234/v1`).
    pub base_url: String,
    /// Env var holding the API key. Empty = no key sent (local servers).
    pub env_key: String,
}

impl CustomProvider {
    /// Build from raw config strings; `None` when there is no usable endpoint.
    /// Normalizes name to lowercase and strips one trailing slash so endpoint
    /// joins never double it.
    pub fn from_config(name: &str, base_url: &str, env_key: &str) -> Option<Self> {
        let base_url = base_url.trim().trim_end_matches('/');
        if base_url.is_empty() {
            return None;
        }
        Some(Self {
            name: name.trim().to_ascii_lowercase(),
            base_url: base_url.to_string(),
            env_key: env_key.trim().to_string(),
        })
    }

    /// The provider segment a model id must carry to route here.
    pub fn matches_id(&self, model_id: &str) -> bool {
        model_id
            .split_once('/')
            .is_some_and(|(prefix, _)| prefix.eq_ignore_ascii_case(&self.name))
    }
}

/// Look up a custom provider by the `<name>/...` prefix of a model id.
pub fn lookup_custom<'a>(
    customs: &'a [CustomProvider],
    model_id: &str,
) -> Option<&'a CustomProvider> {
    customs.iter().find(|c| c.matches_id(model_id))
}

/// Custom providers declared as `[provider_custom.<name>]` tables in
/// `config_dir/config.toml`, sorted by name. Rows without a usable
/// `base_url` are skipped — a half-written entry must not take the whole
/// catalog down.
pub fn load_custom_providers_in(config_dir: &Path) -> Vec<CustomProvider> {
    let raw = match std::fs::read_to_string(config_dir.join("config.toml")) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let value: toml::Value = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<CustomProvider> = value
        .get("provider_custom")
        .and_then(|t| t.as_table())
        .map(|t| {
            t.iter()
                .filter_map(|(name, body)| {
                    CustomProvider::from_config(
                        name,
                        body.get("base_url").and_then(|v| v.as_str()).unwrap_or(""),
                        body.get("env_key").and_then(|v| v.as_str()).unwrap_or(""),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// [`load_custom_providers_in`] against the resolved global config directory
/// (`~/.config/epsilon`; same file that holds `[picker]` state).
pub fn load_custom_providers() -> Vec<CustomProvider> {
    crate::auth::global_dir()
        .map(|d| load_custom_providers_in(&d))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_normalizes_and_rejects_empty_endpoints() {
        let c = CustomProvider::from_config("LMStudio", "http://localhost:1234/v1/", "")
            .expect("valid row");
        assert_eq!(c.name, "lmstudio");
        assert_eq!(c.base_url, "http://localhost:1234/v1");
        assert_eq!(c.env_key, "");

        // No usable endpoint: skipped, never half-constructed.
        assert!(CustomProvider::from_config("x", "   ", "").is_none());
        assert!(CustomProvider::from_config("x", "", "SOME_KEY").is_none());
    }

    #[test]
    fn matches_id_is_prefix_and_case_insensitive() {
        let c = CustomProvider::from_config("lmstudio", "http://x/v1", "").unwrap();
        assert!(c.matches_id("lmstudio/qwen2.5-7b-instruct"));
        assert!(c.matches_id("LMStudio/foo"));
        assert!(
            !c.matches_id("lm-studio/foo"),
            "prefix is the whole segment"
        );
        assert!(!c.matches_id("openai/gpt-4"));
        assert!(!c.matches_id("no-slash"));
    }

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

        let customs = load_custom_providers_in(tmp.path());
        assert_eq!(customs.len(), 2, "broken row skipped, valid rows kept");
        assert_eq!(customs[0].name, "aserver");
        assert_eq!(customs[0].env_key, "");
        assert_eq!(customs[1].name, "zserver");
        assert_eq!(customs[1].env_key, "Z_KEY");
        assert_eq!(customs[1].base_url, "http://z.example/v1");

        // Other config keys survive untouched (we only read).
        let value: toml::Value =
            toml::from_str(&std::fs::read_to_string(tmp.path().join("config.toml")).unwrap())
                .unwrap();
        assert!(value.get("picker").is_some(), "[picker] still present");
    }

    #[test]
    fn missing_file_reads_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_custom_providers_in(tmp.path()).is_empty());
    }
}
