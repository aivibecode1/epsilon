//! Context compaction (ADR-007): prune → clamp tail → strict-template summary →
//! amend prior summaries. Model view only; the human transcript is untouched.
//!
//! Triggers live in the loop; this module provides the pure transformation plus a token
//! estimator for pressure math.

use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::Message;

/// Chars-per-token heuristic. Real BPE counting is an optional refinement; the estimate
/// only needs to be stable and roughly right for threshold math.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Estimated tokens across all LLM-visible message content.
pub fn estimate_messages(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message).sum()
}

fn estimate_message(m: &Message) -> usize {
    match m {
        Message::User(u) => u.content.iter().map(user_len).sum(),
        Message::Assistant(a) => a.content.iter().map(block_len).sum::<usize>() + 8,
        Message::ToolResult(t) => t.content.iter().map(block_len).sum::<usize>() + 4,
        Message::Custom(_) => 0,
    }
}

fn user_len(c: &UserContent) -> usize {
    match c {
        UserContent::Text { text } => estimate_tokens(text),
        UserContent::Image(_) => 512, // vision tokens vary wildly; conservative flat cost
    }
}

fn block_len(b: &ContentBlock) -> usize {
    match b {
        ContentBlock::Text { text } | ContentBlock::Thinking { text } => estimate_tokens(text),
        ContentBlock::Image(_) => 512,
        ContentBlock::ToolCall { arguments, .. } => estimate_tokens(&arguments.to_string()) + 8,
    }
}

/// Protected-tail clamp bounds from opencode (ADR-007).
pub const MIN_TAIL_TOKENS: usize = 2_000;
pub const MAX_TAIL_TOKENS: usize = 15_000;

/// Build the summarization request context: strict template + serialized transcript.
///
/// `prior_summary` triggers amend-mode (Hermes rule): we ask for a DELTA update rather than
/// re-summarizing everything, which keeps summaries stable across successive compactions.
pub fn summary_context(
    transcript: &[Message],
    prior_summary: Option<&str>,
) -> epsilon_llm::LlmContext {
    let mut body = String::with_capacity(16 * 1024);
    if let Some(prev) = prior_summary {
        body.push_str("<prior_summary>\n");
        body.push_str(prev);
        body.push_str(
            "\n</prior_summary>\n\nUpdate this summary with anything important from \
                       the NEW conversation below. Keep every section.\n\n",
        );
    }
    body.push_str("<conversation>\n");
    for m in transcript {
        match m {
            Message::User(u) => {
                let t = u
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        UserContent::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                body.push_str(&format!("[user] {t}\n"));
            }
            Message::Assistant(a) => {
                if !a.text().is_empty() {
                    body.push_str(&format!("[assistant] {}\n", a.text()));
                }
                for (_, name, args) in a.tool_calls() {
                    body.push_str(&format!("[assistant tool call] {name} {args}\n"));
                }
            }
            Message::ToolResult(t) => {
                let txt = t
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                // Tool outputs get a hard cap here — they dominate transcripts.
                // Char-boundary floor walk: byte 2000 can land mid-character on CJK
                // or emoji output and `&txt[..CAP]` would panic (plan 002).
                const CAP: usize = 2_000;
                let clipped = if txt.len() > CAP {
                    let cut = txt
                        .char_indices()
                        .map(|(i, _)| i)
                        .take_while(|i| *i <= CAP)
                        .last()
                        .unwrap_or(CAP);
                    format!("{}…[clipped]", &txt[..cut])
                } else {
                    txt
                };
                body.push_str(&format!(
                    "[tool {} result{}] {clipped}\n",
                    t.tool_name,
                    if t.is_error { " ERROR" } else { "" }
                ));
            }
            Message::Custom(_) => {}
        }
    }
    body.push_str("</conversation>\n");

    epsilon_llm::LlmContext {
        system_prompt: SUMMARY_SYSTEM.into(),
        messages: vec![Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text { text: body }],
        })],
        tools: vec![],
    }
}

pub(crate) const SUMMARY_SYSTEM: &str =
    "You compress coding-agent transcripts into structured summaries. Output ONLY the template.";

const TEMPLATE: &str = "\
## Objective
(one line)

## Important Details
- facts, constraints, decisions worth keeping

## Work State
- what was done / changed

## Next Move
- concrete next action

## Relevant Files
- paths touched or planned";

/// Extract the assistant's summary text from a completed provider turn.
pub fn summary_text(answer: &epsilon_contracts::message::AssistantMessage) -> String {
    let t = answer.text();
    if t.trim().is_empty() {
        TEMPLATE.to_string() // degraded-but-valid: never compact to nothing
    } else {
        t
    }
}

/// The model-free pass: drop old tool RESULTS beyond the protected tail window.
///
/// Tool outputs are the bulk of long transcripts and are reconstructible on demand;
/// protecting only the recent window keeps the tail informative while shrinking fast.
/// Returns the slimmed history plus how many results were pruned.
pub fn prune_old_tool_results(
    messages: &[Message],
    tail_protect_tokens: usize,
) -> (Vec<Message>, usize) {
    // Walk from the end accumulating protected budget.
    let mut protected = 0usize;
    let mut cut_index = 0usize; // first index considered prunable
    for (i, m) in messages.iter().enumerate().rev() {
        protected += estimate_message(m);
        if protected >= tail_protect_tokens {
            cut_index = i;
            break;
        }
        cut_index = i;
    }

    let mut out = Vec::with_capacity(messages.len());
    let mut pruned = 0usize;
    for (i, m) in messages.iter().enumerate() {
        match m {
            Message::ToolResult(t) if i < cut_index && !t.is_error => {
                // Replace with a tombstone user-role note so provider shape stays valid.
                out.push(Message::ToolResult(
                    epsilon_contracts::message::ToolResultMessage {
                        id: t.id.clone(),
                        timestamp_ms: t.timestamp_ms,
                        tool_call_id: t.tool_call_id.clone(),
                        tool_name: t.tool_name.clone(),
                        content: vec![ContentBlock::text("[old tool output cleared]")],
                        details: serde_json::json!({"pruned": true}),
                        usage: None,
                        is_error: false,
                    },
                ));
                pruned += 1;
            }
            other => out.push(other.clone()),
        }
    }
    (out, pruned)
}

#[cfg(test)]
mod plan002_tests {
    use super::*;
    use epsilon_contracts::content::{ContentBlock, UserContent};
    use epsilon_contracts::message::{Message, ToolResultMessage, UserMessage};

    fn tool_result_with_output(text: &str) -> Message {
        Message::ToolResult(ToolResultMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            tool_call_id: epsilon_contracts::new_id("call"),
            tool_name: "bash".into(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            details: serde_json::json!({}),
            usage: None,
            is_error: false,
        })
    }

    #[test]
    fn clipping_multi_byte_tool_output_does_not_panic() {
        // CJK chars are 3 bytes each; byte 2000 lands mid-character. The tombstone
        // path (prune) and the summary clip path must both be panic-free.
        let cjk: String = "好".repeat(1000); // 3000 bytes
        let msgs = vec![tool_result_with_output(&cjk)];
        // prune with tail protection 0: cut_index stays 0 → no pruning, passthrough.
        let (out, pruned) = prune_old_tool_results(&msgs, 0);
        assert_eq!(pruned, 0);
        assert!(matches!(&out[0], Message::ToolResult(_)));
        // The panic used to happen inside summary_context's clip path:
        let ctx = summary_context(&msgs, None);
        assert!(!ctx.system_prompt.is_empty() || !ctx.messages.is_empty());
    }

    #[test]
    fn summary_context_clips_multi_byte_without_panicking() {
        let cjk: String = "🌍".repeat(1500); // ~6000 bytes, crosses the 2000-byte cap
        let msgs = vec![tool_result_with_output(&cjk)];
        // Must not panic; body contains a clipped marker.
        let ctx = summary_context(&msgs, None);
        assert!(!ctx.system_prompt.is_empty() || !ctx.messages.is_empty());
    }

    #[test]
    fn summary_context_amend_mode_includes_prior_summary_marker() {
        let user = Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: "progress note".into(),
            }],
        });
        let ctx = summary_context(&[user], Some("PREV"));
        let rendered = format!("{:?}", ctx.messages);
        assert!(
            rendered.contains("<prior_summary>") && rendered.contains("PREV"),
            "amend prompt must embed the prior summary"
        );
    }
}
