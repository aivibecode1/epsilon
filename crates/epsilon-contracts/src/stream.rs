//! Provider stream events, folded by the loop into a growing assistant message.
//!
//! Port of pi's `AssistantMessageEvent` with deliberate reduction: `text_start/end` and
//! `thinking_start/end` carry no folding information (pi only uses them to swap the partial
//! message pointer), so Epsilon streams deltas plus one terminal event. Fewer events = smaller
//! wire surface for the extension protocol later; folding semantics are unchanged.

use serde::{Deserialize, Serialize};

use crate::message::AssistantMessage;

/// One event from a provider stream.
///
/// Contract: every stream terminates with exactly one [`StreamEvent::Done`], whose
/// [`crate::StopReason`] encodes success OR failure — providers never "throw".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Stream opened; carries the empty partial message the deltas will grow.
    Start {
        /// Initial partial assistant message (ids/metadata set, content empty).
        partial: AssistantMessage,
    },
    /// A chunk of text content was emitted.
    TextDelta {
        /// Index into the partial message's content blocks being appended to.
        index: usize,
        /// Raw text fragment.
        delta: String,
    },
    /// A chunk of thinking/reasoning content was emitted.
    ThinkingDelta {
        /// Index into the partial message's content blocks being appended to.
        index: usize,
        /// Raw reasoning fragment.
        delta: String,
    },
    /// A tool call header arrived (id + name known, arguments incomplete).
    ToolCallStart {
        /// Position of this tool call among the message's tool calls.
        index: usize,
        /// Unique invocation id assigned by the provider.
        id: crate::ids::ToolCallId,
        /// Tool name requested.
        name: String,
    },
    /// A fragment of tool-call arguments JSON arrived.
    ToolCallArgumentsDelta {
        /// Position of this tool call among the message's tool calls.
        index: usize,
        /// Raw JSON fragment; concatenation across deltas forms complete arguments.
        delta_json: String,
    },
    /// Terminal event: the fully assembled message, success or failure encoded in
    /// `stop_reason`.
    Done {
        /// Final assistant message.
        message: AssistantMessage,
    },
}

/// Fold stream events into an optional partial + final message pair.
///
/// This is the single place that knows how deltas become blocks; faux and real providers both
/// emit events and the loop consumes the result, so folding rules live in exactly one spot.
#[derive(Debug, Default)]
pub struct StreamFolder {
    partial: Option<AssistantMessage>,
}

impl StreamFolder {
    /// Feed one event, updating internal state. Returns the event back (for emission) after
    /// state has been applied, or `None` if the event was invalid mid-stream (defensive:
    /// callers treat this as a protocol error).
    pub fn apply(&mut self, event: StreamEvent) -> Option<StreamEvent> {
        match event {
            StreamEvent::Start { ref partial } => {
                if self.partial.is_some() {
                    return None;
                }
                let mut p = partial.clone();
                p.content.clear();
                self.partial = Some(p);
            }
            StreamEvent::TextDelta { index, ref delta } => {
                let msg = self.partial.as_mut()?;
                while msg.content.len() <= index {
                    msg.content.push(crate::content::ContentBlock::text(""));
                }
                match msg.content.get_mut(index) {
                    Some(crate::content::ContentBlock::Text { text }) => text.push_str(delta),
                    Some(crate::content::ContentBlock::Thinking { .. }) => {
                        // DeepSeek/Anthropic wire collision: thinking and text both at index 0.
                        // Preserve both blocks by spilling text to the next text slot.
                        let next = index + 1;
                        while msg.content.len() <= next {
                            msg.content.push(crate::content::ContentBlock::text(""));
                        }
                        match msg.content.get_mut(next) {
                            Some(crate::content::ContentBlock::Text { text }) => {
                                text.push_str(delta)
                            }
                            Some(crate::content::ContentBlock::Thinking { .. }) => {
                                msg.content.insert(
                                    next,
                                    crate::content::ContentBlock::text(delta.clone()),
                                );
                            }
                            _ => return None,
                        }
                    }
                    _ => return None,
                }
            }
            StreamEvent::ThinkingDelta { index, ref delta } => {
                let msg = self.partial.as_mut()?;
                while msg.content.len() <= index {
                    msg.content.push(crate::content::ContentBlock::Thinking {
                        text: String::new(),
                        signature: None,
                    });
                }
                match msg.content.get_mut(index) {
                    Some(crate::content::ContentBlock::Thinking { text, .. }) => {
                        text.push_str(delta)
                    }
                    Some(crate::content::ContentBlock::Text { .. }) => {
                        let next = index + 1;
                        while msg.content.len() <= next {
                            msg.content.push(crate::content::ContentBlock::Thinking {
                                text: String::new(),
                                signature: None,
                            });
                        }
                        match msg.content.get_mut(next) {
                            Some(crate::content::ContentBlock::Thinking { text, .. }) => {
                                text.push_str(delta)
                            }
                            Some(crate::content::ContentBlock::Text { .. }) => {
                                msg.content.insert(
                                    next,
                                    crate::content::ContentBlock::Thinking {
                                        text: delta.clone(),
                                        // No provider stream event carries a signature, so a
                                        // block folded from deltas has none; the field exists
                                        // for the values a session file brings in (M119's pi
                                        // import), not for a guess made here.
                                        signature: None,
                                    },
                                );
                            }
                            _ => return None,
                        }
                    }
                    _ => return None,
                }
            }
            StreamEvent::ToolCallStart {
                index,
                ref id,
                ref name,
            } => {
                let msg = self.partial.as_mut()?;
                while msg.content.len() <= index {
                    // placeholder keeps block indices aligned until arguments arrive
                    msg.content.push(crate::content::ContentBlock::text(""));
                }
                msg.content[index] = crate::content::ContentBlock::ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: serde_json::Value::String(String::new()),
                };
            }
            StreamEvent::ToolCallArgumentsDelta {
                index,
                ref delta_json,
            } => {
                let msg = self.partial.as_mut()?;
                match msg.content.get_mut(index)? {
                    crate::content::ContentBlock::ToolCall { arguments, .. } => {
                        // Arguments accumulate as a JSON *string* until Done parses them;
                        // truncated streams therefore yield detectably-invalid JSON.
                        let acc = arguments.as_str().unwrap_or_default().to_string();
                        let mut acc = acc;
                        acc.push_str(delta_json);
                        *arguments = serde_json::Value::String(acc);
                    }
                    _ => return None,
                }
            }
            StreamEvent::Done { mut message } => {
                // The Done message is authoritative (pi semantics); deltas only fed the
                // live-updated partial above. Parse any string-typed argument payloads here,
                // which also makes truncated (MaxTokens) calls detectably invalid (Null).
                for block in &mut message.content {
                    if let crate::content::ContentBlock::ToolCall { arguments, .. } = block
                        && let Some(raw) = arguments.as_str()
                    {
                        let parsed: serde_json::Value = if raw.trim().is_empty() {
                            serde_json::json!({})
                        } else {
                            serde_json::from_str(raw).unwrap_or(serde_json::Value::Null)
                        };
                        *arguments = parsed;
                    }
                }
                self.partial = None;
                return Some(StreamEvent::Done { message });
            }
        }
        Some(event)
    }

    /// The current partial message, if a stream is open.
    pub fn partial(&self) -> Option<&AssistantMessage> {
        self.partial.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::new_id;
    use crate::message::{StopReason, Usage};

    fn blank() -> AssistantMessage {
        AssistantMessage {
            id: new_id("msg"),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Usage::default(),
        }
    }

    #[test]
    fn folds_text_and_tool_call() {
        let mut f = StreamFolder::default();
        f.apply(StreamEvent::Start { partial: blank() }).unwrap();
        f.apply(StreamEvent::TextDelta {
            index: 0,
            delta: "hel".into(),
        })
        .unwrap();
        f.apply(StreamEvent::TextDelta {
            index: 0,
            delta: "lo".into(),
        })
        .unwrap();
        assert_eq!(f.partial().map(|p| p.text()).as_deref(), Some("hello"));

        // Done is authoritative (pi semantics): carries the full final message; the folder
        // normalizes string-typed tool arguments into JSON values.
        let mut final_msg = blank();
        final_msg.content = vec![
            crate::content::ContentBlock::text("hello"),
            crate::content::ContentBlock::ToolCall {
                id: "tc_1".into(),
                name: "read".into(),
                arguments: serde_json::Value::String("{\"path\":\"a.rs\"}".into()),
            },
        ];
        let done = f.apply(StreamEvent::Done { message: final_msg }).unwrap();
        let StreamEvent::Done { message } = done else {
            panic!("expected done")
        };
        assert_eq!(message.text(), "hello");
        let (_, name, args) = message.tool_calls().next().unwrap();
        assert_eq!(name, "read");
        assert_eq!(*args, serde_json::json!({"path": "a.rs"}));
        assert!(f.partial().is_none());
    }

    #[test]
    fn truncated_arguments_stay_detectable_as_null() {
        let mut final_msg = blank();
        final_msg.stop_reason = StopReason::MaxTokens;
        final_msg.content = vec![crate::content::ContentBlock::ToolCall {
            id: "tc_2".into(),
            name: "write".into(),
            arguments: serde_json::Value::String("{\"path\":".into()),
        }];
        let mut f = StreamFolder::default();
        f.apply(StreamEvent::Start { partial: blank() }).unwrap();
        let done = f.apply(StreamEvent::Done { message: final_msg }).unwrap();
        let StreamEvent::Done { message } = done else {
            panic!("expected done")
        };
        let (_, _, args) = message.tool_calls().next().unwrap();
        assert_eq!(*args, serde_json::Value::Null);
    }

    #[test]
    fn double_start_is_protocol_error() {
        let mut f = StreamFolder::default();
        f.apply(StreamEvent::Start { partial: blank() }).unwrap();
        assert!(f.apply(StreamEvent::Start { partial: blank() }).is_none());
    }
}
