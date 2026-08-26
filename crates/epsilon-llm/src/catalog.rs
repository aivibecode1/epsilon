//! Static model catalog with explicit capability + wire-routing metadata.
//!
//! Port of pi's generated-catalog approach (pi-0.84.2 `packages/ai/src/providers/`):
//! a bundled snapshot of the models.dev registry so browsing needs no network, with
//! each row carrying its wire API (`ModelInfo::wire`) the way pi's models carry
//! `model.api`.
//!
//! The rows themselves live in [`catalog_data`] — **generated** by
//! `scripts/regen_catalog.py` from a live fetch of <https://models.dev/api.json>
//! (DIRECTION-05 closure). Never edit that file by hand: refresh it with
//! `python scripts/regen_catalog.py`, run the gates, fix any moved pins in
//! `catalog_is_current_generation`, and commit script + data together.
//!
//! Binding property since M3: behavior selection reads *capability fields*, never id
//! substrings (anti-pattern from opencode, see comparative-analysis §4). Routing reads
//! `wire`; nothing string-matches model families.

mod catalog_data;

/// Which wire format + endpoint family a model speaks (catalog-driven routing,
/// pi's `model.api` field). Catalog data by design: present in every build, but only
/// `real` builds ever construct a transport from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireApi {
    /// Anthropic Messages (`/v1/messages`, x-api-key).
    AnthropicMessages,
    /// OpenAI-compatible `/chat/completions` (OpenAI, DeepSeek, Groq, xAI, ...).
    OpenAiCompletions {
        /// Provider base URL without trailing slash.
        base_url: &'static str,
        /// Auth env var name (`None`: local Ollama needs no key).
        env_key: Option<&'static str>,
    },
    /// Google Generative Language (`:streamGenerateContent?alt=sse`).
    GoogleGenerativeAi,
}

const ANTHROPIC_WIRE: WireApi = WireApi::AnthropicMessages;
const GOOGLE_WIRE: WireApi = WireApi::GoogleGenerativeAi;
const OLLAMA_WIRE: WireApi = openai_compat("http://localhost:11434/v1", None);
const FAUX_WIRE: WireApi = OLLAMA_WIRE;

/// OpenAI-compatible shorthand (pi's "same API, different endpoint/key" providers).
const fn openai_compat(base_url: &'static str, env_key: Option<&'static str>) -> WireApi {
    WireApi::OpenAiCompletions { base_url, env_key }
}

/// Static description of one model.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Catalog id = the provider-native model name sent on the wire.
    pub id: &'static str,
    /// Provider identifier (picker grouping + auth family).
    pub provider: &'static str,
    /// Human display name for pickers.
    pub label: &'static str,
    /// Context window in tokens.
    pub context_window: u64,
    /// Maximum output tokens.
    pub max_output_tokens: u64,
    /// Input price, micro-USD per million tokens (models.dev list price).
    pub input_cost_micro_per_mtok: u64,
    /// Output price, micro-USD per million tokens (models.dev list price).
    pub output_cost_micro_per_mtok: u64,
    /// Whether tool/function calling works.
    pub supports_tools: bool,
    /// Env var holding this provider's API key (empty string = none needed).
    pub env_key: &'static str,
    /// Wire format + endpoint family this model speaks (pi's `model.api`).
    pub wire: WireApi,
}

use catalog_data::CATALOG_ROWS as MODELS;

/// Bundled catalog slice (picker/browse source; live fetch merges on top).
///
/// Generated from models.dev by `scripts/regen_catalog.py`; includes the offline
/// reference rows (faux) and local ollama rows alongside every current-generation
/// chat/tool model on the nine direct-API providers.
pub fn curated() -> &'static [ModelInfo] {
    MODELS
}

/// Look up a model by exact catalog id.
pub fn lookup_model(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

// ---- custom OpenAI-compatible providers (M28; opencode `provider.options.baseURL`,
//      Hermes `base_url_override`) ----------------------------------------------

/// One user-declared OpenAI-compatible endpoint (`[provider_custom.<name>]`
/// in the global config: `base_url` required, `env_key` optional). The model
/// id `<name>/<model>` streams to `<base_url>/chat/completions` with the key
/// resolved for provider `<name>` (stored auth.json entry wins, then
/// `env_key`). Lives in this crate because wire routing consumes it; the TOML
/// parsing stays with config in epsilon-core.
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

/// Display label for an arbitrary id: catalog label when known, else the id itself
/// (custom entries always render as typed).
pub fn display_label(id: &str) -> &str {
    lookup_model(id).map(|m| m.label).unwrap_or(id)
}

/// Whether the provider credential for `model_id` is present in the environment.
///
/// Models with an empty [`ModelInfo::env_key`] (faux/local) are always ready. Unknown
/// ids conservatively report `false` — the picker shows them under custom entries where
/// the user has already taken responsibility for auth.
///
/// M22: readiness consults [`resolve_api_key`], so a key saved by
/// `epsilon auth login` counts even when no env var is set (stored wins, env
/// fallback — opencode's model).
pub fn credentials_ready(model_id: &str) -> bool {
    match lookup_model(model_id) {
        Some(info) => info.env_key.is_empty() || resolve_api_key(info.provider).is_some(),
        None => false,
    }
}

/// The env var name a custom picker row should instruct the user to set, if known.
pub fn env_key_for(model_id: &str) -> Option<&'static str> {
    lookup_model(model_id)
        .map(|m| m.env_key)
        .filter(|k| !k.is_empty())
}

/// Whether the provider exposes a live `GET /models` listing at all.
///
/// Static knowledge (no network, no feature gate) so UIs can decide whether a
/// live fetch is even worth attempting before injecting a `real`-feature fetcher.
pub fn live_listing_available(provider: &str) -> bool {
    listing_endpoint(provider).is_some()
}

/// Resolve the API key for a provider: stored credential first
/// (`~/.config/epsilon/auth.json`, opencode's auth pattern), then the
/// provider's env var. Returns `None` when neither is present.
///
/// Stored-wins matters because `epsilon auth login` is the documented path;
/// env vars stay as the fallback for CI and users who prefer them. An empty
/// stored key is treated as absent.
pub fn resolve_api_key(provider: &str) -> Option<String> {
    resolve_api_key_in(&crate::auth::global_dir()?, provider, &|name| {
        std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
    })
}

/// The precedence core, with the store directory and env lookup injected so
/// tests can pin stored-wins/env-fallback without touching `$HOME`.
fn resolve_api_key_in(
    dir: &std::path::Path,
    provider: &str,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if let Some(cred) = crate::auth::load_credentials_in(dir).get(provider) {
        let k = cred.key.trim();
        if !k.is_empty() {
            return Some(k.to_string());
        }
    }
    let env_key = lookup_model_first_env(provider)?;
    env_lookup(env_key).filter(|v| !v.is_empty())
}

/// The canonical env var of a provider, taken from any catalog row of that
/// provider (rows share it by construction — see `wire_auth_matches_row_auth`).
fn lookup_model_first_env(provider: &str) -> Option<&'static str> {
    MODELS
        .iter()
        .find(|m| m.provider == provider && !m.env_key.is_empty())
        .map(|m| m.env_key)
}

/// (endpoint URL, auth env var) for a provider's live model listing.
/// Pure static routing data — the network call itself lives in [`live_models`].
#[cfg(feature = "real")]
fn listing_endpoint(provider: &str) -> Option<(&'static str, &'static str)> {
    match provider {
        "anthropic" => Some(("https://api.anthropic.com/v1/models", "ANTHROPIC_API_KEY")),
        "openai" => Some(("https://api.openai.com/v1/models", "OPENAI_API_KEY")),
        "deepseek" => Some(("https://api.deepseek.com/v1/models", "DEEPSEEK_API_KEY")),
        "google" => Some((
            "https://generativelanguage.googleapis.com/v1beta/models",
            "GEMINI_API_KEY",
        )),
        "xai" => Some(("https://api.x.ai/v1/models", "XAI_API_KEY")),
        "groq" => Some(("https://api.groq.com/openai/v1/models", "GROQ_API_KEY")),
        "moonshot" => Some(("https://api.moonshot.ai/v1/models", "MOONSHOT_API_KEY")),
        "zai" => Some(("https://api.z.ai/api/paas/v4/models", "ZAI_API_KEY")),
        "openrouter" => Some(("https://openrouter.ai/api/v1/models", "OPENROUTER_API_KEY")),
        _ => None,
    }
}

#[cfg(not(feature = "real"))]
fn listing_endpoint(_provider: &str) -> Option<(&'static str, &'static str)> {
    None
}

/// List a provider's live model ids via plain authenticated GET `/models`
/// (`real` feature only) — pi/opencode convention for the OpenAI-compatible
/// listing endpoint. Anthropic and Google expose the same shape at their own
/// base URLs. Errors arrive in-band as strings.
#[cfg(feature = "real")]
pub async fn live_models(provider: &str) -> Result<Vec<String>, String> {
    let (url, _env_key) = listing_endpoint(provider)
        .ok_or_else(|| format!("no live listing endpoint for provider '{provider}'"))?;
    // M22: stored credential wins, env var falls back (same precedence as
    // request auth in DirectProvider).
    let key = resolve_api_key(provider).ok_or_else(|| {
        format!(
            "no API key for '{provider}' (run `epsilon auth login {provider}` or set {_env_key})"
        )
    })?;
    let client = crate::http::http_client();
    let mut req = client.get(url);
    match provider {
        "anthropic" => {
            req = req
                .header("x-api-key", &key)
                .header("anthropic-version", "2023-06-01");
        }
        "google" => {
            req = req.header("x-goog-api-key", &key);
        }
        _ => {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("listing failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("bad listing payload: {e}"))?;
    let ids: Vec<String> = body
        .pointer("/data")
        .and_then(|d| d.as_array())
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
        .map(str::to_string)
        .collect();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_api_key_stored_wins_then_env_fallback() {
        // Injected dir + env lookup: no $HOME touched, no env::set_var needed
        // (that's forbidden under unsafe_code=forbid in edition 2024).
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let env = |want: &'static str| {
            move |name: &str| {
                if name == want {
                    Some("env-key".to_string())
                } else {
                    None
                }
            }
        };

        // Neither present -> None.
        let none: Option<String> = None;
        assert_eq!(
            resolve_api_key_in(dir, "anthropic", &|_| none.clone()),
            None,
            "no store, no env"
        );

        // Env only -> env value.
        assert_eq!(
            resolve_api_key_in(dir, "anthropic", &env("ANTHROPIC_API_KEY")),
            Some("env-key".into())
        );

        // Stored beats env.
        crate::auth::save_credential_in(dir, "anthropic", "stored-key").expect("save");
        assert_eq!(
            resolve_api_key_in(dir, "anthropic", &env("ANTHROPIC_API_KEY")),
            Some("stored-key".into()),
            "stored credential must win over the env var"
        );

        // Blank stored key falls back to env (trim-then-check contract). The
        // store refuses blank keys, so simulate a legacy/corrupt file directly.
        std::fs::write(
            crate::auth::auth_path_in(dir),
            serde_json::json!({ "anthropic": { "type": "api", "key": "   " } }).to_string(),
        )
        .expect("write legacy blank key");
        assert_eq!(
            resolve_api_key_in(dir, "anthropic", &env("ANTHROPIC_API_KEY")),
            Some("env-key".into())
        );
    }

    #[test]
    fn lookup_is_exact_match() {
        assert!(lookup_model("claude-sonnet-4-5").is_some());
        assert!(
            lookup_model("Claude-Sonnet-4-5").is_none(),
            "no case-insensitive guessing"
        );
        assert!(lookup_model("gpt").is_none(), "no substring matching");
    }

    #[test]
    fn catalog_groups_and_covers_promised_providers() {
        let mut providers: Vec<&str> = MODELS.iter().map(|m| m.provider).collect();
        providers.sort_unstable();
        providers.dedup();
        for expected in [
            "faux",
            "anthropic",
            "openai",
            "deepseek",
            "google",
            "xai",
            "groq",
            "moonshot",
            "zai",
            "openrouter",
            "ollama",
        ] {
            assert!(
                providers.contains(&expected),
                "provider {expected} missing from bundled catalog"
            );
        }
    }

    #[test]
    fn every_entry_has_label_unique_id_and_wire() {
        let mut ids: Vec<&str> = MODELS.iter().map(|m| m.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "catalog ids must be unique");
        for m in MODELS {
            assert!(!m.label.is_empty());
            assert!(m.context_window > 0);
            let _ = m.wire; // Copy enum: presence is structural, no Option involved
        }
    }

    /// Every cloud row routes through its provider's own credentials — the wire's
    /// env var must agree with the row's env_key (regression guard for the M17
    /// direct-HTTP port; genai-era OPENROUTER/OPEN_ROUTER mismatch class).
    #[test]
    fn wire_auth_matches_row_auth() {
        for m in MODELS {
            if m.provider == "faux" || m.provider == "ollama" {
                continue;
            }
            if let WireApi::OpenAiCompletions { env_key, .. } = &m.wire {
                assert_eq!(*env_key, Some(m.env_key), "{}", m.id);
            }
        }
    }

    /// Prices come from the models.dev snapshot: local/offline rows are exactly
    /// zero, and every cloud provider carries at least one really-priced row —
    /// proving real registry data flowed in (M3 all-placeholder regression guard).
    /// Individual unpriced rows are allowed through as-is (e.g. google gemma
    /// previews have no cost in the registry); we render what upstream states,
    /// never invented rates.
    #[test]
    fn prices_are_real_registry_rates_or_zero_for_local() {
        let mut paid_providers = std::collections::BTreeSet::new();
        for m in MODELS {
            if m.provider == "faux" || m.provider == "ollama" {
                assert_eq!(m.input_cost_micro_per_mtok, 0, "{}", m.id);
                assert_eq!(m.output_cost_micro_per_mtok, 0, "{}", m.id);
            } else if m.input_cost_micro_per_mtok > 0 || m.output_cost_micro_per_mtok > 0 {
                paid_providers.insert(m.provider);
            }
        }
        for p in [
            "anthropic",
            "openai",
            "deepseek",
            "google",
            "xai",
            "groq",
            "moonshot",
            "zai",
            "openrouter",
        ] {
            assert!(
                paid_providers.contains(p),
                "provider {p} has no priced row - snapshot looks placeholder"
            );
        }
    }

    /// Current-generation sanity pins: regeneration must not regress to older eras.
    /// Updated whenever `scripts/regen_catalog.py` pulls a newer snapshot
    /// (2026-08-24 snapshot: 383 rows across 9 direct-API providers).
    #[test]
    fn catalog_is_current_generation() {
        // Newest flagship of each direct-API family at regen time:
        assert!(lookup_model("claude-opus-5").is_some());
        assert!(lookup_model("claude-fable-5").is_some());
        assert!(lookup_model("gpt-5.6").is_some());
        assert!(lookup_model("gemini-3.7-flash").is_some());
        assert!(lookup_model("grok-4.6").is_some());
        assert!(lookup_model("kimi-k3").is_some());
        assert!(lookup_model("glm-5.2").is_some());
        assert!(lookup_model("deepseek-v4-pro").is_some());
        // Long-gone rows must stay gone...
        assert!(lookup_model("claude-sonnet-4").is_none(), "stale row");
        // ...but anything the registry still lists belongs in the snapshot, even
        // legacy workhorses the old hand-curated table skipped (registry-faithful,
        // not taste-curated):
        assert!(lookup_model("gpt-4o").is_some());
        // Dated snapshots are excluded in favor of canonical aliases:
        assert!(
            lookup_model("claude-haiku-4-5-20251001").is_none(),
            "dated snapshot row leaked into the catalog"
        );
    }

    #[test]
    fn local_models_need_no_credentials() {
        assert!(credentials_ready("faux-1"));
        assert!(credentials_ready("llama3.1:8b"));
        assert!(
            !credentials_ready("claude-sonnet-4-5"),
            "test env has no keys"
        );
        assert_eq!(env_key_for("claude-sonnet-4-5"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(env_key_for("kimi-k2.6"), Some("MOONSHOT_API_KEY"));
        assert_eq!(env_key_for("faux-1"), None);
    }

    #[test]
    fn display_label_falls_back_to_id_for_custom_entries() {
        // Label comes verbatim from the models.dev `name` field.
        assert_eq!(display_label("deepseek-chat"), "DeepSeek Chat");
        assert_eq!(
            display_label("totally-custom-model"),
            "totally-custom-model"
        );
    }
}
