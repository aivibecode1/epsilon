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
        /// The provider's opaque signature for this reasoning block — pi's
        /// `thinkingSignature`.
        ///
        /// Anthropic returns one with every reasoning block and requires the same bytes back to
        /// replay the block, which is why pi stores it on the part. This build had no slot for
        /// it, so an imported pi session kept the entry, the role and the text and lost the
        /// signature: the one field-level loss M119 measured (1 entry in `large-session.jsonl`,
        /// 47 in `before-compaction.jsonl`). Carried now, under this build's name for it.
        ///
        /// Optional in **both** directions, which is what keeps the wire format compatible:
        /// `default` parses a block written before this field existed, and
        /// `skip_serializing_if` writes a signature-less block byte-for-byte as it was written
        /// then (`a_signature_less_thinking_block_serializes_exactly_as_before` pins the bytes,
        /// because that is where this promise would quietly break).
        ///
        /// `Some("")` is distinct from `None`: pi writing an empty signature is a part that has
        /// one and it is empty, and the healer keeps the two apart (`legacy_str`'s rule) rather
        /// than tidying a value it was given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
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

    /// The byte-level half of "the wire format stays compatible": a thinking block with no
    /// signature is written exactly as this build wrote it before the field existed.
    ///
    /// `skip_serializing_if` is the promise that would break quietly — a block that serialized
    /// as `"signature":null` would still *parse* here and still round-trip, so re-parsing the
    /// JSON proves nothing. The assertion is on the bytes, and on the older bytes parsing back.
    #[test]
    fn a_signature_less_thinking_block_serializes_exactly_as_before() {
        let block = ContentBlock::Thinking {
            text: "weighing options".into(),
            signature: None,
        };
        assert_eq!(
            serde_json::to_string(&block).unwrap(),
            r#"{"type":"thinking","text":"weighing options"}"#
        );
        // And a block a session written before this change holds still parses — no key, no
        // default to apply, the same value.
        assert_eq!(
            serde_json::from_str::<ContentBlock>(
                r#"{"type":"thinking","text":"weighing options"}"#
            )
            .unwrap(),
            block
        );
    }

    /// A signature is carried through the wire, and comes back as the same value.
    #[test]
    fn a_thinking_signature_round_trips() {
        let block = ContentBlock::Thinking {
            text: "weighing options".into(),
            signature: Some("EpEQCkYIChgCKkCOOc7x0voa".into()),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(
            json.contains(r#""signature":"EpEQCkYIChgCKkCOOc7x0voa""#),
            "the signature must be on the wire, not merely in the struct: {json}"
        );
        assert_eq!(serde_json::from_str::<ContentBlock>(&json).unwrap(), block);
        // Present-and-empty is a value, not an absence: it serializes too.
        let empty = ContentBlock::Thinking {
            text: "t".into(),
            signature: Some(String::new()),
        };
        assert!(
            serde_json::to_string(&empty)
                .unwrap()
                .contains(r#""signature":"""#),
            "an empty signature pi wrote is kept as written, not tidied into an absent one"
        );
    }
}
