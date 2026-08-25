//! The StreamFn contract: providers speak events, never errors.

use async_trait::async_trait;
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;

use epsilon_contracts::message::{AssistantMessage, Message, StopReason, ThinkingLevel, Usage};
use epsilon_contracts::stream::StreamEvent;
use epsilon_contracts::tool::ToolDescriptor;

/// Reference to the model to call (catalog id; resolution happens inside adapters).
#[derive(Debug, Clone)]
pub struct ModelRef {
    /// Catalog model id, e.g. `claude-sonnet-4`.
    pub id: String,
}

impl ModelRef {
    /// Convenience constructor.
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

/// Everything a provider call needs. Messages are already LLM-visible only — filtering of
/// harness-only messages happened upstream (pi's `convertToLlm` boundary).
#[derive(Debug, Clone)]
pub struct LlmContext {
    /// System prompt (assembled by the prompt tier).
    pub system_prompt: String,
    /// Conversation transcript, oldest first.
    pub messages: Vec<Message>,
    /// Tools offered for this call.
    pub tools: Vec<ToolDescriptor>,
    /// Requested thinking/reasoning effort (pi's per-call `reasoning` option).
    /// Wires map/clamp this to provider-native parameters; `Off` sends nothing
    /// unless the wire must explicitly disable (e.g. OpenRouter).
    pub thinking_level: ThinkingLevel,
}

/// A stream of [`StreamEvent`]s terminating with exactly one `Done`.
///
/// Object-safe on purpose: frontends and tests hold `Box<dyn EventStream>`.
#[async_trait]
pub trait EventStream: Send {
    /// Next event, or `None` after the terminal `Done`.
    async fn next(&mut self) -> Option<StreamEvent>;
}

/// Boxed type returned by every provider.
pub type BoxEventStream = Box<dyn EventStream>;

/// A provider adapter.
///
/// Contract (ported verbatim from pi's types.ts):
/// 1. MUST NOT panic or return an error for request/model/runtime failures;
/// 2. MUST return an event stream whose LAST event is `Done`, with `stop_reason`
///    `Error`/`Aborted` carrying `error_message` on failure;
/// 3. abort/cancellation surfaces as `Done` with `StopReason::Aborted`.
///
/// The blanket [`PanicGuard`] wrapper enforces rule 1 mechanically.
#[async_trait]
pub trait StreamFn: Send + Sync {
    /// Begin one provider call; returns immediately with the live event stream.
    async fn stream(&self, model: &ModelRef, ctx: LlmContext) -> BoxEventStream;
}

/// Blanket wrapper: converts panicking streams into `Done { stop_reason: Error }` streams,
/// so no caller can be crashed through the seam.
pub struct PanicGuard<S>(pub S);

#[async_trait]
impl<S: StreamFn> StreamFn for PanicGuard<S> {
    async fn stream(&self, model: &ModelRef, ctx: LlmContext) -> BoxEventStream {
        let fut = AssertUnwindSafe(self.0.stream(model, ctx));
        match fut.catch_unwind().await {
            Ok(inner) => Box::new(GuardedStream {
                inner,
                started: false,
            }),
            Err(panic) => Box::new(PanicDoneStream::new(format!(
                "provider panicked before streaming: {}",
                panic_message(&panic)
            ))),
        }
    }
}

struct GuardedStream {
    inner: BoxEventStream,
    started: bool,
}

#[async_trait]
impl EventStream for GuardedStream {
    async fn next(&mut self) -> Option<StreamEvent> {
        if !self.started {
            self.started = true;
        }
        let fut = AssertUnwindSafe(self.inner.next());
        match fut.catch_unwind().await {
            Ok(ev) => ev,
            Err(panic) => Some(StreamEvent::Done {
                message: error_message(format!(
                    "provider panicked mid-stream: {}",
                    panic_message(&panic)
                )),
            }),
        }
    }
}

/// Extract a printable string from a panic payload without assuming its type.
fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "opaque panic payload".to_string()
    }
}

/// Degenerate stream that emits exactly one failure `Done`.
struct PanicDoneStream {
    message: Option<StreamEvent>,
}

impl PanicDoneStream {
    fn new(message: String) -> Self {
        Self {
            message: Some(StreamEvent::Done {
                message: error_message(message),
            }),
        }
    }
}

#[async_trait]
impl EventStream for PanicDoneStream {
    async fn next(&mut self) -> Option<StreamEvent> {
        self.message.take()
    }
}

/// Standard failure-shaped assistant message.
pub(crate) fn error_message(detail: impl Into<String>) -> AssistantMessage {
    AssistantMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        provider: "unknown".into(),
        model_id: "unknown".into(),
        content: vec![],
        stop_reason: StopReason::Error,
        error_message: Some(detail.into()),
        usage: Usage::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::faux::{FauxProvider, FauxStep};

    struct PanickyProvider;

    #[async_trait]
    impl StreamFn for PanickyProvider {
        async fn stream(&self, _model: &ModelRef, _ctx: LlmContext) -> BoxEventStream {
            panic!("provider exploded");
        }
    }

    #[tokio::test]
    async fn panic_guard_converts_panics_into_done_error() {
        let guarded = PanicGuard(PanickyProvider);
        let mut s = guarded
            .stream(
                &ModelRef::new("x"),
                LlmContext {
                    system_prompt: String::new(),
                    messages: vec![],
                    tools: vec![],
                    thinking_level: ThinkingLevel::Off,
                },
            )
            .await;
        match s.next().await {
            Some(StreamEvent::Done { message }) => {
                assert_eq!(message.stop_reason, StopReason::Error);
                assert!(message.error_message.unwrap().contains("panicked"));
            }
            other => panic!("expected failure Done, got {other:?}"),
        }
        assert!(s.next().await.is_none());
    }

    #[tokio::test]
    async fn faux_error_step_yields_failure_done_not_panic() {
        let p = PanicGuard(FauxProvider::new(vec![FauxStep::Error {
            message: "429 too many requests".into(),
        }]));
        let mut s = p
            .stream(
                &ModelRef::new("faux-1"),
                LlmContext {
                    system_prompt: String::new(),
                    messages: vec![],
                    tools: vec![],
                    thinking_level: ThinkingLevel::Off,
                },
            )
            .await;
        loop {
            match s.next().await {
                Some(StreamEvent::Done { message }) => {
                    assert_eq!(message.stop_reason, StopReason::Error);
                    break;
                }
                Some(_) => continue,
                None => panic!("stream must terminate with Done"),
            }
        }
    }
}
