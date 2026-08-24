//! Anthropic Messages API over direct HTTP (pi port).
//!
//! Ported from pi-0.84.2 `packages/ai/src/api/anthropic-messages.ts`: request shape
//! (system as a text block list, `max_tokens` required), auth header (`x-api-key`
//! plus `anthropic-version`), and the SSE event mapping (`message_start` → usage
//! seed, `content_block_delta` with `text_delta`/`thinking_delta`/`input_json_delta`,
//! `message_delta` → stop reason + final usage). Cache-control blocks on the system
//! prompt follow pi's prompt-cache behavior so provider-side caching stays effective.

use super::{HttpEventStream, ProviderWire};
use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::message::{Message, StopReason, ToolResultMessage};
use epsilon_contracts::stream::StreamEvent;
use epsilon_contracts::tool::ToolDescriptor;
use serde_json::json;

/// Unit struct implementing the Anthropic Messages wire (fixture/test visibility).
pub struct AnthropicWire;

impl ProviderWire for AnthropicWire {
    fn endpoint(&self, model: &str) -> String {
        let _ = model;
        "https://api.anthropic.com/v1/messages".to_string()
    }

    fn env_key(&self) -> &'static str {
        "ANTHROPIC_API_KEY"
    }

    fn headers(&self, key: &str) -> Vec<(String, String)> {
        vec![
            ("x-api-key".to_string(), key.to_string()),
            ("anthropic-version".to_string(), "2023-06-01".to_string()),
        ]
    }

    /// Request shape per pi's `buildParams`. System prompt is one cache-controlled
    /// text block (pi sets `cache_control` on the system block; the stable prefix
    /// then hits the provider prompt cache).
    fn request_body(
        &self,
        model: &str,
        ctx: &crate::LlmContext,
    ) -> Result<serde_json::Value, String> {
        let mut messages = Vec::new();
        for m in &ctx.messages {
            match m {
                Message::User(u) => messages.push(json!({
                    "role": "user",
                    "content": [{"type": "text", "text": user_text(u)}],
                })),
                Message::Assistant(a) => {
                    // Assistant turns carry text + thinking + tool_use blocks.
                    let mut blocks = Vec::new();
                    for b in &a.content {
                        match b {
                            ContentBlock::Text { text } if !text.is_empty() => {
                                blocks.push(json!({"type": "text", "text": text}));
                            }
                            ContentBlock::Thinking { .. } => {
                                // Thinking blocks are never replayed to the API
                                // (pi strips them when rebuilding history).
                            }
                            ContentBlock::ToolCall {
                                id,
                                name,
                                arguments,
                            } => {
                                blocks.push(json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": arguments,
                                }));
                            }
                            _ => {}
                        }
                    }
                    if !blocks.is_empty() {
                        messages.push(json!({"role": "assistant", "content": blocks}));
                    }
                }
                Message::ToolResult(t) => {
                    messages.push(tool_result_block(t));
                }
                Message::Custom(_) => {}
            }
        }

        let tools: Vec<serde_json::Value> = ctx
            .tools
            .iter()
            .map(|d: &ToolDescriptor| {
                json!({
                    "name": d.name,
                    "description": d.description,
                    "input_schema": d.parameters_schema,
                })
            })
            .collect();

        let mut body = json!({
            "model": model,
            "max_tokens": max_output_for(model),
            "messages": messages,
            "stream": true,
        });
        if !ctx.system_prompt.is_empty() {
            body["system"] = json!([{
                "type": "text",
                "text": ctx.system_prompt,
                "cache_control": {"type": "ephemeral"},
            }]);
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        Ok(body)
    }

    /// Map one SSE frame (pi's `iterateAnthropicEvents` + push logic).
    fn on_event(&self, ev: &super::SseEvent, st: &mut HttpEventStream) -> Option<Vec<StreamEvent>> {
        let data: serde_json::Value = serde_json::from_str(&ev.data).ok()?;
        let kind = data.get("type")?.as_str()?;
        match kind {
            "message_start" => {
                st.reset_accumulator();
                if let Some(usage) = data.pointer("/message/usage") {
                    st.seed_usage(usage);
                }
                Some(vec![StreamEvent::Start {
                    partial: st.partial(),
                }])
            }
            "content_block_start" => {
                let block_type = data
                    .pointer("/content_block/type")
                    .and_then(|v| v.as_str())?;
                match block_type {
                    "tool_use" => {
                        let name = data
                            .pointer("/content_block/name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown_tool");
                        st.open_tool_call(name.to_string());
                        None
                    }
                    _ => None,
                }
            }
            "content_block_delta" => {
                let delta_type = data.pointer("/delta/type").and_then(|v| v.as_str())?;
                match delta_type {
                    "text_delta" => {
                        let text = data.pointer("/delta/text").and_then(|v| v.as_str())?;
                        Some(vec![StreamEvent::TextDelta {
                            index: 0,
                            delta: text.to_string(),
                        }])
                    }
                    "thinking_delta" => {
                        let text = data.pointer("/delta/thinking").and_then(|v| v.as_str())?;
                        Some(vec![StreamEvent::ThinkingDelta {
                            index: 0,
                            delta: text.to_string(),
                        }])
                    }
                    "input_json_delta" => {
                        let frag = data
                            .pointer("/delta/partial_json")
                            .and_then(|v| v.as_str())?;
                        st.feed_tool_json(frag);
                        None
                    }
                    _ => None,
                }
            }
            "content_block_stop" => {
                st.close_tool_call();
                None
            }
            "message_delta" => {
                // Final usage arrives here (pi: only overwrite fields present).
                if let Some(usage) = data.get("usage") {
                    st.update_usage_tail(usage);
                }
                // Terminal stop reason rides message_delta (max_tokens / refusal /
                // pause_turn); remembered and mapped at message_stop below.
                if let Some(sr) = data.pointer("/delta/stop_reason").and_then(|v| v.as_str()) {
                    st.note_finish_reason(sr);
                }
                None
            }
            "message_stop" => {
                // Wire stop reason (from message_delta) drives the mapping; tool_use
                // still wins when calls are present, matching map_stop's ordering.
                let has_calls = st.has_open_or_closed_tools();
                let (stop, err) = self.map_stop(st.finish_reason(), has_calls);
                Some(vec![StreamEvent::Done {
                    message: st.finish(stop, err),
                }])
            }
            "error" => {
                let detail = data
                    .pointer("/error/message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown anthropic stream error")
                    .to_string();
                Some(vec![StreamEvent::Done {
                    message: crate::stream_fn::error_message(format!(
                        "anthropic stream error: {detail}"
                    )),
                }])
            }
            "ping" => None,
            _ => None,
        }
    }

    /// pi's `mapStopReason`: tool_use wins over end_turn when calls are present.
    fn map_stop(&self, wire: Option<&str>, has_tool_calls: bool) -> (StopReason, Option<String>) {
        match wire {
            Some("max_tokens") => (
                StopReason::MaxTokens,
                Some("hit the max output token limit".into()),
            ),
            Some("refusal") | Some("stop_reason_refusal") => {
                (StopReason::Error, Some("model refused".into()))
            }
            _ if has_tool_calls => (StopReason::ToolUse, None),
            _ => (StopReason::EndTurn, None),
        }
    }

    /// Anthropic reports final usage only in `message_delta`; treat it as authoritative.
    fn finalize_usage_from_tail(&self) -> bool {
        true
    }
}

fn user_text(u: &epsilon_contracts::message::UserMessage) -> String {
    u.content
        .iter()
        .filter_map(|c| match c {
            epsilon_contracts::content::UserContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Tool results replay as a user turn carrying `tool_result` blocks (pi's
/// `convertMessages`). Multiple consecutive results collapse into one turn.
fn tool_result_block(t: &ToolResultMessage) -> serde_json::Value {
    let body: String = t
        .content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    json!({
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": t.tool_call_id,
            "content": body,
            "is_error": t.is_error,
        }],
    })
}

/// Conservative output ceiling per model family (catalog refines later).
fn max_output_for(model: &str) -> u64 {
    crate::catalog::lookup_model(model)
        .map(|m| m.max_output_tokens)
        .unwrap_or(16_384)
}
