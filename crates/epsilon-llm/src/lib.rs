//! Unified multi-provider LLM layer behind the never-failing [`StreamFn`] contract.
//!
//! The rest of Epsilon never sees provider SDKs: providers implement [`StreamFn`], which
//! returns an [`EventStream`] of [`StreamEvent`]s whose terminal event carries success OR
//! failure (encoded as `stop_reason`). This mirrors pi's StreamFn contract one-to-one.

// These modules are consumed by epsilon-core/epsilon-cli landing in M5+; until those
// milestones compile against them, rustc sees them as dead.
#[allow(dead_code)]
mod catalog;
#[allow(dead_code)]
mod faux;
mod retry;
#[allow(dead_code)]
mod stream_fn;

#[cfg(feature = "real")]
#[allow(dead_code)]
mod genai_adapter;

pub use catalog::{ModelInfo, lookup_model};
pub use faux::{FauxProvider, FauxStep};
pub use retry::{FailureKind, backoff_delay, classify, should_retry};
pub use stream_fn::{BoxEventStream, EventStream, LlmContext, ModelRef, StreamFn};

/// Default maximum attempts for provider retries (opencode parity: 5).
pub const RETRY_MAX_ATTEMPTS: u32 = 5;

#[cfg(test)]
mod contract_tests {
    use super::*;

    /// The documented retry budget lives here so policy changes are deliberate.
    #[test]
    fn retry_budget_is_five() {
        assert_eq!(RETRY_MAX_ATTEMPTS, 5);
    }

    #[test]
    fn catalog_contains_faux_and_real_models() {
        assert!(lookup_model("faux-1").is_some());
        assert!(lookup_model("claude-sonnet-4").is_some());
    }
}
