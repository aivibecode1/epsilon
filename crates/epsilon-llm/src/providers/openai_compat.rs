//! OpenAI-compatible Chat Completions over direct HTTP (pi port).
//!
//! Ported from pi-0.84.2 `packages/ai/src/api/openai-completions.ts`: `/chat/completions`
//! with `Bearer` auth, `stream_options: {"include_usage": true}` so the final chunk
//! carries usage, tool calls as `tool_calls` arrays with indexed JSON-argument
//! fragments, and `[DONE]` termination. One implementation serves every
//! OpenAI-compatible provider we catalog (OpenAI, DeepSeek, Groq, xAI, Moonshot,
//! Z.ai, OpenRouter, local Ollama) — pi's "same API, different endpoint/key" model.

use super::{HttpEventStream, ProviderWire};
use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::message::{Message, StopReason};
use epsilon_contracts::stream::StreamEvent;
use epsilon_contracts::tool::ToolDescriptor;
use serde_json::json;

/// OpenAI-compatible `/chat/completions` wire descriptor (fixture/test visibility).
pub struct OpenAiCompatWire {
    /// Base URL ending without a trailing slash (pi providers' `baseUrl` field).
    /// Owned since M28 so user-declared custom endpoints can ride the same wire.
    pub base_url: String,
    /// Auth env var (`None` for local Ollama). Owned for the same reason.
    pub env_key: Option<String>,
}

impl OpenAiCompatWire {
    fn auth_header(&self, key: &str) -> Option<(String, String)> {
        self.env_key
            .as_ref()
            .map(|_| ("Authorization".to_string(), format!("Bearer {key}")))
    }
}

impl ProviderWire for OpenAiCompatWire {
    fn endpoint(&self, _model: &str) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn env_key(&self) -> &str {
        self.env_key.as_deref().unwrap_or("")
    }

    fn headers(&self, key: &str) -> Vec<(String, String)> {
        match self.auth_header(key) {
            Some(h) => vec![h],
            None => vec![],
        }
    }

    fn request_body(
        &self,
        model: &str,
        ctx: &crate::LlmContext,
    ) -> Result<serde_json::Value, String> {
        let mut messages = Vec::new();
        if !ctx.system_prompt.is_empty() {
            messages.push(json!({"role": "system", "content": ctx.system_prompt}));
        }
        // OpenAI's wire requires each assistant `tool_calls` to be immediately
        // followed by its `role: "tool"` results — buffering all tools to the end
        // (as an earlier draft did) makes every non-trivial history fail with
        // `invalid_request_error: tool_calls must be followed by tool messages`.
        let mut pending_tool_results: Vec<&epsilon_contracts::message::ToolResultMessage> =
            Vec::new();
        let flush_pending =
            |messages: &mut Vec<serde_json::Value>,
             pending: &mut Vec<&epsilon_contracts::message::ToolResultMessage>| {
                // Forwarded images across this batch: at most ONE follow-up
                // user message carries them all (ADR-055).
                let mut follow_images: Vec<super::ToolAttachment> = Vec::new();
                for t in pending.drain(..) {
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
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": t.tool_call_id,
                            "content": body,
                        }));
                        continue;
                    }
                    // Text stays in the tool message; images ride the single
                    // follow-up. This wire carries no document shape, so PDFs
                    // become the shared sized placeholder (mime + bytes).
                    let (mut text, images, documents) = super::tool_media_for(model, t);
                    for doc in &documents {
                        let note = super::sized_document_placeholder(&doc.mime, doc.data.len());
                        if text.is_empty() {
                            text = note;
                        } else {
                            text.push('\n');
                            text.push_str(&note);
                        }
                    }
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": t.tool_call_id,
                        "content": text,
                    }));
                    follow_images.extend(images);
                }
                if !follow_images.is_empty() {
                    let mut parts = vec![json!({
                        "type": "text",
                        "text": super::TOOL_IMAGE_FOLLOWUP_HEADER,
                    })];
                    for img in &follow_images {
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{};base64,{}", img.mime, img.data),
                            },
                        }));
                    }
                    messages.push(json!({"role": "user", "content": parts}));
                }
            };
        for m in &ctx.messages {
            match m {
                Message::User(u) => {
                    flush_pending(&mut messages, &mut pending_tool_results);
                    let text = u
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            epsilon_contracts::content::UserContent::Text { text } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    messages.push(json!({"role": "user", "content": text}));
                }
                Message::Assistant(a) => {
                    flush_pending(&mut messages, &mut pending_tool_results);
                    let calls: Vec<serde_json::Value> = a
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::ToolCall {
                                id,
                                name,
                                arguments,
                            } => Some(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(arguments)
                                        .unwrap_or_else(|_| "{}".into()),
                                },
                            })),
                            _ => None,
                        })
                        .collect();
                    let text = a.text();
                    if calls.is_empty() {
                        messages.push(json!({"role": "assistant", "content": text}));
                    } else {
                        let content = if text.is_empty() { None } else { Some(text) };
                        messages.push(json!({
                            "role": "assistant",
                            "content": content,
                            "tool_calls": calls,
                        }));
                    }
                }
                Message::ToolResult(_) => {
                    pending_tool_results.push(match m {
                        Message::ToolResult(t) => t,
                        _ => unreachable!("arm matched ToolResult"),
                    });
                }
                Message::Custom(_) => {}
            }
        }
        flush_pending(&mut messages, &mut pending_tool_results);

        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": true,
            // pi: request usage explicitly; several providers omit it otherwise.
            "stream_options": {"include_usage": true},
        });
        apply_thinking(&mut body, model, ctx.thinking_level);
        if !ctx.tools.is_empty() {
            let tools: Vec<serde_json::Value> = ctx
                .tools
                .iter()
                .map(|d: &ToolDescriptor| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": d.name,
                            "description": d.description,
                            "parameters": d.parameters_schema,
                        },
                    })
                })
                .collect();
            body["tools"] = json!(tools);
        }
        Ok(body)
    }

    fn on_event(&self, ev: &super::SseEvent, st: &mut HttpEventStream) -> Option<Vec<StreamEvent>> {
        let data = ev.data.trim();
        if data == "[DONE]" {
            let has_calls = st.has_open_or_closed_tools();
            let (stop, err) = self.map_stop(st.finish_reason(), has_calls);
            return Some(vec![StreamEvent::Done {
                message: st.finish(stop, err),
            }]);
        }
        let v: serde_json::Value = serde_json::from_str(data).ok()?;
        // Non-chat payloads (some proxies emit errors as bare JSON).
        if let Some(err_msg) = v.pointer("/error/message").and_then(|x| x.as_str()) {
            // M62: an error frame can arrive after real deltas — the accumulator
            // decides whether anything survives (salvage discipline).
            let message = st.failure_message(format!("provider stream error: {err_msg}"));
            return Some(vec![StreamEvent::Done { message }]);
        }
        for choice in v.get("choices").and_then(|c| c.as_array())?.iter() {
            if let Some(delta) = choice.get("delta") {
                if let Some(text) = delta.get("content").and_then(|t| t.as_str())
                    && !text.is_empty()
                {
                    st.append_text(text);
                    return Some(vec![StreamEvent::TextDelta {
                        index: 0,
                        delta: text.to_string(),
                    }]);
                }
                if let Some(reasoning) = delta.get("reasoning_content").and_then(|t| t.as_str())
                    && !reasoning.is_empty()
                {
                    st.append_thinking(reasoning);
                    return Some(vec![StreamEvent::ThinkingDelta {
                        index: 0,
                        delta: reasoning.to_string(),
                    }]);
                }
                if let Some(calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                    for call in calls {
                        let idx = call.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                        if let (Some(id), Some(name)) = (
                            call.get("id").and_then(|x| x.as_str()),
                            call.pointer("/function/name").and_then(|x| x.as_str()),
                        ) {
                            st.open_tool_call_at(idx, id.to_string(), name.to_string());
                        }
                        if let Some(frag) =
                            call.pointer("/function/arguments").and_then(|x| x.as_str())
                        {
                            st.feed_tool_json_at(idx, frag);
                        }
                    }
                }
            }
            if let Some(finish) = choice.get("finish_reason").and_then(|f| f.as_str()) {
                st.note_finish_reason(finish);
            }
        }
        // Usage rides the last chunk when stream_options.include_usage is honored.
        if let Some(usage) = v.get("usage").filter(|u| !u.is_null()) {
            st.seed_usage_compat(usage);
        }
        None
    }

    fn map_stop(&self, wire: Option<&str>, has_tool_calls: bool) -> (StopReason, Option<String>) {
        match wire {
            Some("length") => (
                StopReason::MaxTokens,
                Some("hit the max output token limit".into()),
            ),
            Some("content_filter") => (StopReason::Error, Some("content filter".into())),
            _ if has_tool_calls => (StopReason::ToolUse, None),
            _ => (StopReason::EndTurn, None),
        }
    }

    fn finalize_usage_from_tail(&self) -> bool {
        false
    }
}

/// Apply the shared thinking ladder to an OpenAI-compatible request body
/// (pi openai-completions.ts:747-840, reduced to the families our catalog
/// actually routes). Wave D: the shape comes from the catalog row's
/// `reasoning_shape` capability field, never from id substrings. Unknown or
/// custom ids take `StandardEffort` — today's default branch, byte for byte.
fn apply_thinking(
    body: &mut serde_json::Value,
    model: &str,
    level: epsilon_contracts::ThinkingLevel,
) {
    if !level.enabled() {
        return;
    }
    let shape = crate::catalog::reasoning_shape_for(model)
        .unwrap_or(crate::catalog::ReasoningShape::StandardEffort);
    apply_thinking_for_shape(body, shape, level);
}

/// Shape → body bytes. Exhaustive on purpose (no wildcard arm): adding a
/// `ReasoningShape` variant fails to compile until both this wire and the
/// Mistral wire decide what it means on their protocol.
fn apply_thinking_for_shape(
    body: &mut serde_json::Value,
    shape: crate::catalog::ReasoningShape,
    level: epsilon_contracts::ThinkingLevel,
) {
    use crate::catalog::ReasoningShape as S;
    if !level.enabled() {
        return;
    }
    let effort = effort_str(level);
    match shape {
        S::None => {}
        S::StandardEffort | S::MistralEffort => {
            body["reasoning_effort"] = json!(effort);
        }
        S::OpenRouterReasoning => {
            body["reasoning"] = json!({ "effort": effort });
        }
        S::DeepSeekThinking => {
            // deepseek-reasoner / v4 reasoning rows take a thinking switch + effort.
            body["thinking"] = json!({"type": "enabled"});
            body["reasoning_effort"] = json!(effort);
        }
        S::GlmThinking => {
            // Z.ai GLM family: explicit enable + effort.
            body["thinking"] = json!({"type": "enabled", "clear_thinking": false});
            body["reasoning_effort"] = json!(effort);
        }
        // The Mistral wire owns this shape (`prompt_mode`); the compat
        // protocol has no knob for it. Unreachable via `wire_for`
        // (magistral rows ride the Mistral wire), so silence is the safe
        // answer rather than a guessed knob.
        S::MagistralPromptMode => {}
    }
}

/// Ladder -> OpenAI-style effort string (`xhigh` included; pi maps 1:1).
fn effort_str(level: epsilon_contracts::ThinkingLevel) -> &'static str {
    use epsilon_contracts::ThinkingLevel as T;
    match level {
        T::Off => "off",
        T::Minimal => "minimal",
        T::Low => "low",
        T::Medium => "medium",
        T::High => "high",
        T::XHigh | T::Max => "xhigh",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LlmContext;
    use crate::catalog::ReasoningShape;
    use epsilon_contracts::content::{ContentBlock, UserContent};
    use epsilon_contracts::message::{Message, ToolResultMessage, UserMessage};
    use epsilon_contracts::new_id;

    fn user(text: &str) -> Message {
        Message::User(UserMessage {
            id: new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: text.to_string(),
            }],
        })
    }

    fn assistant_tool_call(id: &str, name: &str) -> Message {
        Message::Assistant(epsilon_contracts::message::AssistantMessage {
            id: new_id("msg"),
            timestamp_ms: 0,
            provider: "test".into(),
            model_id: "test".into(),
            content: vec![ContentBlock::ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: serde_json::json!({}),
            }],
            stop_reason: epsilon_contracts::message::StopReason::ToolUse,
            error_message: None,
            usage: Default::default(),
        })
    }

    fn tool_result(id: &str, name: &str) -> Message {
        Message::ToolResult(ToolResultMessage {
            id: new_id("msg"),
            timestamp_ms: 0,
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            content: vec![ContentBlock::text("ok")],
            details: serde_json::json!({}),
            usage: None,
            is_error: false,
        })
    }

    #[test]
    fn request_body_interleaves_tool_results_immediately_after_their_assistant() {
        // History with two tool turns: assistant(tool A) -> result A -> assistant(tool B) -> result B -> user.
        // The OpenAI wire requires: assistant(A), tool(A), assistant(B), tool(B), user.
        // An earlier draft buffered all tools to the end (user last, tools trailing)
        // which DeepSeek rejects with `tool_calls must be followed by tool messages`.
        let msgs = vec![
            user("first question"),
            assistant_tool_call("tc_1", "bash"),
            tool_result("tc_1", "bash"),
            assistant_tool_call("tc_2", "bash"),
            tool_result("tc_2", "bash"),
            user("say hi now"),
        ];
        let ctx = LlmContext {
            system_prompt: "sys".to_string(),
            messages: msgs,
            tools: vec![],
            thinking_level: epsilon_contracts::ThinkingLevel::Off,
        };
        let wire = OpenAiCompatWire {
            base_url: "https://api.example.com/v1".to_string(),
            env_key: Some("K".into()),
        };
        let body = wire.request_body("deepseek-chat", &ctx).expect("body");
        let got: Vec<String> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap().to_string())
            .collect();
        // system + user + assistant(tool) + tool + assistant(tool) + tool + user
        assert_eq!(
            got,
            vec![
                "system",
                "user",
                "assistant",
                "tool",
                "assistant",
                "tool",
                "user"
            ],
            "each tool must immediately follow its assistant (got {got:?})"
        );
        // And the tool ids ride in order.
        assert_eq!(body["messages"][3]["tool_call_id"], "tc_1");
        assert_eq!(body["messages"][5]["tool_call_id"], "tc_2");
    }

    fn fixture_row(id: &'static str, shape: ReasoningShape) -> crate::catalog::ModelInfo {
        crate::catalog::ModelInfo {
            id,
            provider: "fixture",
            label: "fixture",
            context_window: 1,
            max_output_tokens: 1,
            input_cost_micro_per_mtok: 0,
            output_cost_micro_per_mtok: 0,
            cache_read_cost_micro_per_mtok: 0,
            cache_write_cost_micro_per_mtok: 0,
            supports_tools: true,
            env_key: "",
            wire: crate::catalog::WireApi::AnthropicMessages,
            reasoning_shape: shape,
            supports_vision: true,
        }
    }

    fn shaped_body(
        shape: ReasoningShape,
        level: epsilon_contracts::ThinkingLevel,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({});
        apply_thinking_for_shape(&mut body, shape, level);
        body
    }

    /// The bytes follow the FIELD, not the id: two fixture rows with
    /// history-shaped ids but swapped fields route by field — including the
    /// direction history never produced (a deepseek id sending the plain
    /// knob, a magistral id sending deepseek bytes). The mapper never sees
    /// the id at all; lookup-then-map composition is pinned by
    /// `tests/reasoning_routing.rs` on real catalog ids.
    #[test]
    fn routing_follows_the_field_not_the_id() {
        use epsilon_contracts::ThinkingLevel as T;
        let row = fixture_row("deepseek-v4-pro", ReasoningShape::StandardEffort);
        let mut body = serde_json::json!({});
        apply_thinking_for_shape(&mut body, row.reasoning_shape, T::High);
        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("thinking").is_none(), "{body}");

        let row = fixture_row("magistral-medium-latest", ReasoningShape::DeepSeekThinking);
        let mut body = serde_json::json!({});
        apply_thinking_for_shape(&mut body, row.reasoning_shape, T::High);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "high");
    }

    /// Every variant maps to its documented bytes; `Off` silences all of them.
    #[test]
    fn every_shape_maps_to_its_documented_bytes() {
        use epsilon_contracts::ThinkingLevel as T;
        let std = shaped_body(ReasoningShape::StandardEffort, T::Medium);
        assert_eq!(std["reasoning_effort"], "medium");
        let mistral = shaped_body(ReasoningShape::MistralEffort, T::Medium);
        assert_eq!(mistral["reasoning_effort"], "medium");
        let or = shaped_body(ReasoningShape::OpenRouterReasoning, T::Medium);
        assert_eq!(or["reasoning"]["effort"], "medium");
        assert!(or.get("reasoning_effort").is_none(), "{or}");
        let ds = shaped_body(ReasoningShape::DeepSeekThinking, T::Low);
        assert_eq!(ds["thinking"]["type"], "enabled");
        assert_eq!(ds["reasoning_effort"], "low");
        let glm = shaped_body(ReasoningShape::GlmThinking, T::Low);
        assert_eq!(glm["thinking"]["clear_thinking"], false);
        assert_eq!(glm["reasoning_effort"], "low");
        for shape in [ReasoningShape::None, ReasoningShape::MagistralPromptMode] {
            let quiet = shaped_body(shape, T::High);
            assert!(
                quiet.as_object().is_some_and(|o| o.is_empty()),
                "{shape:?}: {quiet}"
            );
        }
        for shape in [
            ReasoningShape::StandardEffort,
            ReasoningShape::OpenRouterReasoning,
            ReasoningShape::DeepSeekThinking,
            ReasoningShape::GlmThinking,
            ReasoningShape::MistralEffort,
            ReasoningShape::MagistralPromptMode,
            ReasoningShape::None,
        ] {
            let off = shaped_body(shape, T::Off);
            assert!(
                off.as_object().is_some_and(|o| o.is_empty()),
                "{shape:?}: {off}"
            );
        }
    }

    /// Unknown/custom ids take the default branch (today's bytes, preserved).
    #[test]
    fn unknown_ids_take_the_default_branch() {
        let mut body = serde_json::json!({});
        apply_thinking(
            &mut body,
            "totally-unknown-model",
            epsilon_contracts::ThinkingLevel::Medium,
        );
        assert_eq!(body["reasoning_effort"], "medium");
        assert!(body.get("reasoning").is_none(), "{body}");
        assert!(body.get("thinking").is_none(), "{body}");
    }

    fn image_ctx(text: &str, mime: &str, data: &str) -> crate::LlmContext {
        use epsilon_contracts::content::ImageRef;
        crate::LlmContext {
            system_prompt: String::new(),
            messages: vec![
                assistant_tool_call("tc_1", "read"),
                Message::ToolResult(ToolResultMessage {
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
                }),
            ],
            tools: vec![],
            thinking_level: epsilon_contracts::ThinkingLevel::Off,
        }
    }

    fn compat_wire() -> OpenAiCompatWire {
        OpenAiCompatWire {
            base_url: "https://api.example.com/v1".to_string(),
            env_key: Some("K".into()),
        }
    }

    #[test]
    fn tool_result_text_plus_image_adds_one_follow_up_user_message() {
        let body = compat_wire()
            .request_body(
                "deepseek/deepseek-v4-flash-vision-exp",
                &image_ctx("ok", "image/png", "iVBORw0KGgo="),
            )
            .expect("body");
        let s = body.to_string();
        assert!(
            s.contains("iVBORw0KGgo=") && s.contains("image_url"),
            "follow-up user message with image_url bytes missing: {s}"
        );
        assert!(
            s.contains("Attached image(s) from tool result:"),
            "follow-up header vocabulary missing: {s}"
        );
        // Exactly ONE follow-up user message carries the image.
        let follow_ups = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "user" && m["content"].to_string().contains("image_url"))
            .count();
        assert_eq!(follow_ups, 1, "exactly one follow-up user message: {s}");
    }

    #[test]
    fn tool_result_pdf_becomes_sized_placeholder_naming_mime() {
        let body = compat_wire()
            .request_body(
                "deepseek/deepseek-v4-flash-vision-exp",
                &image_ctx("ok", "application/pdf", "JVBERi0="),
            )
            .expect("body");
        let s = body.to_string();
        assert!(
            s.contains("application/pdf"),
            "PDF mime missing (placeholder must name it): {s}"
        );
        assert!(
            !s.contains("JVBERi0="),
            "PDF base64 must never ride in a text field: {s}"
        );
    }

    #[test]
    fn blind_model_omits_bytes_with_placeholder() {
        let body = compat_wire()
            .request_body(
                "deepseek-flash",
                &image_ctx("ok", "image/png", "iVBORw0KGgo="),
            )
            .expect("body");
        let s = body.to_string();
        assert!(
            s.contains("(tool image omitted: model does not support images)"),
            "blind placeholder missing: {s}"
        );
        assert!(
            !s.contains("iVBORw0KGgo="),
            "blind model must receive no image bytes: {s}"
        );
    }
}
