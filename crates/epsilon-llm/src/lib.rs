//! Unified multi-provider LLM layer behind the never-failing [`StreamFn`] contract.
//!
//! The rest of Epsilon never sees provider SDKs: providers implement [`StreamFn`], which
//! returns an [`EventStream`] of [`StreamEvent`]s whose terminal event carries success OR
//! failure (encoded as `stop_reason`). This mirrors pi's StreamFn contract one-to-one.
//!
//! Real providers (M17, `real` feature) speak their own HTTP APIs directly
//! ([`DirectProvider`], pi port) — no SDK/router crate. Routing is catalog-driven via
//! each model's [`crate::catalog::ModelInfo::wire`] field.

pub mod auth;
pub mod catalog;
pub mod config_value;
pub mod custom;
#[cfg(feature = "real")]
pub mod fallback;
mod faux;
mod http;
pub mod oauth;
pub mod oauth_login;
#[cfg(feature = "real")]
pub mod providers;
pub use sanitize::{Fix, ParsedArgs, parse_tool_arguments, unique_call_ids};
mod retry;
mod retry_stream;
mod sanitize;
mod stream_fn;

pub use auth::{
    OauthRefresh, OauthRefreshed, OauthToken, StoredCredential, load_credentials,
    oauth_access_token, oauth_access_token_in, oauth_needs_refresh_in, remove_credential,
    save_credential, save_credential_full, save_credential_full_in, save_oauth, save_oauth_in,
    store_refreshed_oauth_in,
};
pub use catalog::{ModelInfo, curated, display_label, env_key_for, lookup_model, resolve_api_key};
pub use config_value::{
    clear_command_cache, is_command_value, missing_template_vars, resolve_config_value,
};

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
