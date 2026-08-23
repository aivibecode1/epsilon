//! Static model catalog with explicit capability metadata (pi's generated-catalog
//! approach, bundled: no network at browse time).
//!
//! Two properties enforced since M3 stay binding:
//! - behavior selection reads *capability fields*, never id substrings (anti-pattern
//!   from opencode, see comparative-analysis §4);
//! - prices are documented placeholders until a registry sync lands — never rendered
//!   as authoritative billing.
//!
//! The picker (M11) renders [`MODELS`] grouped by [`ModelInfo::provider`] with
//! [`ModelInfo::env_key`] driving the auth hint column.

/// Static description of one model.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Catalog id (what goes into [`crate::ModelRef`] and genai's resolver).
    pub id: &'static str,
    /// Provider identifier (picker grouping).
    pub provider: &'static str,
    /// Human display name for pickers.
    pub label: &'static str,
    /// Context window in tokens.
    pub context_window: u64,
    /// Maximum output tokens.
    pub max_output_tokens: u64,
    /// Input price, micro-USD per million tokens (PLACEHOLDER).
    pub input_cost_micro_per_mtok: u64,
    /// Output price, micro-USD per million tokens (PLACEHOLDER).
    pub output_cost_micro_per_mtok: u64,
    /// Whether tool/function calling works.
    pub supports_tools: bool,
    /// Env var holding this provider's API key (empty string = none needed).
    pub env_key: &'static str,
}

const ANTHROPIC: &str = "ANTHROPIC_API_KEY";
const OPENAI: &str = "OPENAI_API_KEY";
const DEEPSEEK: &str = "DEEPSEEK_API_KEY";
const GEMINI: &str = "GEMINI_API_KEY";
const XAI: &str = "XAI_API_KEY";
const GROQ: &str = "GROQ_API_KEY";
const MOONSHOT: &str = "MOONSHOT_API_KEY";
const OPENROUTER: &str = "OPENROUTER_API_KEY";

/// Known models, grouped by provider in display order. Capabilities are conservative
/// (`supports_tools: false` hides features we have not verified per model).
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
    },
    // ---- anthropic --------------------------------------------------------------------
    ModelInfo {
        id: "claude-sonnet-4",
        provider: "anthropic",
        label: "Claude Sonnet 4",
        context_window: 200_000,
        max_output_tokens: 16_384,
        input_cost_micro_per_mtok: 3_000_000,
        output_cost_micro_per_mtok: 15_000_000,
        supports_tools: true,
        env_key: ANTHROPIC,
    },
    ModelInfo {
        id: "claude-opus-4-1",
        provider: "anthropic",
        label: "Claude Opus 4.1",
        context_window: 200_000,
        max_output_tokens: 32_000,
        input_cost_micro_per_mtok: 15_000_000,
        output_cost_micro_per_mtok: 75_000_000,
        supports_tools: true,
        env_key: ANTHROPIC,
    },
    ModelInfo {
        id: "claude-haiku-4",
        provider: "anthropic",
        label: "Claude Haiku 4",
        context_window: 200_000,
        max_output_tokens: 16_384,
        input_cost_micro_per_mtok: 800_000,
        output_cost_micro_per_mtok: 4_000_000,
        supports_tools: true,
        env_key: ANTHROPIC,
    },
    // ---- openai -----------------------------------------------------------------------
    ModelInfo {
        id: "gpt-4o",
        provider: "openai",
        label: "GPT-4o",
        context_window: 128_000,
        max_output_tokens: 16_384,
        input_cost_micro_per_mtok: 2_500_000,
        output_cost_micro_per_mtok: 10_000_000,
        supports_tools: true,
        env_key: OPENAI,
    },
    ModelInfo {
        id: "gpt-5",
        provider: "openai",
        label: "GPT-5",
        context_window: 256_000,
        max_output_tokens: 32_000,
        input_cost_micro_per_mtok: 1_250_000,
        output_cost_micro_per_mtok: 10_000_000,
        supports_tools: true,
        env_key: OPENAI,
    },
    ModelInfo {
        id: "gpt-5-mini",
        provider: "openai",
        label: "GPT-5 mini",
        context_window: 256_000,
        max_output_tokens: 32_000,
        input_cost_micro_per_mtok: 250_000,
        output_cost_micro_per_mtok: 2_000_000,
        supports_tools: true,
        env_key: OPENAI,
    },
    // ---- deepseek ---------------------------------------------------------------------
    ModelInfo {
        id: "deepseek-chat",
        provider: "deepseek",
        label: "DeepSeek Chat (V3.x)",
        context_window: 128_000,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 270_000,
        output_cost_micro_per_mtok: 1_100_000,
        supports_tools: true,
        env_key: DEEPSEEK,
    },
    ModelInfo {
        id: "deepseek-reasoner",
        provider: "deepseek",
        label: "DeepSeek Reasoner (R-series)",
        context_window: 128_000,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 550_000,
        output_cost_micro_per_mtok: 2_190_000,
        supports_tools: false,
        env_key: DEEPSEEK,
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
        env_key: GEMINI,
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
        env_key: GEMINI,
    },
    // ---- xai --------------------------------------------------------------------------
    ModelInfo {
        id: "grok-4",
        provider: "xai",
        label: "Grok 4",
        context_window: 256_000,
        max_output_tokens: 32_768,
        input_cost_micro_per_mtok: 3_000_000,
        output_cost_micro_per_mtok: 15_000_000,
        supports_tools: true,
        env_key: XAI,
    },
    ModelInfo {
        id: "grok-3-mini",
        provider: "xai",
        label: "Grok 3 mini",
        context_window: 131_072,
        max_output_tokens: 16_384,
        input_cost_micro_per_mtok: 300_000,
        output_cost_micro_per_mtok: 500_000,
        supports_tools: true,
        env_key: XAI,
    },
    // ---- groq ---------------------------------------------------------------------------
    ModelInfo {
        id: "llama-3.3-70b-versatile",
        provider: "groq",
        label: "Llama 3.3 70B (Groq)",
        context_window: 131_072,
        max_output_tokens: 32_768,
        input_cost_micro_per_mtok: 590_000,
        output_cost_micro_per_mtok: 790_000,
        supports_tools: true,
        env_key: GROQ,
    },
    // ---- moonshot ----------------------------------------------------------------------
    ModelInfo {
        id: "kimi-k2",
        provider: "moonshot",
        label: "Kimi K2",
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 600_000,
        output_cost_micro_per_mtok: 2_500_000,
        supports_tools: true,
        env_key: MOONSHOT,
    },
    // ---- openrouter --------------------------------------------------------------------
    ModelInfo {
        id: "anthropic/claude-sonnet-4",
        provider: "openrouter",
        label: "Claude Sonnet 4 (via OpenRouter)",
        context_window: 200_000,
        max_output_tokens: 16_384,
        input_cost_micro_per_mtok: 3_000_000,
        output_cost_micro_per_mtok: 15_000_000,
        supports_tools: true,
        env_key: OPENROUTER,
    },
    // ---- ollama (local, no key) --------------------------------------------------------
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
    },
];

/// Bundled catalog slice (picker/browse source; live fetch lands post-M11).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_is_exact_match() {
        assert!(lookup_model("claude-sonnet-4").is_some());
        assert!(
            lookup_model("Claude-Sonnet-4").is_none(),
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
    fn every_entry_has_label_and_unique_id() {
        let mut ids: Vec<&str> = MODELS.iter().map(|m| m.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "catalog ids must be unique");
        for m in MODELS {
            assert!(!m.label.is_empty());
            assert!(m.context_window > 0);
        }
    }

    #[test]
    fn local_models_need_no_credentials() {
        assert!(credentials_ready("faux-1"));
        assert!(credentials_ready("llama3.1:8b"));
        assert!(
            !credentials_ready("claude-sonnet-4"),
            "test env has no keys"
        );
        assert_eq!(env_key_for("claude-sonnet-4"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(env_key_for("faux-1"), None);
    }

    #[test]
    fn display_label_falls_back_to_id_for_custom_entries() {
        assert_eq!(display_label("deepseek-chat"), "DeepSeek Chat (V3.x)");
        assert_eq!(
            display_label("totally-custom-model"),
            "totally-custom-model"
        );
    }
}
