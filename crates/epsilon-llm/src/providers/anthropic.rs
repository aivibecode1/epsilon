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

/// Pinned Claude Code CLI version pi reports as `user-agent: claude-cli/<v>`
/// on the OAuth branch (`anthropic-messages.ts: claudeCodeVersion`).
pub const CLAUDE_CODE_VERSION: &str = "2.1.75";

/// pi `isOAuthToken`: OAuth access tokens carry the `sk-ant-oat` infix;
/// API keys never do. The flag selects the Bearer + Claude Code identity
/// header set (and the body shape) in `headers` / `request_body`.
pub fn is_oauth_token(key: &str) -> bool {
    key.contains("sk-ant-oat")
}

impl ProviderWire for AnthropicWire {
    fn endpoint(&self, model: &str) -> String {
        let _ = model;
        "https://api.anthropic.com/v1/messages".to_string()
    }

    fn env_key(&self) -> &'static str {
        "ANTHROPIC_API_KEY"
    }

    fn headers(&self, key: &str) -> Vec<(String, String)> {
        if is_oauth_token(key) {
            // pi `createClient` OAuth branch: Bearer auth + Claude Code
            // identity headers (`authToken`, `claude-code-20250219` +
            // `oauth-2025-04-20` betas, `user-agent: claude-cli/...`,
            // `x-app: cli`). The access token arrives via `toAuth`.
            vec![
                ("authorization".to_string(), format!("Bearer {key}")),
                (
                    "anthropic-beta".to_string(),
                    "claude-code-20250219,oauth-2025-04-20".to_string(),
                ),
                (
                    "user-agent".to_string(),
                    format!("claude-cli/{CLAUDE_CODE_VERSION}"),
                ),
                ("x-app".to_string(), "cli".to_string()),
            ]
        } else {
            vec![
                ("x-api-key".to_string(), key.to_string()),
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ]
        }
    }

    /// Request shape per pi's `buildParams`. System prompt is one cache-controlled
    /// text block (pi sets `cache_control` on the system block; the stable prefix
    /// then hits the provider prompt cache). On the OAuth branch pi PREPENDS the
    /// Claude Code identity line before the caller prompt (same cache block).
    fn request_body(
        &self,
        model: &str,
        ctx: &crate::LlmContext,
    ) -> Result<serde_json::Value, String> {
        let mut messages = Vec::new();
        // Which message index gets the tail breakpoint, decided **before** the loop so
        // the body is not mutated while it is being read, and because the decision is a
        // property of the conversation rather than of any one block's shape.
        let tail_break = crate::cache_policy::CachePolicy::DEFAULT
            .mark_last_user_turn
            .then(|| last_user_turn_index(&ctx.messages))
            .flatten();
        // Consecutive tool results are one user-role turn on the wire, not one
        // message per result. Grouping them is both the shape Anthropic expects
        // after parallel tool calls and the boundary the cache marker lands on.
        let mut pending_tool_results: Vec<serde_json::Value> = Vec::new();
        let mut pending_is_tail = false;
        for (index, m) in ctx.messages.iter().enumerate() {
            match m {
                Message::User(u) => {
                    flush_tool_results(&mut messages, &mut pending_tool_results, pending_is_tail);
                    pending_is_tail = false;
                    let mut blocks = user_blocks(u);
                    if Some(index) == tail_break {
                        mark_last_block(&mut blocks);
                    }
                    messages.push(json!({"role": "user", "content": blocks}));
                }
                Message::Assistant(a) => {
                    flush_tool_results(&mut messages, &mut pending_tool_results, pending_is_tail);
                    pending_is_tail = false;
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
                    let mut message = tool_result_block(model, t);
                    if let Some(content) = message
                        .get_mut("content")
                        .and_then(serde_json::Value::as_array_mut)
                        && let Some(block) = content.pop()
                    {
                        pending_tool_results.push(block);
                    }
                    pending_is_tail |= Some(index) == tail_break;
                }
                Message::Custom(_) => {
                    flush_tool_results(&mut messages, &mut pending_tool_results, pending_is_tail);
                    pending_is_tail = false;
                }
            }
        }
        flush_tool_results(&mut messages, &mut pending_tool_results, pending_is_tail);

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
        let policy = crate::cache_policy::CachePolicy::DEFAULT;
        if !ctx.system_prompt.is_empty() {
            let mut block = json!({"type": "text", "text": ctx.system_prompt});
            if policy.mark_last_system {
                block["cache_control"] = crate::cache_policy::ephemeral();
            }
            body["system"] = json!([block]);
        }
        if !tools.is_empty() {
            let mut tools = tools;
            if policy.mark_last_tool
                && let Some(last) = tools.last_mut()
            {
                last["cache_control"] = crate::cache_policy::ephemeral();
            }
            body["tools"] = json!(tools);
        }
        // Thinking configuration (pi anthropic-messages.ts:1049-1073). Every
        // Anthropic-API model in the generated catalog is the adaptive-thinking
        // generation (Claude 4.5+); `Off` sends nothing (API default is off).
        // ponytail: budget-based `{"type":"enabled","budget_tokens":N}` returns
        // here if a pre-adaptive Claude row ever enters the catalog.
        if ctx.thinking_level.enabled() {
            body["thinking"] = json!({"type": "adaptive", "display": "summarized"});
            if let Some(effort) = anthropic_effort(ctx.thinking_level) {
                body["output_config"] = json!({ "effort": effort });
            }
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
                        st.append_text(text);
                        Some(vec![StreamEvent::TextDelta {
                            index: 0,
                            delta: text.to_string(),
                        }])
                    }
                    "thinking_delta" => {
                        let text = data.pointer("/delta/thinking").and_then(|v| v.as_str())?;
                        st.append_thinking(text);
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
                let message = st.failure_message(format!("anthropic stream error: {detail}"));
                Some(vec![StreamEvent::Done { message }])
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

/// A user turn's blocks, as the wire wants them.
///
/// Split out from the loop so the cache marker has something to attach to that is the
/// same shape the unmarked turn would have had. A helper that produced a *different*
/// block shape depending on the marker would make the wire untestable.
fn user_blocks(u: &epsilon_contracts::message::UserMessage) -> Vec<serde_json::Value> {
    vec![json!({"type": "text", "text": user_text(u)})]
}

/// Put `cache_control` on the last block of `blocks`, if there is one.
///
/// Guarded rather than assumed: an empty user turn must not gain a marker on a block that
/// does not exist, and a silently dropped marker is exactly the failure this policy exists
/// to stop costing money.
fn mark_last_block(blocks: &mut [serde_json::Value]) {
    if let Some(last) = blocks.last_mut() {
        last["cache_control"] = crate::cache_policy::ephemeral();
    }
}

/// Flush grouped tool-result blocks as one user-role turn.
///
/// `mark_tail` is true when the last message before the flush was the cache
/// boundary; the marker belongs on the last block of the group, not on an
/// earlier sibling.
fn flush_tool_results(
    messages: &mut Vec<serde_json::Value>,
    pending: &mut Vec<serde_json::Value>,
    mark_tail: bool,
) {
    if pending.is_empty() {
        return;
    }
    if mark_tail && let Some(last) = pending.last_mut() {
        last["cache_control"] = crate::cache_policy::ephemeral();
    }
    messages.push(json!({
        "role": "user",
        "content": std::mem::take(pending),
    }));
}

/// The index of the **last user-role turn**: a user message or a tool-result
/// message, because both are sent under `"role": "user"`.
///
/// Last, not first: the point is a prefix that does not move while the turn's own tool
/// round-trips extend the transcript after it. Tool results are included because pi
/// marks the last user/system message on the wire (`anthropic-messages.ts:1529-1553`);
/// stopping at the prompt leaves every result after it uncached.
fn last_user_turn_index(messages: &[Message]) -> Option<usize> {
    messages
        .iter()
        .rposition(|m| matches!(m, Message::User(_) | Message::ToolResult(_)))
}

/// Tool results replay as a user turn carrying `tool_result` blocks (pi's
/// `convertMessages`). Multiple consecutive results collapse into one turn.
///
/// Text-only results keep the historical string shape byte-for-byte.
/// Results with images/PDFs (ADR-055) carry native blocks: a text part first
/// (the shared lead when image-only), then one `image` block per image and
/// one `document` block per PDF. Blind/oversize attachments never reach here
/// as bytes — the shared helper replaced them with placeholder words.
fn tool_result_block(model: &str, t: &ToolResultMessage) -> serde_json::Value {
    let has_media = t
        .content
        .iter()
        .any(|c| matches!(c, ContentBlock::Image(_)));
    if !has_media {
        let body: String = t
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": t.tool_call_id,
                "content": body,
                "is_error": t.is_error,
            }],
        });
    }
    let (text, images, documents) = super::tool_media_for(model, t);
    let mut parts = Vec::new();
    parts.push(json!({
        "type": "text",
        "text": if text.is_empty() { super::TOOL_IMAGE_LEAD.to_string() } else { text },
    }));
    for img in &images {
        parts.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": img.mime,
                "data": img.data,
            },
        }));
    }
    for doc in &documents {
        parts.push(json!({
            "type": "document",
            "source": {
                "type": "base64",
                "media_type": doc.mime,
                "data": doc.data,
            },
        }));
    }
    json!({
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": t.tool_call_id,
            "content": parts,
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

/// Map the shared ladder onto Anthropic effort values (pi `mapAnthropicEffort`).
/// `Max` has no native value yet and is dropped (adaptive decides on its own).
fn anthropic_effort(level: epsilon_contracts::ThinkingLevel) -> Option<&'static str> {
    use epsilon_contracts::ThinkingLevel as T;
    match level {
        T::Off | T::Max => None,
        T::Minimal | T::Low => Some("low"),
        T::Medium => Some("medium"),
        T::High => Some("high"),
        T::XHigh => Some("xhigh"),
    }
}

/// Claude Code identity line pi PREPENDS to the system prompt on the OAuth
/// branch (`buildParams`: `params.system` starts with this text, caller prompt
/// follows in the same cache-controlled block).
pub const OAUTH_SYSTEM_IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_fn::LlmContext;
    use epsilon_contracts::content::ImageRef;
    use epsilon_contracts::message::{Message, ToolResultMessage};
    use epsilon_contracts::new_id;

    const PNG_BYTES: &str = "iVBORw0KGgo=";
    const PDF_BYTES: &str = "JVBERi0=";

    fn image_result(text: &str, mime: &str, data: &str) -> LlmContext {
        LlmContext {
            system_prompt: String::new(),
            messages: vec![Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "read".into(),
                content: vec![
                    ContentBlock::text(text),
                    ContentBlock::Image(ImageRef {
                        mime: mime.to_string(),
                        data: data.to_string(),
                    }),
                ],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })],
            tools: vec![],
            thinking_level: epsilon_contracts::ThinkingLevel::Off,
        }
    }

    #[test]
    fn tool_result_text_plus_image_emits_native_blocks() {
        let body = AnthropicWire
            .request_body(
                "claude-haiku-4-5",
                &image_result("ok", "image/png", PNG_BYTES),
            )
            .expect("body");
        let s = body.to_string();
        assert!(
            s.contains(PNG_BYTES),
            "native image bytes missing from tool_result: {s}"
        );
        assert!(
            s.contains("\"type\":\"image\"") && s.contains("\"media_type\":\"image/png\""),
            "native Anthropic image block shape missing: {s}"
        );
    }

    #[test]
    fn tool_result_pdf_emits_document_shape_naming_mime() {
        let body = AnthropicWire
            .request_body(
                "claude-haiku-4-5",
                &image_result("ok", "application/pdf", PDF_BYTES),
            )
            .expect("body");
        let s = body.to_string();
        assert!(
            s.contains("application/pdf"),
            "PDF mime missing from tool_result: {s}"
        );
    }

    #[test]
    fn image_only_tool_result_keeps_see_attached_lead() {
        let ctx = LlmContext {
            system_prompt: String::new(),
            messages: vec![Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "read".into(),
                content: vec![ContentBlock::Image(ImageRef {
                    mime: "image/png".into(),
                    data: PNG_BYTES.into(),
                })],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })],
            tools: vec![],
            thinking_level: epsilon_contracts::ThinkingLevel::Off,
        };
        let body = AnthropicWire
            .request_body("claude-haiku-4-5", &ctx)
            .expect("body");
        let s = body.to_string();
        assert!(
            s.contains("(see attached image)"),
            "image-only lead missing: {s}"
        );
        assert!(s.contains(PNG_BYTES), "image bytes missing: {s}");
    }
}
