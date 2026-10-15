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
//!
//! **M225: this crate no longer parses it.** The reader is `core::config::custom_providers_in`,
//! because `epsilon-core` depends on this crate and not the other way round, and the rows are
//! installed here by the composition root. `toml` left this crate's manifest with that move — the
//! rule it enforced (one reader per file) is worth more than the convenience of parsing in place.

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

/// The custom providers this process was given, read by everyone who needs them.
///
/// **M225: the value is installed once, and every reader reads it.** `load_custom_providers` used to
/// open and parse `config.toml` on *every* call — and its callers are `DirectProvider::stream`, the
/// fallback chain's key resolution and `try_stream_direct`, so a config file was read on the hot path
/// of every request, by a crate that is not allowed to own the parse.
///
/// The alternative shape (threading `&[CustomProvider]` through) was measured before choosing this:
/// the readers sit behind the `StreamFn` trait and inside the fallback chain, so the value would have
/// to cross the trait boundary into every stream call — configuration in the middle of the hot path.
/// So this is **composition state, not resolution**: the process that owns configuration installs the
/// rows once, before anything runs, and the readers ask for them.
static INSTALLED: std::sync::OnceLock<std::sync::RwLock<Vec<CustomProvider>>> =
    std::sync::OnceLock::new();

/// Install the custom providers for this process, replacing any previous set.
///
/// Called once by the composition root (M225). It *replaces* rather than appends, so a second call
/// from a test observes the second set and never a union no configuration asked for.
pub fn install_providers(rows: Vec<CustomProvider>) {
    let slot = INSTALLED.get_or_init(|| std::sync::RwLock::new(Vec::new()));
    if let Ok(mut current) = slot.write() {
        *current = rows;
    }
}

/// The installed custom providers, or an empty set when nothing was installed.
///
/// A clone per call: the readers are on the request path, and handing out a borrow of shared state
/// would tie every caller's lifetime to a lock this crate does not own.
#[must_use]
pub fn custom_providers() -> Vec<CustomProvider> {
    INSTALLED
        .get()
        .and_then(|slot| slot.read().ok().map(|rows| rows.clone()))
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

    /// M225: what was installed is what every reader gets, and a second install replaces the first.
    ///
    /// **Kept here because this is the crate's contract**, even though nothing installs in a unit test
    /// that has not called `install_providers` itself: the readers that matter are on the request path,
    /// and the question they are asking is "was the process configured, and with what".
    #[test]
    fn installing_replaces_the_previous_set() {
        let row = |name: &str| CustomProvider::from_config(name, "http://x/v1", "").unwrap();
        install_providers(vec![row("first")]);
        assert_eq!(
            custom_providers()
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["first"]
        );

        // A second install is a *set*, not an append: a test that installs twice must not see a union
        // that no configuration ever asked for.
        install_providers(vec![row("second")]);
        assert_eq!(
            custom_providers()
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["second"],
            "replaced, not merged"
        );
    }
}
