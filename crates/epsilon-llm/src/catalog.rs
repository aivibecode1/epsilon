//! Static model catalog with explicit capability + wire-routing metadata.
//!
//! Port of pi's generated-catalog approach (pi-0.84.2 `packages/ai/src/providers/`):
//! a bundled snapshot of the models.dev registry so browsing needs no network, with
//! each row carrying its wire API (`ModelInfo::wire`) the way pi's models carry
//! `model.api`. Regenerated 2026-08 from a live fetch of <https://models.dev/api.json>;
//! prices are that registry's REAL published rates (USD per MTok × 1e6), replacing
//! the M3 placeholders — the footer cost line is an estimate against public list
//! prices, not a billing feed.
//!
//! Binding property since M3: behavior selection reads *capability fields*, never id
//! substrings (anti-pattern from opencode, see comparative-analysis §4). Routing reads
//! `wire`; nothing string-matches model families.

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

const ANTHROPIC_API: WireApi = WireApi::AnthropicMessages;
const GOOGLE_API: WireApi = WireApi::GoogleGenerativeAi;

/// OpenAI-compatible shorthand (pi's "same API, different endpoint/key" providers).
const fn openai_compat(base_url: &'static str, env_key: Option<&'static str>) -> WireApi {
    WireApi::OpenAiCompletions { base_url, env_key }
}

/// Known models, grouped by provider in display order. Curated subset of the
/// 2026-08 models.dev snapshot: current flagship/workhorse/cheap tiers per provider,
/// tool-calling chat models only (image/TTS/embedding rows omitted — this harness
/// streams text+tools).
pub const MODELS: &[ModelInfo] = &[
    // ---- offline reference -----------------------------------------------------------
    ModelInfo {
        id: "faux-1",
        provider: "faux",
        label: "Faux (offline scripted)",
        context_window: 1_000_000,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 0,
        output_cost_micro_per_mtok: 0,
        supports_tools: true,
        env_key: "",
        wire: openai_compat("http://localhost:9", None), // never dialed; faux is offline
    },
    // ---- anthropic --------------------------------------------------------------------
    ModelInfo {
        id: "claude-opus-4-5",
        provider: "anthropic",
        label: "Claude Opus 4.5",
        context_window: 200_000,
        max_output_tokens: 64_000,
        input_cost_micro_per_mtok: 5_000_000,
        output_cost_micro_per_mtok: 25_000_000,
        supports_tools: true,
        env_key: "ANTHROPIC_API_KEY",
        wire: ANTHROPIC_API,
    },
    ModelInfo {
        id: "claude-sonnet-4-5",
        provider: "anthropic",
        label: "Claude Sonnet 4.5",
        context_window: 1_000_000,
        max_output_tokens: 64_000,
        input_cost_micro_per_mtok: 3_000_000,
        output_cost_micro_per_mtok: 15_000_000,
        supports_tools: true,
        env_key: "ANTHROPIC_API_KEY",
        wire: ANTHROPIC_API,
    },
    ModelInfo {
        id: "claude-haiku-4-5",
        provider: "anthropic",
        label: "Claude Haiku 4.5",
        context_window: 200_000,
        max_output_tokens: 64_000,
        input_cost_micro_per_mtok: 1_000_000,
        output_cost_micro_per_mtok: 5_000_000,
        supports_tools: true,
        env_key: "ANTHROPIC_API_KEY",
        wire: ANTHROPIC_API,
    },
    ModelInfo {
        id: "claude-opus-4-6",
        provider: "anthropic",
        label: "Claude Opus 4.6",
        context_window: 1_000_000,
        max_output_tokens: 128_000,
        input_cost_micro_per_mtok: 5_000_000,
        output_cost_micro_per_mtok: 25_000_000,
        supports_tools: true,
        env_key: "ANTHROPIC_API_KEY",
        wire: ANTHROPIC_API,
    },
    ModelInfo {
        id: "claude-sonnet-4-6",
        provider: "anthropic",
        label: "Claude Sonnet 4.6",
        context_window: 1_000_000,
        max_output_tokens: 128_000,
        input_cost_micro_per_mtok: 3_000_000,
        output_cost_micro_per_mtok: 15_000_000,
        supports_tools: true,
        env_key: "ANTHROPIC_API_KEY",
        wire: ANTHROPIC_API,
    },
    ModelInfo {
        id: "claude-opus-5",
        provider: "anthropic",
        label: "Claude Opus 5",
        context_window: 1_000_000,
        max_output_tokens: 128_000,
        input_cost_micro_per_mtok: 5_000_000,
        output_cost_micro_per_mtok: 25_000_000,
        supports_tools: true,
        env_key: "ANTHROPIC_API_KEY",
        wire: ANTHROPIC_API,
    },
    ModelInfo {
        id: "claude-sonnet-5",
        provider: "anthropic",
        label: "Claude Sonnet 5",
        context_window: 1_000_000,
        max_output_tokens: 128_000,
        input_cost_micro_per_mtok: 2_000_000,
        output_cost_micro_per_mtok: 10_000_000,
        supports_tools: true,
        env_key: "ANTHROPIC_API_KEY",
        wire: ANTHROPIC_API,
    },
    // ---- openai -----------------------------------------------------------------------
    ModelInfo {
        id: "gpt-5.2",
        provider: "openai",
        label: "GPT-5.2",
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_micro_per_mtok: 1_750_000,
        output_cost_micro_per_mtok: 14_000_000,
        supports_tools: true,
        env_key: "OPENAI_API_KEY",
        wire: openai_compat("https://api.openai.com/v1", Some("OPENAI_API_KEY")),
    },
    ModelInfo {
        id: "gpt-5.3-codex",
        provider: "openai",
        label: "GPT-5.3 Codex",
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_micro_per_mtok: 1_750_000,
        output_cost_micro_per_mtok: 14_000_000,
        supports_tools: true,
        env_key: "OPENAI_API_KEY",
        wire: openai_compat("https://api.openai.com/v1", Some("OPENAI_API_KEY")),
    },
    ModelInfo {
        id: "gpt-5.4",
        provider: "openai",
        label: "GPT-5.4",
        context_window: 1_050_000,
        max_output_tokens: 128_000,
        input_cost_micro_per_mtok: 2_500_000,
        output_cost_micro_per_mtok: 15_000_000,
        supports_tools: true,
        env_key: "OPENAI_API_KEY",
        wire: openai_compat("https://api.openai.com/v1", Some("OPENAI_API_KEY")),
    },
    ModelInfo {
        id: "gpt-5.4-mini",
        provider: "openai",
        label: "GPT-5.4 mini",
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_micro_per_mtok: 750_000,
        output_cost_micro_per_mtok: 4_500_000,
        supports_tools: true,
        env_key: "OPENAI_API_KEY",
        wire: openai_compat("https://api.openai.com/v1", Some("OPENAI_API_KEY")),
    },
    ModelInfo {
        id: "gpt-5.4-nano",
        provider: "openai",
        label: "GPT-5.4 nano",
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_micro_per_mtok: 200_000,
        output_cost_micro_per_mtok: 1_250_000,
        supports_tools: true,
        env_key: "OPENAI_API_KEY",
        wire: openai_compat("https://api.openai.com/v1", Some("OPENAI_API_KEY")),
    },
    ModelInfo {
        id: "gpt-5-mini",
        provider: "openai",
        label: "GPT-5 mini",
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_micro_per_mtok: 250_000,
        output_cost_micro_per_mtok: 2_000_000,
        supports_tools: true,
        env_key: "OPENAI_API_KEY",
        wire: openai_compat("https://api.openai.com/v1", Some("OPENAI_API_KEY")),
    },
    // ---- deepseek ---------------------------------------------------------------------
    ModelInfo {
        id: "deepseek-chat",
        provider: "deepseek",
        label: "DeepSeek Chat (V4)",
        context_window: 1_000_000,
        max_output_tokens: 384_000,
        input_cost_micro_per_mtok: 140_000,
        output_cost_micro_per_mtok: 280_000,
        supports_tools: true,
        env_key: "DEEPSEEK_API_KEY",
        wire: openai_compat("https://api.deepseek.com/v1", Some("DEEPSEEK_API_KEY")),
    },
    ModelInfo {
        id: "deepseek-reasoner",
        provider: "deepseek",
        label: "DeepSeek Reasoner",
        context_window: 1_000_000,
        max_output_tokens: 384_000,
        input_cost_micro_per_mtok: 140_000,
        output_cost_micro_per_mtok: 280_000,
        supports_tools: true,
        env_key: "DEEPSEEK_API_KEY",
        wire: openai_compat("https://api.deepseek.com/v1", Some("DEEPSEEK_API_KEY")),
    },
    ModelInfo {
        id: "deepseek-v4-pro",
        provider: "deepseek",
        label: "DeepSeek V4 Pro",
        context_window: 1_000_000,
        max_output_tokens: 384_000,
        input_cost_micro_per_mtok: 435_000,
        output_cost_micro_per_mtok: 870_000,
        supports_tools: true,
        env_key: "DEEPSEEK_API_KEY",
        wire: openai_compat("https://api.deepseek.com/v1", Some("DEEPSEEK_API_KEY")),
    },
    // ---- google -----------------------------------------------------------------------
    ModelInfo {
        id: "gemini-2.5-pro",
        provider: "google",
        label: "Gemini 2.5 Pro",
        context_window: 1_048_576,
        max_output_tokens: 65_536,
        input_cost_micro_per_mtok: 1_250_000,
        output_cost_micro_per_mtok: 10_000_000,
        supports_tools: true,
        env_key: "GEMINI_API_KEY",
        wire: GOOGLE_API,
    },
    ModelInfo {
        id: "gemini-2.5-flash",
        provider: "google",
        label: "Gemini 2.5 Flash",
        context_window: 1_048_576,
        max_output_tokens: 65_536,
        input_cost_micro_per_mtok: 300_000,
        output_cost_micro_per_mtok: 2_500_000,
        supports_tools: true,
        env_key: "GEMINI_API_KEY",
        wire: GOOGLE_API,
    },
    ModelInfo {
        id: "gemini-3-flash-preview",
        provider: "google",
        label: "Gemini 3 Flash (preview)",
        context_window: 1_048_576,
        max_output_tokens: 65_536,
        input_cost_micro_per_mtok: 500_000,
        output_cost_micro_per_mtok: 3_000_000,
        supports_tools: true,
        env_key: "GEMINI_API_KEY",
        wire: GOOGLE_API,
    },
    ModelInfo {
        id: "gemini-3.1-pro-preview",
        provider: "google",
        label: "Gemini 3.1 Pro (preview)",
        context_window: 1_048_576,
        max_output_tokens: 65_536,
        input_cost_micro_per_mtok: 2_000_000,
        output_cost_micro_per_mtok: 12_000_000,
        supports_tools: true,
        env_key: "GEMINI_API_KEY",
        wire: GOOGLE_API,
    },
    ModelInfo {
        id: "gemini-3.6-flash",
        provider: "google",
        label: "Gemini 3.6 Flash",
        context_window: 1_048_576,
        max_output_tokens: 65_536,
        input_cost_micro_per_mtok: 750_000,
        output_cost_micro_per_mtok: 3_750_000,
        supports_tools: true,
        env_key: "GEMINI_API_KEY",
        wire: GOOGLE_API,
    },
    // ---- xai --------------------------------------------------------------------------
    ModelInfo {
        id: "grok-4.3",
        provider: "xai",
        label: "Grok 4.3",
        context_window: 1_000_000,
        max_output_tokens: 30_000,
        input_cost_micro_per_mtok: 1_250_000,
        output_cost_micro_per_mtok: 2_500_000,
        supports_tools: true,
        env_key: "XAI_API_KEY",
        wire: openai_compat("https://api.x.ai/v1", Some("XAI_API_KEY")),
    },
    ModelInfo {
        id: "grok-4.5",
        provider: "xai",
        label: "Grok 4.5",
        context_window: 500_000,
        max_output_tokens: 500_000,
        input_cost_micro_per_mtok: 2_000_000,
        output_cost_micro_per_mtok: 6_000_000,
        supports_tools: true,
        env_key: "XAI_API_KEY",
        wire: openai_compat("https://api.x.ai/v1", Some("XAI_API_KEY")),
    },
    ModelInfo {
        id: "grok-4.6",
        provider: "xai",
        label: "Grok 4.6",
        context_window: 500_000,
        max_output_tokens: 500_000,
        input_cost_micro_per_mtok: 2_000_000,
        output_cost_micro_per_mtok: 6_000_000,
        supports_tools: true,
        env_key: "XAI_API_KEY",
        wire: openai_compat("https://api.x.ai/v1", Some("XAI_API_KEY")),
    },
    // ---- groq -------------------------------------------------------------------------
    ModelInfo {
        id: "llama-3.3-70b-versatile",
        provider: "groq",
        label: "Llama 3.3 70B (Groq)",
        context_window: 131_072,
        max_output_tokens: 32_768,
        input_cost_micro_per_mtok: 590_000,
        output_cost_micro_per_mtok: 790_000,
        supports_tools: true,
        env_key: "GROQ_API_KEY",
        wire: openai_compat("https://api.groq.com/openai/v1", Some("GROQ_API_KEY")),
    },
    ModelInfo {
        id: "openai/gpt-oss-120b",
        provider: "groq",
        label: "GPT-OSS 120B (Groq)",
        context_window: 131_072,
        max_output_tokens: 65_536,
        input_cost_micro_per_mtok: 150_000,
        output_cost_micro_per_mtok: 600_000,
        supports_tools: true,
        env_key: "GROQ_API_KEY",
        wire: openai_compat("https://api.groq.com/openai/v1", Some("GROQ_API_KEY")),
    },
    // ---- moonshot ---------------------------------------------------------------------
    ModelInfo {
        id: "kimi-k2-thinking",
        provider: "moonshot",
        label: "Kimi K2 Thinking",
        context_window: 262_144,
        max_output_tokens: 262_144,
        input_cost_micro_per_mtok: 600_000,
        output_cost_micro_per_mtok: 2_500_000,
        supports_tools: true,
        env_key: "MOONSHOT_API_KEY",
        wire: openai_compat("https://api.moonshot.ai/v1", Some("MOONSHOT_API_KEY")),
    },
    ModelInfo {
        id: "kimi-k2.6",
        provider: "moonshot",
        label: "Kimi K2.6",
        context_window: 262_144,
        max_output_tokens: 262_144,
        input_cost_micro_per_mtok: 950_000,
        output_cost_micro_per_mtok: 4_000_000,
        supports_tools: true,
        env_key: "MOONSHOT_API_KEY",
        wire: openai_compat("https://api.moonshot.ai/v1", Some("MOONSHOT_API_KEY")),
    },
    ModelInfo {
        id: "kimi-k3",
        provider: "moonshot",
        label: "Kimi K3",
        context_window: 1_048_576,
        max_output_tokens: 131_072,
        input_cost_micro_per_mtok: 3_000_000,
        output_cost_micro_per_mtok: 15_000_000,
        supports_tools: true,
        env_key: "MOONSHOT_API_KEY",
        wire: openai_compat("https://api.moonshot.ai/v1", Some("MOONSHOT_API_KEY")),
    },
    // ---- z.ai -------------------------------------------------------------------------
    ModelInfo {
        id: "glm-4.7",
        provider: "zai",
        label: "GLM-4.7",
        context_window: 204_800,
        max_output_tokens: 131_072,
        input_cost_micro_per_mtok: 600_000,
        output_cost_micro_per_mtok: 2_200_000,
        supports_tools: true,
        env_key: "ZAI_API_KEY",
        wire: openai_compat("https://api.z.ai/api/paas/v4", Some("ZAI_API_KEY")),
    },
    ModelInfo {
        id: "glm-5",
        provider: "zai",
        label: "GLM-5",
        context_window: 204_800,
        max_output_tokens: 131_072,
        input_cost_micro_per_mtok: 1_000_000,
        output_cost_micro_per_mtok: 3_200_000,
        supports_tools: true,
        env_key: "ZAI_API_KEY",
        wire: openai_compat("https://api.z.ai/api/paas/v4", Some("ZAI_API_KEY")),
    },
    // ---- openrouter ---------------------------------------------------------------------
    ModelInfo {
        id: "anthropic/claude-sonnet-4.5",
        provider: "openrouter",
        label: "Claude Sonnet 4.5 (via OpenRouter)",
        context_window: 1_000_000,
        max_output_tokens: 64_000,
        input_cost_micro_per_mtok: 3_000_000,
        output_cost_micro_per_mtok: 15_000_000,
        supports_tools: true,
        env_key: "OPENROUTER_API_KEY",
        wire: openai_compat("https://openrouter.ai/api/v1", Some("OPENROUTER_API_KEY")),
    },
    // ---- ollama (local, no key) ---------------------------------------------------------
    ModelInfo {
        id: "llama3.1:8b",
        provider: "ollama",
        label: "Llama 3.1 8B (local)",
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 0,
        output_cost_micro_per_mtok: 0,
        supports_tools: false,
        env_key: "",
        wire: openai_compat("http://localhost:11434/v1", None),
    },
    ModelInfo {
        id: "qwen3:14b",
        provider: "ollama",
        label: "Qwen 3 14B (local)",
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 0,
        output_cost_micro_per_mtok: 0,
        supports_tools: false,
        env_key: "",
        wire: openai_compat("http://localhost:11434/v1", None),
    },
];

/// Bundled catalog slice (picker/browse source; live fetch via Ctrl+F merges on top).
pub fn curated() -> &'static [ModelInfo] {
    MODELS
}

/// Look up a model by exact catalog id.
pub fn lookup_model(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
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
pub fn credentials_ready(model_id: &str) -> bool {
    match lookup_model(model_id) {
        Some(info) => info.env_key.is_empty() || std::env::var_os(info.env_key).is_some(),
        None => false,
    }
}

/// The env var name a custom picker row should instruct the user to set, if known.
pub fn env_key_for(model_id: &str) -> Option<&'static str> {
    lookup_model(model_id)
        .map(|m| m.env_key)
        .filter(|k| !k.is_empty())
}

/// List a provider's live model ids via plain authenticated GET `/models`
/// (`real` feature only) — pi/opencode convention for the OpenAI-compatible
/// listing endpoint. Anthropic and Google expose the same shape at their own
/// base URLs. Errors arrive in-band as strings.
#[cfg(feature = "real")]
pub async fn live_models(provider: &str) -> Result<Vec<String>, String> {
    let (url, env_key): (String, &str) = match provider {
        "anthropic" => (
            "https://api.anthropic.com/v1/models".into(),
            "ANTHROPIC_API_KEY",
        ),
        "openai" => ("https://api.openai.com/v1/models".into(), "OPENAI_API_KEY"),
        "deepseek" => (
            "https://api.deepseek.com/v1/models".into(),
            "DEEPSEEK_API_KEY",
        ),
        "google" => (
            "https://generativelanguage.googleapis.com/v1beta/models".into(),
            "GEMINI_API_KEY",
        ),
        "xai" => ("https://api.x.ai/v1/models".into(), "XAI_API_KEY"),
        "groq" => (
            "https://api.groq.com/openai/v1/models".into(),
            "GROQ_API_KEY",
        ),
        "moonshot" => (
            "https://api.moonshot.ai/v1/models".into(),
            "MOONSHOT_API_KEY",
        ),
        "zai" => ("https://api.z.ai/api/paas/v4/models".into(), "ZAI_API_KEY"),
        "openrouter" => (
            "https://openrouter.ai/api/v1/models".into(),
            "OPENROUTER_API_KEY",
        ),
        other => return Err(format!("no live listing endpoint for provider '{other}'")),
    };
    let key = std::env::var(env_key).map_err(|_| format!("missing {env_key}"))?;
    let client = crate::http::http_client();
    let mut req = client.get(&url);
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

    /// Prices come from the models.dev snapshot: paid providers carry real rates,
    /// local/offline rows are exactly zero. Guards against placeholder regression.
    #[test]
    fn prices_are_real_registry_rates_or_zero_for_local() {
        for m in MODELS {
            if m.provider == "faux" || m.provider == "ollama" {
                assert_eq!(m.input_cost_micro_per_mtok, 0, "{}", m.id);
                assert_eq!(m.output_cost_micro_per_mtok, 0, "{}", m.id);
            } else {
                assert!(m.input_cost_micro_per_mtok > 0, "{}", m.id);
                assert!(m.output_cost_micro_per_mtok > 0, "{}", m.id);
            }
        }
    }

    /// Current-generation sanity: the 2026-08 regeneration must not regress to
    /// pre-4.5 Claude / GPT-5.x-less OpenAI / Gemini-2-only eras again.
    #[test]
    fn catalog_is_current_generation() {
        assert!(lookup_model("claude-opus-5").is_some());
        assert!(lookup_model("claude-sonnet-4-5").is_some());
        assert!(lookup_model("gpt-5.4").is_some());
        assert!(lookup_model("gemini-3.6-flash").is_some());
        assert!(lookup_model("grok-4.6").is_some());
        assert!(lookup_model("kimi-k2.6").is_some());
        assert!(lookup_model("glm-5").is_some());
        assert!(lookup_model("claude-sonnet-4").is_none(), "stale row");
        assert!(lookup_model("gpt-4o").is_none(), "stale row");
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
        assert_eq!(display_label("deepseek-chat"), "DeepSeek Chat (V4)");
        assert_eq!(
            display_label("totally-custom-model"),
            "totally-custom-model"
        );
    }
}
