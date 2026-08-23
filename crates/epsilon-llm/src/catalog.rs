//! Minimal static model catalog with explicit capability metadata.
//!
//! Placeholder for the models.dev-synced catalog (planned post-M5). The important property
//! already enforced here: behavior selection reads *capability fields*, never id substrings
//! (anti-pattern from opencode, see comparative-analysis §4).

/// Static description of one model.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Catalog id (what goes into [`crate::ModelRef`]).
    pub id: &'static str,
    /// Provider identifier.
    pub provider: &'static str,
    /// Context window in tokens.
    pub context_window: u64,
    /// Maximum output tokens.
    pub max_output_tokens: u64,
    /// Input price, micro-USD per million tokens.
    pub input_cost_micro_per_mtok: u64,
    /// Output price, micro-USD per million tokens.
    pub output_cost_micro_per_mtok: u64,
    /// Whether tool/function calling works.
    pub supports_tools: bool,
}

/// Known models. Prices are placeholders until the catalog sync lands; capabilities are
/// conservative (false = feature hidden) so we never send unsupported parameters.
pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "claude-sonnet-4",
        provider: "anthropic",
        context_window: 200_000,
        max_output_tokens: 16_384,
        input_cost_micro_per_mtok: 3_000_000,
        output_cost_micro_per_mtok: 15_000_000,
        supports_tools: true,
    },
    ModelInfo {
        id: "gpt-4o",
        provider: "openai",
        context_window: 128_000,
        max_output_tokens: 16_384,
        input_cost_micro_per_mtok: 2_500_000,
        output_cost_micro_per_mtok: 10_000_000,
        supports_tools: true,
    },
    ModelInfo {
        id: "faux-1",
        provider: "faux",
        context_window: 1_000_000,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 0,
        output_cost_micro_per_mtok: 0,
        supports_tools: true,
    },
];

/// Look up a model by catalog id.
pub fn lookup_model(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
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
}
