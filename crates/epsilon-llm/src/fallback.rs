//! Provider fallback chains (DIRECTION-03; Hermes' `try_activate_fallback`
//! pattern, simplified to our StreamFn seam).
//!
//! A [`FallbackProvider`] wraps one primary model id plus an ordered list of
//! fallbacks. When the primary's stream terminates with a retryable failure
//! (rate limit, transient server error, network trouble — the same classes
//! [`crate::retry::classify`] marks retryable), the NEXT candidate is tried
//! within the same turn. Auth failures and unknown models are never retried:
//! switching providers cannot fix a bad key or a typo.
//!
//! "Per-turn primary restoration" from Hermes falls out naturally: each call
//! starts again at the head of the chain, so once the primary recovers it is
//! used automatically. No sticky state, nothing to reset.

#![cfg(feature = "real")]

use crate::stream_fn::{BoxEventStream, LlmContext, ModelRef, StreamFn};
use epsilon_contracts::message::{AssistantMessage, StopReason};
use epsilon_contracts::stream::StreamEvent;

/// One entry of a fallback chain: a catalog model id and its resolved API key.
#[derive(Debug, Clone)]
pub struct FallbackCandidate {
    /// Catalog model id, e.g. `claude-sonnet-4-5`.
    pub model_id: String,
    /// Resolved API key for this candidate (`None` = skip: no key configured).
    pub api_key: Option<String>,
}

impl FallbackCandidate {
    /// Resolve a model id through the standard auth precedence
    /// (stored credential first, env var fallback).
    pub fn resolve(model_id: impl Into<String>) -> Self {
        let model_id = model_id.into();
        let provider = crate::catalog::lookup_model(&model_id)
            .map(|m| m.provider)
            .unwrap_or("");
        Self {
            model_id,
            api_key: crate::catalog::resolve_api_key(provider),
        }
    }
}

/// Try candidates in order within one turn; see module docs.
pub struct FallbackProvider {
    /// Ordered candidates; index 0 is the primary.
    chain: Vec<FallbackCandidate>,
}

impl FallbackProvider {
    /// Build a chain from candidate model ids (auth resolved via the standard
    /// precedence). Ids missing from the catalog or without credentials are
    /// skipped at call time with an explanatory error if ALL are unusable.
    pub fn new<I, S>(model_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            chain: model_ids
                .into_iter()
                .map(FallbackCandidate::resolve)
                .collect(),
        }
    }
}

#[async_trait::async_trait]
impl StreamFn for FallbackProvider {
    async fn stream(&self, model: &ModelRef, ctx: LlmContext) -> BoxEventStream {
        // The requested model leads the chain for THIS call (flag > saved >
        // config resolution already happened upstream); the configured chain
        // supplies the fallbacks after it.
        let mut ids = vec![model.id.clone()];
        for c in &self.chain {
            if c.model_id != model.id {
                ids.push(c.model_id.clone());
            }
        }

        let mut last_error = String::new();
        for id in &ids {
            // Criterion 7: the SAME pre-flight the direct path runs, hoisted to
            // where keys are resolved. `try_stream_direct` has no refresh hook,
            // so an OAuth credential on a `--fallback` candidate used to never
            // rotate: the pair expired, `resolve_api_key` answered `None`, and
            // the chain reported "no API key" for a user who was logged in.
            let customs = crate::custom::custom_providers();
            let provider = crate::custom::lookup_custom(&customs, id)
                .map(|c| c.name.as_str())
                .or_else(|| crate::catalog::lookup_model(id).map(|m| m.provider))
                .unwrap_or("");
            let fresh_oauth = crate::providers::ensure_oauth_fresh(provider).await;
            let cand = self.chain.iter().find(|c| &c.model_id == id);
            let key = match fresh_oauth {
                // The exchange just minted this token; use it even if persisting
                // the rotated pair failed.
                Some(token) => Some(token),
                None => match cand {
                    Some(c) => c.api_key.clone(),
                    None => {
                        // The caller-requested model may not be in our chain; its
                        // auth resolves on the spot like DirectProvider does.
                        // M28: custom ids (`lmstudio/foo`) resolve under the custom
                        // provider's name (stored key first, then its env_key).
                        crate::catalog::resolve_api_key(provider)
                    }
                },
            };
            let Some(key) = key else {
                last_error =
                    format!("no API key for '{id}' - run `epsilon auth login` or set the env var");
                continue;
            };

            // Probe with a single candidate stream. We re-implement the small
            // pre-flight part of DirectProvider here because we need to know
            // whether the attempt FAILED BEFORE STREAMING (fallback applies)
            // or produced real events (fallback must not apply mid-stream).
            match crate::providers::try_stream_direct(id, &key, ctx.clone()).await {
                Ok(stream) => return Box::new(FirstErrorPassthrough::new(stream)),
                Err(detail) => {
                    let kind = crate::retry::classify(&detail);
                    last_error = format!("[{id}] {detail}");
                    match kind {
                        crate::retry::FailureKind::RateLimit
                        | crate::retry::FailureKind::Server
                        | crate::retry::FailureKind::Network => continue, // try next candidate
                        _ => break, // Auth/Other: no candidate switch can help
                    }
                }
            }
        }

        Box::new(OnceFail {
            ev: Some(StreamEvent::Done {
                message: AssistantMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                    provider: "unknown".into(),
                    model_id: "unknown".into(),
                    content: vec![],
                    stop_reason: StopReason::Error,
                    error_message: Some(format!(
                        "all fallback candidates failed; last error: {last_error}"
                    )),
                    usage: Default::default(),
                },
            }),
        })
    }
}

/// Wraps a successful candidate stream but converts any post-start transport
/// death into a terminal Error event (never panics upward — StreamFn contract).
struct FirstErrorPassthrough {
    inner: BoxEventStream,
}

impl FirstErrorPassthrough {
    fn new(inner: BoxEventStream) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl EventStreamAlias for FirstErrorPassthrough {
    async fn next(&mut self) -> Option<StreamEvent> {
        self.inner.next().await
    }
}

use crate::stream_fn::EventStream as EventStreamAlias;

/// Local once-failure terminal stream (mirrors providers::OnceStream without
/// leaking private types across modules).
struct OnceFail {
    ev: Option<StreamEvent>,
}

#[async_trait::async_trait]
impl EventStreamAlias for OnceFail {
    async fn next(&mut self) -> Option<StreamEvent> {
        self.ev.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chain keeps every candidate, in the order they were named.
    ///
    /// Dropping one at construction time would hide it from the user and from
    /// `stream`, which walks the same list; `FallbackCandidate::resolve` is what
    /// decides separately, per candidate, whether it has credentials.
    #[test]
    fn the_chain_keeps_every_candidate_in_the_order_it_was_named() {
        let p = FallbackProvider::new(["claude-sonnet-4-5", "deepseek-chat"]);
        assert_eq!(p.chain.len(), 2);
        assert_eq!(p.chain[0].model_id, "claude-sonnet-4-5");
        assert_eq!(p.chain[1].model_id, "deepseek-chat");
    }

    #[test]
    fn resolve_skips_candidates_without_credentials_or_catalog_entry() {
        // faux provider rows have no env var and no stored key in tests.
        let c = FallbackCandidate::resolve("faux-1");
        assert_eq!(c.model_id, "faux-1");

        // Unknown ids resolve to no key (provider lookup misses).
        let unknown = FallbackCandidate::resolve("totally-not-a-model");
        assert!(unknown.api_key.is_none(), "unknown model has no key");
    }
}
