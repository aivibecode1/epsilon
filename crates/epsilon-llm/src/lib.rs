//! Unified multi-provider LLM layer behind the never-failing [`StreamFn`] contract.
//!
//! The rest of Epsilon never sees provider SDKs: providers implement [`StreamFn`], which
//! returns an [`EventStream`] of [`StreamEvent`]s whose terminal event carries success OR
//! failure (encoded as `stop_reason`). This mirrors pi's StreamFn contract one-to-one.

pub mod catalog;
mod faux;
mod retry;
mod retry_stream;
mod stream_fn;

#[cfg(feature = "real")]
#[allow(dead_code)]
mod genai_adapter;

pub use catalog::{ModelInfo, curated, display_label, env_key_for, lookup_model};
pub use faux::{FauxProvider, FauxStep, FauxToolCall};
pub use retry::{FailureKind, backoff_delay, classify, should_retry};
pub use retry_stream::{RetryNotice, Retrying};
pub use stream_fn::{BoxEventStream, EventStream, LlmContext, ModelRef, PanicGuard, StreamFn};

/// Default maximum attempts for provider retries (opencode parity: 5).
pub const RETRY_MAX_ATTEMPTS: u32 = 5;
