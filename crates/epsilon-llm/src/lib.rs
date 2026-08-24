//! Unified multi-provider LLM layer behind the never-failing [`StreamFn`] contract.
//!
//! The rest of Epsilon never sees provider SDKs: providers implement [`StreamFn`], which
//! returns an [`EventStream`] of [`StreamEvent`]s whose terminal event carries success OR
//! failure (encoded as `stop_reason`). This mirrors pi's StreamFn contract one-to-one.
//!
//! Real providers (M17, `real` feature) speak their own HTTP APIs directly
//! ([`DirectProvider`], pi port) — no SDK/router crate. Routing is catalog-driven via
//! each model's [`crate::catalog::ModelInfo::wire`] field.

pub mod catalog;
mod faux;
mod http;
#[cfg(feature = "real")]
mod providers;
mod retry;
mod retry_stream;
mod stream_fn;

pub use catalog::{ModelInfo, curated, display_label, env_key_for, lookup_model};

#[cfg(feature = "real")]
pub use catalog::WireApi;
#[cfg(feature = "real")]
pub use catalog::live_models;
pub use faux::{FauxProvider, FauxStep, FauxToolCall};
#[cfg(feature = "real")]
pub use providers::DirectProvider;
pub use retry::{FailureKind, backoff_delay, classify, should_retry};
pub use retry_stream::{RetryNotice, Retrying};
pub use stream_fn::{BoxEventStream, EventStream, LlmContext, ModelRef, PanicGuard, StreamFn};

/// Default maximum attempts for provider retries (opencode parity: 5).
pub const RETRY_MAX_ATTEMPTS: u32 = 5;
