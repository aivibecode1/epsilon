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
                for t in pending.drain(..) {
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
/// actually routes): OpenAI-style `reasoning_effort` for gpt/o-series/xai,
/// nested `reasoning.effort` for OpenRouter, `enable_thinking` for zai,
/// `thinking.type` for deepseek. Unknown ids get plain `reasoning_effort`
/// only when the level is on; `Off` sends nothing (API defaults are off).
fn apply_thinking(
    body: &mut serde_json::Value,
    model: &str,
    level: epsilon_contracts::ThinkingLevel,
) {
    if !level.enabled() {
        return;
    }
    let effort = effort_str(level);
    if model.starts_with("openrouter/") || model.contains("/openrouter/") {
        body["reasoning"] = json!({ "effort": effort });
    } else if model.starts_with("deepseek") && model != "deepseek-chat" {
        // deepseek-reasoner / v4 reasoning rows take a thinking switch + effort.
        body["thinking"] = json!({"type": "enabled"});
        body["reasoning_effort"] = json!(effort);
    } else if model.starts_with("glm") || model.starts_with("zai/") {
        // Z.ai GLM family: explicit enable + effort.
        body["thinking"] = json!({"type": "enabled", "clear_thinking": false});
        body["reasoning_effort"] = json!(effort);
    } else if model.starts_with("kimi") {
        // Moonshot kimi-thinking rows honor effort; others ignore it harmlessly.
        if model.contains("thinking") {
            body["reasoning_effort"] = json!(effort);
        }
    } else if ["o1", "o3", "o4"].iter().any(|p| model.starts_with(p)) || model.starts_with("gpt-5")
    {
        body["reasoning_effort"] = json!(effort);
    } else {
        // Unknown/other compat models: send the standard knob when enabled;
        // providers that lack it reject loudly rather than silently misroute
        // (same posture as pi's default branch).
        body["reasoning_effort"] = json!(effort);
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
}
