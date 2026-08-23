//! Content blocks: the atoms inside messages.

use serde::{Deserialize, Serialize};

/// A block inside a user message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserContent {
    /// Plain UTF-8 text.
    Text {
        /// The text itself.
        text: String,
    },
    /// An image reference; `data` is base64-encoded bytes.
    Image(ImageRef),
}

/// Base64-carried image data. Providers receive it as a data URL or native image part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    /// MIME type, e.g. `image/png`.
    pub mime: String,
    /// Base64 payload (no data-URL prefix).
    pub data: String,
}

/// A block inside an assistant or tool-result message.
///
/// Port of pi's content union (`TextContent | ImageContent | ToolCall`), plus an explicit
/// `Thinking` block: pi models reasoning via stream events and message metadata; we keep it
/// as first-class content so transcripts replay thinking verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain UTF-8 text.
    Text {
        /// The text itself.
        text: String,
    },
    /// Model reasoning content (extended thinking), preserved for replay fidelity.
    Thinking {
        /// The reasoning text emitted before/alongside the answer.
        text: String,
    },
    /// An image reference; `data` is base64-encoded bytes.
    Image(ImageRef),
    /// A tool invocation requested by the model.
    ToolCall {
        /// Unique id for this invocation (echoed in the matching [`crate::ToolResultMessage`]).
        id: crate::ids::ToolCallId,
        /// Registered tool name.
        name: String,
        /// Raw JSON arguments exactly as produced by the model (validated later at the gate).
        arguments: serde_json::Value,
    },
}

impl ContentBlock {
    /// Convenience constructor for text blocks.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_tagged_variants() {
        let block = ContentBlock::ToolCall {
            id: "tc_1".into(),
            name: "read".into(),
            arguments: serde_json::json!({"path": "a.rs"}),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"tool_call\""));
        assert_eq!(serde_json::from_str::<ContentBlock>(&json).unwrap(), block);
    }

    #[test]
    fn text_constructor_works() {
        assert_eq!(
            ContentBlock::text("hi"),
            ContentBlock::Text { text: "hi".into() }
        );
    }
}
