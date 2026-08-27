//! Script-driven deterministic provider: the backbone of Epsilon's offline testing strategy.
//!
//! A `FauxProvider` consumes a script of [`FauxStep`]s (from JSON or code), emitting realistic
//! stream events per call. Requests are recorded for assertions. Full-loop integration tests
//! run offline, free, and millisecond-fast against this — pi's faux-provider pattern.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::message::{AssistantMessage, StopReason, Usage};
use epsilon_contracts::new_id;
use epsilon_contracts::stream::StreamEvent;

use crate::stream_fn::{BoxEventStream, EventStream, LlmContext, ModelRef, StreamFn};

/// One scripted provider response.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FauxStep {
    /// Streams `text` in small deltas, then ends the turn.
    Assistant {
        /// The full assistant text.
        text: String,
    },
    /// Optionally streams text, then requests tool calls and stops.
    ToolCalls {
        /// Optional preamble text.
        text: Option<String>,
        /// The tool calls requested, in order.
        calls: Vec<FauxToolCall>,
    },
    /// Like [`FauxStep::ToolCalls`] but final arguments JSON is truncated mid-string with
    /// stop_reason MaxTokens — exercises the fail-all-tool-calls rule.
    Truncated {
        /// The tool calls whose arguments get cut.
        calls: Vec<FauxToolCall>,
    },
    /// Terminal provider error.
    Error {
        /// Failure detail carried in error_message.
        message: String,
    },
}

impl FauxStep {
    /// Convenience constructor: single-text turn.
    pub fn say(text: impl Into<String>) -> Self {
        FauxStep::Assistant { text: text.into() }
    }

    /// Convenience constructor: one tool call request.
    pub fn call(id: &str, name: &str, arguments: serde_json::Value) -> Self {
        FauxStep::ToolCalls {
            text: None,
            calls: vec![FauxToolCall {
                id: id.into(),
                name: name.into(),
                arguments,
            }],
        }
    }
}

/// A scripted tool call request.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FauxToolCall {
    /// Invocation id to use (stability helps assertions).
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Arguments object.
    pub arguments: serde_json::Value,
}

/// Scripted provider; consumes one step per call; once exhausted it repeats an empty
/// EndTurn so runaway loops still terminate via their own budget.
pub struct FauxProvider {
    steps: Arc<Mutex<VecDeque<FauxStep>>>,
    recorded: Arc<Mutex<Vec<LlmContext>>>,
}

impl FauxProvider {
    /// Build from a script.
    pub fn new(steps: impl Into<VecDeque<FauxStep>>) -> Self {
        Self {
            steps: Arc::new(Mutex::new(steps.into())),
            recorded: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Parse a script from JSON:
    ///
    /// ```json
    /// [{"type":"assistant","text":"hello"},
    ///  {"type":"tool_calls","text":null,
    ///   "calls":[{"id":"tc_1","name":"read","arguments":{"path":"a.rs"}}]}]
    /// ```
    pub fn from_script_json(script: &str) -> Result<Self, String> {
        let steps: Vec<FauxStep> =
            serde_json::from_str(script).map_err(|e| format!("invalid faux script: {e}"))?;
        Ok(Self::new(steps))
    }

    /// Every request this provider received, in order (for assertions).
    pub fn recorded_requests(&self) -> Vec<LlmContext> {
        self.recorded.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Live handle to the request log — lets tests assert after a run completes.
    pub fn recorded_requests_handle(&self) -> Arc<Mutex<Vec<LlmContext>>> {
        Arc::clone(&self.recorded)
    }
}

#[async_trait]
impl StreamFn for FauxProvider {
    async fn stream(&self, model: &ModelRef, ctx: LlmContext) -> BoxEventStream {
        if let Ok(mut r) = self.recorded.lock() {
            r.push(ctx);
        }
        let step = self
            .steps
            .lock()
            .ok()
            .and_then(|mut q| q.pop_front())
            .unwrap_or_else(|| FauxStep::say(
                "Faux provider out of scripted steps — this is the offline demo (only has two turns). Run `epsilon auth login <provider>` and restart epsilon to talk to a real model.",
            ));
        let events = build_events(&model.id.clone(), step);
        Box::new(FauxEventStream {
            pending: events.into(),
        })
    }
}

/// Emits its precomputed event list across successive `next` calls.
struct FauxEventStream {
    pending: VecDeque<StreamEvent>,
}

#[async_trait]
impl EventStream for FauxEventStream {
    async fn next(&mut self) -> Option<StreamEvent> {
        self.pending.pop_front()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn blank(model_id: &str) -> AssistantMessage {
    AssistantMessage {
        id: new_id("msg"),
        timestamp_ms: now_ms(),
        provider: "faux".into(),
        model_id: model_id.to_string(),
        content: vec![],
        stop_reason: StopReason::EndTurn,
        error_message: None,
        usage: Usage {
            input: 100,
            output: 50,
            total_tokens: 150,
            ..Default::default()
        },
    }
}

fn chunks_of(s: &str, n: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < s.len() {
        let end = s[start..]
            .char_indices()
            .nth(n)
            .map(|(i, _)| start + i)
            .unwrap_or(s.len());
        out.push(&s[start..end]);
        start = end;
    }
    out
}

/// Translate one script step into a realistic event sequence.
///
/// Invariants (asserted by tests):
/// - exactly one `Start` first and one `Done` last;
/// - `Done` is authoritative: carries fully assembled content (pi semantics);
/// - text/argument deltas are small slices, mimicking real providers.
fn build_events(model_id: &str, step: FauxStep) -> Vec<StreamEvent> {
    let mut events: Vec<StreamEvent> = Vec::new();
    match step {
        FauxStep::Assistant { text } => {
            events.push(StreamEvent::Start {
                partial: blank(model_id),
            });
            for piece in chunks_of(&text, 8) {
                events.push(StreamEvent::TextDelta {
                    index: 0,
                    delta: piece.to_string(),
                });
            }
            let mut done = blank(model_id);
            done.content = vec![ContentBlock::text(text)];
            events.push(StreamEvent::Done { message: done });
        }
        FauxStep::ToolCalls { text, calls } => {
            events.push(StreamEvent::Start {
                partial: blank(model_id),
            });

            let mut content = Vec::new();
            if let Some(t) = &text {
                for piece in chunks_of(t, 8) {
                    events.push(StreamEvent::TextDelta {
                        index: 0,
                        delta: piece.to_string(),
                    });
                }
                content.push(ContentBlock::text(t.clone()));
            }
            for call in &calls {
                let idx = content.len();
                let args_json =
                    serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into());
                events.push(StreamEvent::ToolCallStart {
                    index: idx,
                    id: call.id.clone(),
                    name: call.name.clone(),
                });
                for piece in chunks_of(&args_json, 6) {
                    events.push(StreamEvent::ToolCallArgumentsDelta {
                        index: idx,
                        delta_json: piece.to_string(),
                    });
                }
                content.push(ContentBlock::ToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                });
            }
            let mut done = blank(model_id);
            done.content = content;
            done.stop_reason = StopReason::ToolUse;
            events.push(StreamEvent::Done { message: done });
        }
        FauxStep::Truncated { calls } => {
            events.push(StreamEvent::Start {
                partial: blank(model_id),
            });

            let mut content = Vec::new();
            for call in &calls {
                let idx = content.len();
                // Serialize then CUT mid-string: models truncated at the token limit emit
                // exactly this shape.
                let args_json =
                    serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into());
                let cut_len = args_json.len().saturating_sub(2).max(1);
                let cut = &args_json[..cut_len];
                events.push(StreamEvent::ToolCallStart {
                    index: idx,
                    id: call.id.clone(),
                    name: call.name.clone(),
                });
                events.push(StreamEvent::ToolCallArgumentsDelta {
                    index: idx,
                    delta_json: cut.to_string(),
                });
                content.push(ContentBlock::ToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: serde_json::Value::String(cut.to_string()),
                });
            }
            let mut done = blank(model_id);
            done.content = content;
            done.stop_reason = StopReason::MaxTokens;
            events.push(StreamEvent::Done { message: done });
        }
        FauxStep::Error { message } => {
            events.push(StreamEvent::Start {
                partial: blank(model_id),
            });
            let mut done = blank(model_id);
            done.stop_reason = StopReason::Error;
            done.error_message = Some(message);
            events.push(StreamEvent::Done { message: done });
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> LlmContext {
        LlmContext {
            system_prompt: "s".into(),
            messages: vec![],
            tools: vec![],
            thinking_level: epsilon_contracts::ThinkingLevel::Off,
        }
    }

    #[tokio::test]
    async fn streams_scripted_assistant_turn() {
        let p = FauxProvider::new(vec![FauxStep::say("hello world")]);
        let mut s = p.stream(&ModelRef::new("faux-1"), ctx()).await;

        assert!(matches!(s.next().await, Some(StreamEvent::Start { .. })));
        let mut text = String::new();
        loop {
            match s.next().await {
                Some(StreamEvent::TextDelta { delta, .. }) => text.push_str(&delta),
                Some(StreamEvent::Done { message }) => {
                    assert_eq!(message.text(), "hello world");
                    assert_eq!(text, "hello world");
                    break;
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn records_requests_and_exhausts_to_empty_endturns() {
        let p = FauxProvider::new(vec![FauxStep::say("only")]);
        let _ = p.stream(&ModelRef::new("faux-1"), ctx()).await;
        let _ = p.stream(&ModelRef::new("faux-1"), ctx()).await; // exhausted -> empty EndTurn
        assert_eq!(p.recorded_requests().len(), 2);
    }

    #[tokio::test]
    async fn parses_json_script() {
        let p = FauxProvider::from_script_json(
            r#"[{"type":"assistant","text":"hi"},
                {"type":"tool_calls","text":null,
                 "calls":[{"id":"tc_1","name":"read","arguments":{"path":"a.rs"}}]}]"#,
        )
        .expect("script parses");
        assert_eq!(p.recorded_requests().len(), 0);
    }
}
