//! OpenAI Responses wire over direct HTTP (pi port).
//!
//! Ported from pi-0.84.2 `packages/ai/src/api/openai-responses-shared.ts`
//! (`processResponsesStream` + `convertResponsesMessages`) and
//! `openai-responses.ts` (endpoint + 16-token floor): `POST {base}/responses`
//! with `stream: true`, SSE frames of `response.*` events. Only the mainstream
//! event surface is ported — reasoning-encrypted content, custom/grammar tools,
//! deferred tool-search, and service-tier pricing stay out (documented below).
//!
//! pi routes this wire for the `openai`, `openai-codex`, `azure-openai-responses`
//! providers; here it is a free choice per catalog row (the same way
//! `OpenAiCompletions` serves every compat provider).

use super::{HttpEventStream, ProviderWire};
use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::message::{Message, StopReason};
use epsilon_contracts::stream::StreamEvent;
use epsilon_contracts::tool::ToolDescriptor;
use serde_json::json;

/// OpenAI Responses `/responses` wire descriptor.
pub struct OpenAiResponsesWire {
    /// Base URL ending without a trailing slash (pi providers' `baseUrl` field).
    pub base_url: String,
    /// Auth env var.
    pub env_key: Option<String>,
}

/// OpenAI Responses rejects `max_output_tokens` below 16 (pi, issue #6265).
pub const RESPONSES_MIN_OUTPUT_TOKENS: u64 = 16;

impl ProviderWire for OpenAiResponsesWire {
    fn endpoint(&self, _model: &str) -> String {
        format!("{}/responses", self.base_url)
    }

    fn env_key(&self) -> &str {
        self.env_key.as_deref().unwrap_or("")
    }

    fn headers(&self, key: &str) -> Vec<(String, String)> {
        if self.env_key.is_some() {
            vec![("Authorization".to_string(), format!("Bearer {key}"))]
        } else {
            vec![]
        }
    }

    fn request_body(
        &self,
        model: &str,
        ctx: &crate::LlmContext,
    ) -> Result<serde_json::Value, String> {
        // `input`: same rolling history shape as chat completions, but the
        // Responses item vocabulary (`input_text` / `output_text` /
        // `function_call` / `function_call_output`) instead of roles.
        let mut input: Vec<serde_json::Value> = Vec::new();
        if !ctx.system_prompt.is_empty() {
            input.push(json!({
                "role": "system",
                "content": [{"type": "input_text", "text": ctx.system_prompt}],
            }));
        }
        for m in &ctx.messages {
            match m {
                Message::User(u) => {
                    let mut parts: Vec<serde_json::Value> = Vec::new();
                    for c in &u.content {
                        match c {
                            epsilon_contracts::content::UserContent::Text { text } => {
                                parts.push(json!({"type": "input_text", "text": text}));
                            }
                            epsilon_contracts::content::UserContent::Image(img) => {
                                parts.push(json!({
                                    "type": "input_image",
                                    "detail": "auto",
                                    "image_url": format!("data:{};base64,{}", img.mime, img.data),
                                }));
                            }
                        }
                    }
                    if !parts.is_empty() {
                        input.push(json!({"role": "user", "content": parts}));
                    }
                }
                Message::Assistant(a) => {
                    for b in &a.content {
                        match b {
                            ContentBlock::Text { text } => {
                                input.push(json!({
                                    "type": "message",
                                    "role": "assistant",
                                    "content": [{"type": "output_text", "text": text}],
                                }));
                            }
                            ContentBlock::Thinking { text, .. } => {
                                input.push(json!({
                                    "type": "reasoning",
                                    "summary": [{"type": "summary_text", "text": text}],
                                }));
                            }
                            ContentBlock::ToolCall {
                                id,
                                name,
                                arguments,
                            } => {
                                // Responses tool ids are `call_id|item_id`; replay keeps
                                // the pair intact (pi `normalizeToolCallId`).
                                input.push(json!({
                                    "type": "function_call",
                                    "call_id": id,
                                    "name": name,
                                    "arguments": arguments.to_string(),
                                }));
                            }
                            ContentBlock::Image(img) => {
                                input.push(json!({
                                    "role": "user",
                                    "content": [{
                                        "type": "input_image",
                                        "detail": "auto",
                                        "image_url": format!("data:{};base64,{}", img.mime, img.data),
                                    }],
                                }));
                            }
                        }
                    }
                }
                Message::ToolResult(t) => {
                    let text = t
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    // `function_call_output` pairs with the call via `call_id`
                    // (the part before `|` when pi's pair form is present).
                    let call_id = t.tool_call_id.split('|').next().unwrap_or(&t.tool_call_id);
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": text,
                    }));
                }
                Message::Custom(_) => {}
            }
        }
        let tools: Vec<serde_json::Value> = ctx
            .tools
            .iter()
            .map(|t: &ToolDescriptor| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters_schema,
                })
            })
            .collect();
        let mut body = json!({
            "model": short_model(model),
            "input": input,
            "stream": true,
            "store": false,
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        // Conservative output ceiling from the catalog (Anthropic-wire parity:
        // every wire caps from the same source). Below pi's 16-token floor the
        // field is omitted — the API default applies instead of an error.
        let max_out = crate::catalog::lookup_model(model)
            .map(|m| m.max_output_tokens)
            .unwrap_or(16_384);
        if max_out >= RESPONSES_MIN_OUTPUT_TOKENS {
            body["max_output_tokens"] = json!(max_out);
        }
        Ok(body)
    }

    fn on_event(&self, ev: &super::SseEvent, st: &mut HttpEventStream) -> Option<Vec<StreamEvent>> {
        // Responses SSE: `event: <type>` + `data: {...}` per frame. The shared
        // decoder delivers `ev.name` (event type) + `ev.data` (JSON payload).
        let v: serde_json::Value = serde_json::from_str(ev.data.trim()).ok()?;
        match ev.name.as_deref() {
            Some("response.output_text.delta") => {
                let delta = v.get("delta")?.as_str()?;
                if delta.is_empty() {
                    return None;
                }
                st.append_text(delta);
                Some(vec![StreamEvent::TextDelta {
                    index: 0,
                    delta: delta.to_string(),
                }])
            }
            Some("response.reasoning_summary_text.delta" | "response.reasoning_text.delta") => {
                let delta = v.get("delta")?.as_str()?;
                if delta.is_empty() {
                    return None;
                }
                st.append_thinking(delta);
                Some(vec![StreamEvent::ThinkingDelta {
                    index: 0,
                    delta: delta.to_string(),
                }])
            }
            Some("response.function_call_arguments.delta") => {
                let idx = v.get("output_index")?.as_u64().unwrap_or(0) as usize;
                if let Some(delta) = v.get("delta").and_then(|d| d.as_str()) {
                    st.feed_tool_json_at(idx, delta);
                }
                None
            }
            Some("response.output_item.added") => {
                let item = v.get("item")?;
                let idx = v.get("output_index")?.as_u64().unwrap_or(0) as usize;
                match item.get("type")?.as_str()? {
                    "function_call" => {
                        // pi pairs `call_id|id` so replay can address both.
                        let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                        let item_id = item.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        let name = item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let paired = if item_id.is_empty() {
                            call_id.to_string()
                        } else {
                            format!("{call_id}|{item_id}")
                        };
                        st.open_tool_call_at(idx, paired, name);
                        if let Some(args) = item.get("arguments").and_then(|a| a.as_str()) {
                            st.feed_tool_json_at(idx, args);
                        }
                    }
                    "reasoning" | "message" => {}
                    _ => {}
                }
                None
            }
            Some("response.output_item.done") => {
                let item = v.get("item")?;
                let idx = v.get("output_index")?.as_u64().unwrap_or(0) as usize;
                if item.get("type")?.as_str()? != "function_call" {
                    return None;
                }
                if let Some(args) = item.get("arguments").and_then(|a| a.as_str()) {
                    // Final arguments snapshot wins over delta fragments (pi
                    // parses `item.arguments || partialJson` at done time).
                    st.set_tool_json_at(idx, args);
                }
                None
            }
            Some("response.completed" | "response.incomplete") => {
                if let Some(resp) = v.get("response") {
                    if let Some(usage) = resp.get("usage") {
                        st.seed_usage_compat(&responses_usage(usage));
                    }
                    let status = resp.get("status").and_then(|s| s.as_str());
                    let incomplete = resp
                        .get("incomplete_details")
                        .and_then(|d| d.get("reason"))
                        .and_then(|r| r.as_str())
                        .map(str::to_string);
                    let has_calls = st.has_open_or_closed_tools();
                    let (stop, err) = map_responses_stop(status, incomplete.as_deref(), has_calls);
                    st.note_finish_reason(status.unwrap_or("completed"));
                    return Some(vec![StreamEvent::Done {
                        message: st.finish(stop, err),
                    }]);
                }
                None
            }
            Some("response.failed") => {
                let msg = v
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("response failed without error details")
                    .to_string();
                let message = st.failure_message(format!("provider stream error: {msg}"));
                Some(vec![StreamEvent::Done { message }])
            }
            Some("error") => {
                let msg = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error")
                    .to_string();
                let message = st.failure_message(format!("provider stream error: {msg}"));
                Some(vec![StreamEvent::Done { message }])
            }
            _ => None,
        }
    }

    fn map_stop(&self, wire: Option<&str>, has_tool_calls: bool) -> (StopReason, Option<String>) {
        map_responses_stop(wire, None, has_tool_calls)
    }

    fn finalize_usage_from_tail(&self) -> bool {
        false
    }
}

/// Strip a custom-provider prefix (`lmstudio/qwen-...` → `qwen-...`) before
/// sending: the endpoint already identifies the provider.
fn short_model(model: &str) -> &str {
    model.split_once('/').map(|(_, rest)| rest).unwrap_or(model)
}

/// Normalize Responses `usage` (`input_tokens` / `output_tokens` /
/// `input_tokens`) into the compat key shape `seed_usage_compat` reads, together with the
/// cache counts this API nests one level deeper (`input_tokens_details`).
///
/// M140: `input_tokens_details` was previously dropped here, so on this wire `cache_read`
/// was always 0 — the header showed a confident `cache 0%` and the cached prefix was
/// billed at the full input rate with no floor marker. pi carries the same object and
/// subtracts both counts (`openai-responses-shared.ts:567`).
pub(crate) fn responses_usage(usage: &serde_json::Value) -> serde_json::Value {
    let mut out = json!({
        "input_tokens": usage.get("input_tokens").cloned().unwrap_or(json!(0)),
        "output_tokens": usage.get("output_tokens").cloned().unwrap_or(json!(0)),
        "total_tokens": usage.get("total_tokens").cloned().unwrap_or(json!(0)),
    });
    // Forwarded verbatim when present; `seed_usage_compat` reads the cached and
    // written counts out of it. Absent on a reply with no cache activity — and
    // left absent here, because inventing zero keys would turn silence into a
    // measured zero (D5: `CH 0%` instead of `CH n/a`).
    if let Some(details) = usage.get("input_tokens_details") {
        out["prompt_tokens_details"] = details.clone();
    }
    // Same forwarding for the reasoning split: the Responses API nests
    // `reasoning_tokens` in `output_tokens_details` (pi
    // `openai-responses-shared.ts:572`), and `seed_usage_compat` reads that
    // shape. Dropped here, the split would silently vanish — `reasoning` would
    // be `None` on a reply that reasoned. Absent stays absent (D5 again).
    if let Some(details) = usage.get("output_tokens_details") {
        out["output_tokens_details"] = details.clone();
    }
    out
}

/// pi `mapStopReason`: `completed` → stop, `incomplete/max_output_tokens` →
/// length, other incompletes + `failed`/`cancelled` → error. Tool calls still
/// upgrade a plain stop to ToolUse (the loop needs the batch, not a turn end).
fn map_responses_stop(
    status: Option<&str>,
    incomplete_reason: Option<&str>,
    has_tool_calls: bool,
) -> (StopReason, Option<String>) {
    match status {
        None => (StopReason::EndTurn, None),
        Some("completed" | "in_progress" | "queued") => {
            if has_tool_calls {
                (StopReason::ToolUse, None)
            } else {
                (StopReason::EndTurn, None)
            }
        }
        Some("incomplete") => {
            if incomplete_reason == Some("max_output_tokens") {
                (
                    StopReason::MaxTokens,
                    Some("hit the max output token limit".into()),
                )
            } else {
                (
                    StopReason::Error,
                    Some(
                        incomplete_reason
                            .map(|r| format!("Response incomplete: {r}"))
                            .unwrap_or_else(|| {
                                "Response incomplete without a provider reason".into()
                            }),
                    ),
                )
            }
        }
        Some("failed" | "cancelled") => {
            (StopReason::Error, Some(status.unwrap_or("failed").into()))
        }
        Some(other) => (
            StopReason::Error,
            Some(format!("unhandled response status: {other}")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::HttpEventStream;
    use crate::stream_fn::LlmContext;
    use epsilon_contracts::content::UserContent;
    use epsilon_contracts::message::{Message, UserMessage};
    use epsilon_contracts::new_id;

    fn ctx_with(text: &str) -> LlmContext {
        LlmContext {
            system_prompt: "sys".into(),
            messages: vec![Message::User(UserMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                content: vec![UserContent::Text { text: text.into() }],
            })],
            tools: vec![],
            thinking_level: epsilon_contracts::ThinkingLevel::Off,
        }
    }

    fn wire() -> OpenAiResponsesWire {
        OpenAiResponsesWire {
            base_url: "https://api.openai.com/v1".into(),
            env_key: Some("OPENAI_API_KEY".into()),
        }
    }

    fn sse(name: &str, data: &serde_json::Value) -> crate::http::SseEvent {
        crate::http::SseEvent {
            name: Some(name.into()),
            data: data.to_string(),
        }
    }

    #[test]
    fn request_body_uses_responses_vocabulary() {
        let body = wire()
            .request_body("gpt-5.6", &ctx_with("hi"))
            .expect("body");
        assert_eq!(body["model"], "gpt-5.6");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        // system + user, input_text vocabulary (never chat roles/content strings).
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["role"], "system");
        assert_eq!(input[1]["content"][0]["type"], "input_text");
        assert!(body.get("max_output_tokens").is_some(), "catalog ceiling");
    }

    #[test]
    fn stream_reassembles_text_tool_and_done() {
        let w = wire();
        let mut st = HttpEventStream::empty_for_tests();
        // Text delta accumulates and emits TextDelta.
        let ev = w
            .on_event(
                &sse(
                    "response.output_text.delta",
                    &serde_json::json!({"delta": "Hello"}),
                ),
                &mut st,
            )
            .expect("text batch");
        assert!(matches!(ev[0], StreamEvent::TextDelta { .. }));
        // Tool opens with a paired call_id|item_id, args stream in, then the
        // done snapshot replaces fragments.
        assert!(
            w.on_event(
                &sse(
                    "response.output_item.added",
                    &serde_json::json!({
                        "output_index": 0,
                        "item": {"type": "function_call", "call_id": "call_1",
                                 "id": "fc_1", "name": "read", "arguments": "{\"pa"}
                    }),
                ),
                &mut st,
            )
            .is_none()
        );
        assert!(
            w.on_event(
                &sse(
                    "response.function_call_arguments.delta",
                    &serde_json::json!({"output_index": 0, "delta": "th\"}"}),
                ),
                &mut st,
            )
            .is_none()
        );
        assert!(
            w.on_event(
                &sse(
                    "response.output_item.done",
                    &serde_json::json!({
                        "output_index": 0,
                        "item": {"type": "function_call",
                                 "arguments": "{\"path\":\"a.rs\"}"}
                    }),
                ),
                &mut st,
            )
            .is_none()
        );
        // Terminal completed with tool activity → ToolUse.
        let done = w
            .on_event(
                &sse(
                    "response.completed",
                    &serde_json::json!({"response": {
                        "status": "completed",
                        "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
                    }}),
                ),
                &mut st,
            )
            .expect("done batch");
        let StreamEvent::Done { message } = &done[0] else {
            panic!("expected Done, got {:?}", done[0]);
        };
        assert_eq!(message.stop_reason, StopReason::ToolUse);
        let calls: Vec<_> = message.tool_calls().collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "call_1|fc_1");
        assert_eq!(calls[0].1, "read");
        assert_eq!(calls[0].2, &serde_json::json!({"path": "a.rs"}));
        assert_eq!(message.usage.input, 10);
    }

    #[test]
    fn incomplete_and_failed_map_to_error() {
        let w = wire();
        let mut st = HttpEventStream::empty_for_tests();
        let done = w
            .on_event(
                &sse(
                    "response.incomplete",
                    &serde_json::json!({"response": {
                        "status": "incomplete",
                        "incomplete_details": {"reason": "max_output_tokens"},
                    }}),
                ),
                &mut st,
            )
            .expect("done batch");
        let StreamEvent::Done { message } = &done[0] else {
            panic!("expected Done");
        };
        assert_eq!(message.stop_reason, StopReason::MaxTokens);

        let failed = w
            .on_event(
                &sse("response.failed", &serde_json::json!({"response": {}})),
                &mut st,
            )
            .expect("done batch");
        let StreamEvent::Done { message } = &failed[0] else {
            panic!("expected Done");
        };
        assert_eq!(message.stop_reason, StopReason::Error);
        assert!(message.error_message.as_deref().unwrap().contains("failed"));
    }
}
