//! The retry EXECUTOR: wraps any [`StreamFn`] with ADR-010's policy.
//!
//! Retry scope: a failure is retried while **nothing user-visible has been emitted**.
//! That covers pre-stream failures (the first event is `Done{stop_reason: Error}`) and
//! live streams that die before their first delta — a stall watchdog tripping, a proxy
//! cutting a quiet connection, a provider 500 sent as an SSE error frame. In both cases
//! the retry is invisible: no partial was ever shown, so nothing can be duplicated.
//!
//! Once a delta has been forwarded we pass failures through untouched. Re-running the
//! model after the user has watched half an answer would duplicate or contradict
//! visible text, and silently discarding that text is worse than an honest error.
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
type RetryHook = Arc<dyn Fn(&RetryNotice) + Send + Sync>;

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
        self.on_retry = Some(Arc::new(hook));
        self
    }
}

/// Extract the failure detail from a terminal error event (`None` = not a failure).
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

/// Whether an event is output the user could already have seen. `Start` is not:
/// it only opens the message, so a fresh attempt may replace it (M60).
fn is_visible_output(ev: &StreamEvent) -> bool {
    matches!(
        ev,
        StreamEvent::TextDelta { .. }
            | StreamEvent::ThinkingDelta { .. }
            | StreamEvent::ToolCallStart { .. }
            | StreamEvent::ToolCallArgumentsDelta { .. }
            | StreamEvent::Done { .. }
    )
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
                    let retriable = error_detail(&first)
                        .filter(|d| should_retry(classify(d), attempt, self.max_attempts));
                    if let Some(detail) = retriable {
                        notice(&self.on_retry, model, &detail, attempt).await;
                        attempt += 1;
                        continue; // fresh attempt; the buffered failure is discarded
                    }
                    return Box::new(LiveStream {
                        inner: Arc::clone(&self.inner),
                        model: model.clone(),
                        ctx: ctx.clone(),
                        max_attempts: self.max_attempts,
                        on_retry: self.on_retry.clone(),
                        attempt,
                        first: Some(first),
                        rest: Some(stream),
                        emitted: false,
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

/// Fire the retry notice (if any) and wait the scheduled backoff.
async fn notice(hook: &Option<RetryHook>, model: &ModelRef, detail: &str, attempt: u32) {
    let kind = classify(detail);
    let delay = backoff_delay(attempt, None);
    if let Some(hook) = hook {
        hook(&RetryNotice {
            model: model.id.clone(),
            kind,
            attempt,
            delay,
            detail: detail.to_string(),
        });
    }
    tokio::time::sleep(delay).await;
}

/// Replays the peeked first event, then streams the remainder — and, while nothing
/// visible has been emitted, re-runs the whole call on a retryable failure (M60).
struct LiveStream {
    inner: Arc<dyn StreamFn>,
    model: ModelRef,
    ctx: LlmContext,
    max_attempts: u32,
    on_retry: Option<RetryHook>,
    /// Attempt that produced the currently-live stream (zero-based).
    attempt: u32,
    /// Peeked-but-unreplayed event.
    first: Option<StreamEvent>,
    /// The live attempt; `None` once it is exhausted or replaced.
    rest: Option<BoxEventStream>,
    /// A delta went out: a retry now would duplicate or contradict visible output.
    emitted: bool,
}

impl LiveStream {
    /// Re-run the call after a content-free failure, swallowing the fresh stream's
    /// duplicate `Start` — the caller has already seen this call open.
    async fn retry(&mut self, detail: &str) {
        notice(&self.on_retry, &self.model, detail, self.attempt).await;
        self.attempt += 1;
        let mut fresh = self.inner.stream(&self.model, self.ctx.clone()).await;
        match fresh.next().await {
            Some(StreamEvent::Start { .. }) => {}
            Some(ev) => self.first = Some(ev),
            None => {
                // The replacement stream is degenerate; let the caller see the end
                // rather than looping without progress.
                self.rest = None;
                return;
            }
        }
        self.rest = Some(fresh);
    }
}

#[async_trait]
impl EventStream for LiveStream {
    async fn next(&mut self) -> Option<StreamEvent> {
        loop {
            if let Some(first) = self.first.take() {
                self.emitted |= is_visible_output(&first);
                return Some(first);
            }
            let stream = self.rest.as_mut()?;
            match stream.next().await {
                Some(ev) => {
                    let retriable = error_detail(&ev).filter(|d| {
                        !self.emitted && should_retry(classify(d), self.attempt, self.max_attempts)
                    });
                    if let Some(detail) = retriable {
                        self.retry(&detail).await;
                        continue;
                    }
                    self.emitted |= is_visible_output(&ev);
                    return Some(ev);
                }
                None => self.rest = None,
            }
        }
    }
}

/// Test/fixture stream over a fixed event list.
pub(crate) struct VecStream {
    events: std::collections::VecDeque<StreamEvent>,
}

impl VecStream {
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

    /// Streams that open, then fail with a retryable error having emitted `deltas`
    /// text deltas first (M60 fixtures).
    struct OpensThenDies {
        deltas: usize,
        calls: Mutex<u32>,
    }

    #[async_trait]
    impl StreamFn for OpensThenDies {
        async fn stream(&self, m: &ModelRef, _c: LlmContext) -> BoxEventStream {
            *self.calls.lock().unwrap() += 1;
            let mut events = vec![StreamEvent::Start {
                partial: empty_assistant(m.id.as_str()),
            }];
            for i in 0..self.deltas {
                events.push(StreamEvent::TextDelta {
                    index: 0,
                    delta: format!("d{i}"),
                });
            }
            events.push(StreamEvent::Done {
                message: error_message("connection reset by peer"),
            });
            VecStream::from_events(events)
        }
    }

    fn empty_assistant(model_id: &str) -> AssistantMessage {
        AssistantMessage {
            id: new_id("msg"),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: model_id.to_string(),
            content: vec![],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Usage::default(),
        }
    }

    #[tokio::test]
    async fn a_live_stream_that_dies_before_any_output_is_retried_transparently() {
        // The stall-watchdog / mid-stream-reset case: the call opened, produced
        // nothing, and failed. Nobody saw anything, so the retry is invisible —
        // including exactly ONE Start in the caller's view.
        let provider = Arc::new(OpensThenDies {
            deltas: 0,
            calls: Mutex::new(0),
        });
        let notices = Arc::new(Mutex::new(Vec::<RetryNotice>::new()));
        let log = Arc::clone(&notices);
        let mut s = Retrying::new(Arc::clone(&provider) as Arc<dyn StreamFn>)
            .with_max_attempts(3)
            .with_on_retry(move |n| log.lock().unwrap().push(n.clone()))
            .stream(&model(), ctx())
            .await;

        let mut starts = 0;
        let mut terminal = None;
        while let Some(ev) = s.next().await {
            match ev {
                StreamEvent::Start { .. } => starts += 1,
                StreamEvent::Done { message } => {
                    terminal = Some(message);
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(*provider.calls.lock().unwrap(), 3, "one per attempt");
        assert_eq!(starts, 1, "the duplicate Start of a retry is swallowed");
        assert_eq!(notices.lock().unwrap().len(), 2, "one notice per wait");
        assert_eq!(notices.lock().unwrap()[0].kind, FailureKind::Network);
        let done = terminal.expect("terminal event");
        assert!(
            done.error_message
                .as_deref()
                .is_some_and(|m| m.contains("reset")),
            "exhausted attempts surface the real failure: {done:?}"
        );
    }

    #[tokio::test]
    async fn a_stream_that_died_after_output_passes_the_failure_through() {
        // Half an answer is already on screen: re-running would duplicate it.
        let provider = Arc::new(OpensThenDies {
            deltas: 2,
            calls: Mutex::new(0),
        });
        let notices = Arc::new(Mutex::new(Vec::<RetryNotice>::new()));
        let log = Arc::clone(&notices);
        let mut s = Retrying::new(Arc::clone(&provider) as Arc<dyn StreamFn>)
            .with_on_retry(move |n| log.lock().unwrap().push(n.clone()))
            .stream(&model(), ctx())
            .await;

        let mut seen = Vec::new();
        while let Some(ev) = s.next().await {
            match ev {
                StreamEvent::TextDelta { delta, .. } => seen.push(delta),
                StreamEvent::Done { .. } => break,
                _ => {}
            }
        }
        assert_eq!(seen, vec!["d0", "d1"], "visible deltas arrive exactly once");
        assert_eq!(
            *provider.calls.lock().unwrap(),
            1,
            "never re-run after output"
        );
        assert!(notices.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_live_failure_recovered_on_the_next_attempt_streams_normally() {
        // First call: opens, stalls, fails. Second: the real answer.
        struct StallThenAnswer {
            calls: Mutex<u32>,
        }
        #[async_trait]
        impl StreamFn for StallThenAnswer {
            async fn stream(&self, m: &ModelRef, _c: LlmContext) -> BoxEventStream {
                let n = {
                    let mut c = self.calls.lock().unwrap();
                    *c += 1;
                    *c
                };
                if n == 1 {
                    return VecStream::from_events(vec![
                        StreamEvent::Start {
                            partial: empty_assistant(m.id.as_str()),
                        },
                        StreamEvent::Done {
                            message: error_message("provider stream timed out: no data for 120s"),
                        },
                    ]);
                }
                let mut final_msg = empty_assistant(m.id.as_str());
                final_msg.content = vec![ContentBlock::text("answer")];
                VecStream::from_events(vec![
                    StreamEvent::Start {
                        partial: empty_assistant(m.id.as_str()),
                    },
                    StreamEvent::TextDelta {
                        index: 0,
                        delta: "answer".into(),
                    },
                    StreamEvent::Done { message: final_msg },
                ])
            }
        }
        let provider = Arc::new(StallThenAnswer {
            calls: Mutex::new(0),
        });
        let done = collect_done(
            Retrying::new(Arc::clone(&provider) as Arc<dyn StreamFn>)
                .stream(&model(), ctx())
                .await,
        )
        .await;
        assert_eq!(done.text(), "answer");
        assert_eq!(*provider.calls.lock().unwrap(), 2);
        // The watchdog message classifies as a network failure — that classification
        // is what lets the retry (and the model-level fallback) act on it.
        assert_eq!(
            classify("provider stream timed out: no data for 120s"),
            FailureKind::Network
        );
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
