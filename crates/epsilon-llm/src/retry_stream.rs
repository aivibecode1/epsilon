//! The retry EXECUTOR: wraps any [`StreamFn`] with ADR-010's policy.
//!
//! Retry scope is deliberately narrow: only **pre-stream failures** (the first event is
//! `Done{stop_reason: Error}`) are retried — once a stream has started producing, we
//! pass it through untouched rather than risk duplicate partials. This covers the
//! overwhelmingly common case (transport/setup failures surface before the first token)
//! while keeping mid-stream semantics simple and honest.
//!
//! Every scheduled wait fires the `on_retry` hook so callers can render a notice or
//! append a durable ledger entry (ADR-010's keyed attempts; callers add turn/step
//! context the provider layer cannot know).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::retry::{FailureKind, backoff_delay, classify, should_retry};
use crate::stream_fn::{
    BoxEventStream, EventStream, LlmContext, ModelRef, StreamFn, error_message,
};
use epsilon_contracts::message::StopReason;
use epsilon_contracts::stream::StreamEvent;

/// What one scheduled retry looked like (for UI notices / durable ledger entries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryNotice {
    /// Model the failing call targeted.
    pub model: String,
    /// Classified failure.
    pub kind: FailureKind,
    /// Zero-based attempt number that just failed.
    pub attempt: u32,
    /// Scheduled wait before the next try.
    pub delay: Duration,
    /// Final failure detail from the provider (classified text).
    pub detail: String,
}

/// Observer invoked for every scheduled retry wait.
type RetryHook = Box<dyn Fn(&RetryNotice) + Send + Sync>;

/// Wrap `inner` with up to `max_attempts` tries for retryable pre-stream failures.
pub struct Retrying {
    inner: Arc<dyn StreamFn>,
    max_attempts: u32,
    on_retry: Option<RetryHook>,
}

impl Retrying {
    /// Default policy: 4 total attempts (3 retries).
    pub fn new(inner: Arc<dyn StreamFn>) -> Self {
        Self {
            inner,
            max_attempts: 4,
            on_retry: None,
        }
    }

    /// Override the total-attempts cap (minimum 1 = no retries).
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    /// Observe every scheduled wait (UI notice / durable ledger).
    pub fn with_on_retry(mut self, hook: impl Fn(&RetryNotice) + Send + Sync + 'static) -> Self {
        self.on_retry = Some(Box::new(hook));
        self
    }

    fn error_detail(ev: &StreamEvent) -> Option<String> {
        if let StreamEvent::Done { message } = ev
            && message.stop_reason == StopReason::Error
        {
            return Some(
                message
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "unknown provider error".into()),
            );
        }
        None
    }
}

#[async_trait]
impl StreamFn for Retrying {
    async fn stream(&self, model: &ModelRef, ctx: LlmContext) -> BoxEventStream {
        let mut attempt: u32 = 0;
        loop {
            let mut stream = self.inner.stream(model, ctx.clone()).await;
            // Peek event #1: pre-stream failure or live stream?
            match stream.next().await {
                Some(first) => {
                    let retriable = Self::error_detail(&first)
                        .filter(|d| should_retry(classify(d), attempt, self.max_attempts));
                    if let Some(detail) = retriable {
                        let kind = classify(&detail);
                        let delay = backoff_delay(attempt, None);
                        if let Some(hook) = &self.on_retry {
                            hook(&RetryNotice {
                                model: model.id.clone(),
                                kind,
                                attempt,
                                delay,
                                detail: detail.clone(),
                            });
                        }
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                        continue; // fresh attempt; the buffered failure is discarded
                    }
                    return Box::new(PassthroughFirst {
                        first: Some(first),
                        rest: stream,
                    });
                }
                None => {
                    // Degenerate empty stream: an inner contract violation, not a
                    // transient condition — surface it without retrying.
                    return VecStream::single(StreamEvent::Done {
                        message: error_message("provider stream ended without events"),
                    });
                }
            }
        }
    }
}

/// Replays the peeked first event, then streams the remainder verbatim.
struct PassthroughFirst {
    first: Option<StreamEvent>,
    rest: BoxEventStream,
}

#[async_trait]
impl EventStream for PassthroughFirst {
    async fn next(&mut self) -> Option<StreamEvent> {
        if let Some(first) = self.first.take() {
            return Some(first);
        }
        self.rest.next().await
    }
}

/// Test/fixture stream over a fixed event list.
pub(crate) struct VecStream {
    events: std::collections::VecDeque<StreamEvent>,
}

impl VecStream {
    #[allow(dead_code)]
    pub(crate) fn single(ev: StreamEvent) -> BoxEventStream {
        Self::from_events(vec![ev])
    }

    /// Fixed event list -> boxed stream (test fixture).
    pub(crate) fn from_events(events: Vec<StreamEvent>) -> BoxEventStream {
        Box::new(Self {
            events: events.into(),
        })
    }
}

#[async_trait]
impl EventStream for VecStream {
    async fn next(&mut self) -> Option<StreamEvent> {
        self.events.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::faux::{FauxProvider, FauxStep};
    use crate::stream_fn::PanicGuard;
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::message::{AssistantMessage, Usage};
    use epsilon_contracts::new_id;
    use std::sync::Mutex;

    fn ctx() -> LlmContext {
        LlmContext {
            system_prompt: String::new(),
            messages: vec![],
            tools: vec![],
            thinking_level: epsilon_contracts::ThinkingLevel::Off,
        }
    }

    fn model() -> ModelRef {
        ModelRef::new("faux-1")
    }

    async fn collect_done(mut s: BoxEventStream) -> AssistantMessage {
        loop {
            match s.next().await {
                Some(StreamEvent::Done { message }) => return message,
                Some(_) => continue,
                None => panic!("stream ended without Done"),
            }
        }
    }

    /// Fails the first `failures` calls with a retryable 503, then delegates.
    struct Flaky {
        inner: Arc<dyn StreamFn>,
        remaining: Mutex<u32>,
    }

    #[async_trait]
    impl StreamFn for Flaky {
        async fn stream(&self, model: &ModelRef, c: LlmContext) -> BoxEventStream {
            let should_fail = {
                let mut rem = self.remaining.lock().unwrap();
                if *rem > 0 {
                    *rem -= 1;
                    true
                } else {
                    false
                }
            };
            if should_fail {
                return VecStream::single(StreamEvent::Done {
                    message: error_message("HTTP 503 Service Unavailable"),
                });
            }
            self.inner.stream(model, c).await
        }
    }

    #[tokio::test]
    async fn retries_transient_pre_stream_failures_then_succeeds() {
        let flaky = Arc::new(Flaky {
            inner: Arc::new(PanicGuard(FauxProvider::new(vec![FauxStep::Assistant {
                text: "recovered".into(),
            }]))),
            remaining: Mutex::new(2),
        });
        let notices = Arc::new(Mutex::new(Vec::<RetryNotice>::new()));
        let log = Arc::clone(&notices);
        let retried =
            Retrying::new(flaky).with_on_retry(move |n| log.lock().unwrap().push(n.clone()));

        let done = collect_done(retried.stream(&model(), ctx()).await).await;
        assert_eq!(done.text(), "recovered");
        assert_eq!(notices.lock().unwrap().len(), 2);
        assert_eq!(notices.lock().unwrap()[0].kind, FailureKind::Server);
    }

    #[tokio::test]
    async fn exhausted_retries_surface_the_final_error() {
        struct Always503;
        #[async_trait]
        impl StreamFn for Always503 {
            async fn stream(&self, _m: &ModelRef, _c: LlmContext) -> BoxEventStream {
                VecStream::single(StreamEvent::Done {
                    message: error_message("500 internal server error"),
                })
            }
        }
        let done = collect_done(
            Retrying::new(Arc::new(Always503))
                .stream(&model(), ctx())
                .await,
        )
        .await;
        assert!(
            done.error_message
                .as_deref()
                .is_some_and(|m| m.contains("500")),
            "final failure must surface verbatim"
        );
    }

    #[tokio::test]
    async fn auth_failures_never_retry() {
        struct AlwaysAuth;
        #[async_trait]
        impl StreamFn for AlwaysAuth {
            async fn stream(&self, _m: &ModelRef, _c: LlmContext) -> BoxEventStream {
                VecStream::single(StreamEvent::Done {
                    message: error_message("401 Unauthorized"),
                })
            }
        }
        let done = collect_done(
            Retrying::new(Arc::new(AlwaysAuth))
                .stream(&model(), ctx())
                .await,
        )
        .await;
        assert!(done.error_message.unwrap().contains("Unauthorized"));
    }

    #[tokio::test]
    async fn successful_streams_pass_through_untouched() {
        // Deltas must survive in order and exactly once — peeking may not consume them.
        let mut delta = AssistantMessage {
            id: new_id("msg"),
            timestamp_ms: 0,
            provider: "t".into(),
            model_id: "t".into(),
            content: vec![ContentBlock::text("")],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Usage::default(),
        };
        delta.content[0] = ContentBlock::text("hey");
        struct Fixed(Vec<StreamEvent>);
        #[async_trait]
        impl StreamFn for Fixed {
            async fn stream(&self, _m: &ModelRef, _c: LlmContext) -> BoxEventStream {
                VecStream::from_events(self.0.clone())
            }
        }
        let events = vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "he".into(),
            },
            StreamEvent::TextDelta {
                index: 0,
                delta: "y".into(),
            },
            StreamEvent::Done { message: delta },
        ];
        let mut s = Retrying::new(Arc::new(Fixed(events.clone())))
            .stream(&model(), ctx())
            .await;
        let mut seen = Vec::new();
        while let Some(ev) = s.next().await {
            seen.push(match ev {
                StreamEvent::Done { .. } => "done".to_string(),
                StreamEvent::TextDelta { delta, .. } => format!("delta:{delta}"),
                _ => "other".to_string(),
            });
        }
        assert_eq!(seen, vec!["delta:he", "delta:y", "done"]);
    }
}
