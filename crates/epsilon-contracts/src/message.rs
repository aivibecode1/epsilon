//! Messages: the three LLM-visible roles plus the extension escape hatch.

use serde::{Deserialize, Serialize};

use crate::content::{ContentBlock, UserContent};
use crate::ids::{MessageId, ToolCallId};

/// How much reasoning the model should do before answering (pi's `ThinkingLevel`).
///
/// One shared ladder for every wire; each provider maps/clamps it to its own
/// parameter shape at request-build time (`ProviderWire::apply_thinking`). The
/// level rides in [`crate::LlmContext`](epsilon context) per call and persists
/// across turns like the model id.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    /// No extended thinking requested.
    #[default]
    Off,
    /// Smallest useful budget.
    Minimal,
    /// Low effort.
    Low,
    /// Balanced default for hard tasks.
    Medium,
    /// High effort.
    High,
    /// Extra-high (Anthropic `xhigh`, OpenAI `xhigh`).
    XHigh,
    /// Maximum the provider allows.
    Max,
}

impl ThinkingLevel {
    /// All levels in ascending order (pi's `THINKING_LEVELS`).
    pub const ALL: [ThinkingLevel; 7] = [
        ThinkingLevel::Off,
        ThinkingLevel::Minimal,
        ThinkingLevel::Low,
        ThinkingLevel::Medium,
        ThinkingLevel::High,
        ThinkingLevel::XHigh,
        ThinkingLevel::Max,
    ];

    /// Next level in the ladder, wrapping to `Off` (pi's cycle semantics).
    pub fn next(&self) -> ThinkingLevel {
        let i = Self::ALL.iter().position(|l| l == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Lowercase display name (`"off"`, `"xhigh"`, ...).
    pub fn as_str(&self) -> &'static str {
        match self {
            ThinkingLevel::Off => "off",
            ThinkingLevel::Minimal => "minimal",
            ThinkingLevel::Low => "low",
            ThinkingLevel::Medium => "medium",
            ThinkingLevel::High => "high",
            ThinkingLevel::XHigh => "xhigh",
            ThinkingLevel::Max => "max",
        }
    }

    /// Parse a lowercase name; unknown ids fall back to [`ThinkingLevel::Off`].
    pub fn parse(s: &str) -> Option<ThinkingLevel> {
        Self::ALL
            .iter()
            .find(|l| l.as_str() == s.to_ascii_lowercase())
            .copied()
    }

    /// Whether this level requests any thinking at all.
    pub fn enabled(&self) -> bool {
        *self != ThinkingLevel::Off
    }
}

/// Why the provider stopped producing the assistant message.
///
/// Ported from pi's stop reasons with one rename: `length` becomes [`StopReason::MaxTokens`]
/// because "output token limit" is what every provider actually means, and the loop treats it
/// specially (fail-all-tool-calls rule).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished its turn on its own.
    EndTurn,
    /// The model stopped to request tool execution.
    ToolUse,
    /// Output was cut off by the output token limit. Any tool calls in this message are
    /// unsafe to execute (arguments may be truncated); the loop fails them all.
    MaxTokens,
    /// The provider or transport failed; `error_message` carries detail. Never thrown —
    /// always encoded here (StreamFn never-fails contract).
    Error,
    /// The run was cancelled by the user.
    Aborted,
}

impl StopReason {
    /// True for the two failure-shaped terminal states.
    pub fn is_failure(&self) -> bool {
        matches!(self, StopReason::Error | StopReason::Aborted)
    }
}

/// Token accounting for one provider call.
///
/// Costs use **integer micro-USD** (`u64`) instead of floats: long sessions accumulate many
/// usages, and f64 drift would make displayed totals lie (pattern borrowed from ZSeven-W/agent-rs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt tokens billed at input rate (excludes cache hits).
    pub input: u64,
    /// Completion tokens.
    pub output: u64,
    /// Tokens served from the prompt cache.
    pub cache_read: u64,
    /// Tokens written to the prompt cache this call.
    pub cache_write: u64,
    /// Total tokens (input + output + cache traffic) as reported by the provider when available.
    pub total_tokens: u64,
}

/// Per-rate cost components in micro-USD (1_000_000 = $1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostMicro {
    /// Cost of `Usage::input` tokens.
    pub input: u64,
    /// Cost of `Usage::output` tokens.
    pub output: u64,
    /// Cost of cache reads.
    pub cache_read: u64,
    /// Cost of cache writes.
    pub cache_write: u64,
}

impl Usage {
    /// Component-wise addition (port of pi's `combineUsage`); also available via `+`.
    pub fn combine(self, other: Usage) -> Usage {
        Usage {
            input: self.input + other.input,
            output: self.output + other.output,
            cache_read: self.cache_read + other.cache_read,
            cache_write: self.cache_write + other.cache_write,
            total_tokens: self.total_tokens + other.total_tokens,
        }
    }
}

impl std::ops::Add for Usage {
    type Output = Usage;
    fn add(self, rhs: Usage) -> Usage {
        Usage::combine(self, rhs)
    }
}

impl std::ops::Add for CostMicro {
    type Output = CostMicro;
    fn add(self, rhs: CostMicro) -> CostMicro {
        CostMicro {
            input: self.input + rhs.input,
            output: self.output + rhs.output,
            cache_read: self.cache_read + rhs.cache_read,
            cache_write: self.cache_write + rhs.cache_write,
        }
    }
}

/// A user turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMessage {
    /// Unique id.
    pub id: MessageId,
    /// Unix epoch milliseconds.
    pub timestamp_ms: u64,
    /// Text/image content blocks.
    pub content: Vec<UserContent>,
}

/// The model's response for one provider call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    /// Unique id.
    pub id: MessageId,
    /// Unix epoch milliseconds.
    pub timestamp_ms: u64,
    /// Provider identifier, e.g. `anthropic`.
    pub provider: String,
    /// Model identifier as sent to the provider, e.g. `claude-sonnet-4`.
    pub model_id: String,
    /// Ordered content blocks (text/thinking/tool calls/images).
    pub content: Vec<ContentBlock>,
    /// Terminal state; failures arrive here rather than as errors (StreamFn contract).
    pub stop_reason: StopReason,
    /// Human-readable failure detail when [`StopReason::Error`] or [`StopReason::Aborted`].
    pub error_message: Option<String>,
    /// Token usage reported by the provider for this call.
    pub usage: Usage,
}

impl AssistantMessage {
    /// Collect the tool calls requested by this message, in model order.
    pub fn tool_calls(&self) -> impl Iterator<Item = (&str, &str, &serde_json::Value)> {
        self.content.iter().filter_map(|b| match b {
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
            } => Some((id.as_str(), name.as_str(), arguments)),
            _ => None,
        })
    }

    /// Concatenated text blocks (thinking excluded), joined without separators.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// Result of executing one tool invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultMessage {
    /// Unique id.
    pub id: MessageId,
    /// Unix epoch milliseconds.
    pub timestamp_ms: u64,
    /// Echoes [`crate::ContentBlock::ToolCall`]'s id.
    pub tool_call_id: ToolCallId,
    /// Registered name of the executed tool.
    pub tool_name: String,
    /// Content returned to the model (text/images). Errors put the message here too.
    pub content: Vec<ContentBlock>,
    /// Arbitrary structured payload for UI/log rendering; never sent to the model.
    pub details: serde_json::Value,
    /// Token usage of the tool's own work, if the tool reports any.
    pub usage: Option<Usage>,
    /// True when the invocation failed; the model sees an error result and can recover.
    pub is_error: bool,
}

/// App-defined messages that ride in transcripts but never reach the LLM
/// (pi's `CustomAgentMessages`, flattened to a typed-tag + JSON payload because Rust has no
/// declaration merging; see docs/milestones/M2 for the tradeoff discussion).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomMessage {
    /// Unique id.
    pub id: MessageId,
    /// Unix epoch milliseconds.
    pub timestamp_ms: u64,
    /// Application-chosen discriminator, e.g. `"todo_snapshot"`.
    pub custom_type: String,
    /// Free-form payload owned by the registering component.
    pub payload: serde_json::Value,
}

/// Every transcript participant. Only `user`, `assistant`, and `tool_result` convert to
/// provider payloads; `custom` stays harness-visible (pi's `convertToLlm` filter, built in).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    /// A user turn.
    User(UserMessage),
    /// A model turn.
    Assistant(AssistantMessage),
    /// A tool execution outcome.
    ToolResult(ToolResultMessage),
    /// Harness/app-only message, filtered out before provider calls.
    Custom(CustomMessage),
}

impl Message {
    /// Unix-millisecond timestamp regardless of role.
    pub fn timestamp_ms(&self) -> u64 {
        match self {
            Message::User(m) => m.timestamp_ms,
            Message::Assistant(m) => m.timestamp_ms,
            Message::ToolResult(m) => m.timestamp_ms,
            Message::Custom(m) => m.timestamp_ms,
        }
    }

    /// True when this role converts to a provider payload.
    pub fn is_llm_visible(&self) -> bool {
        !matches!(self, Message::Custom(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::new_id;

    #[test]
    fn thinking_level_ladder_wraps_parses_and_serializes() {
        // Full-ladder cycle from Off lands back on Off.
        let mut level = ThinkingLevel::Off;
        for expected in ThinkingLevel::ALL.iter().skip(1) {
            level = level.next();
            assert_eq!(&level, expected);
        }
        assert_eq!(level.next(), ThinkingLevel::Off, "Max wraps to Off");

        // Parse is case-insensitive and rejects unknowns.
        assert_eq!(ThinkingLevel::parse("XHIGH"), Some(ThinkingLevel::XHigh));
        assert_eq!(ThinkingLevel::parse("off"), Some(ThinkingLevel::Off));
        assert_eq!(ThinkingLevel::parse("bogus"), None);

        // serde round-trip uses snake_case ids (session persistence format).
        let json = serde_json::to_string(&ThinkingLevel::XHigh).expect("serialize");
        assert_eq!(json, "\"x_high\"", "serde rename keeps the wire id stable");
        let back: ThinkingLevel = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, ThinkingLevel::XHigh);

        // Enabled is exactly "not Off".
        for l in ThinkingLevel::ALL {
            assert_eq!(l.enabled(), l != ThinkingLevel::Off);
        }
    }

    fn assistant(stop: StopReason) -> AssistantMessage {
        AssistantMessage {
            id: new_id("msg"),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![ContentBlock::text("hello")],
            stop_reason: stop,
            error_message: None,
            usage: Usage::default(),
        }
    }

    #[test]
    fn roundtrips_all_roles() {
        let msgs = vec![
            Message::User(UserMessage {
                id: new_id("msg"),
                timestamp_ms: 1,
                content: vec![UserContent::Text { text: "hi".into() }],
            }),
            Message::Assistant(assistant(StopReason::EndTurn)),
            Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 3,
                tool_call_id: "tc_1".into(),
                tool_name: "read".into(),
                content: vec![ContentBlock::text("contents")],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            }),
            Message::Custom(CustomMessage {
                id: new_id("evt"),
                timestamp_ms: 4,
                custom_type: "todo".into(),
                payload: serde_json::json!({"items": []}),
            }),
        ];
        for m in msgs {
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), m);
        }
    }

    #[test]
    fn stop_reason_semantics() {
        assert!(StopReason::Error.is_failure());
        assert!(StopReason::Aborted.is_failure());
        assert!(!StopReason::MaxTokens.is_failure());
    }

    #[test]
    fn usage_add_accumulates() {
        let a = Usage {
            input: 10,
            output: 5,
            ..Default::default()
        };
        let b = Usage {
            input: 1,
            output: 2,
            cache_read: 7,
            ..Default::default()
        };
        let c = a + b;
        assert_eq!(c.input, 11);
        assert_eq!(c.output, 7);
        assert_eq!(c.cache_read, 7);
    }

    #[test]
    fn assistant_helpers_work() {
        let mut m = assistant(StopReason::ToolUse);
        m.content.push(ContentBlock::ToolCall {
            id: "tc_9".into(),
            name: "bash".into(),
            arguments: serde_json::json!({"command": "ls"}),
        });
        assert_eq!(m.text(), "hello");
        let calls: Vec<_> = m.tool_calls().collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "bash");
    }

    #[test]
    fn custom_is_not_llm_visible() {
        let m = Message::Custom(CustomMessage {
            id: new_id("evt"),
            timestamp_ms: 0,
            custom_type: "x".into(),
            payload: serde_json::Value::Null,
        });
        assert!(!m.is_llm_visible());
    }
}
