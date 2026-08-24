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
    pub base_url: &'static str,
    /// Auth env var (`None` for local Ollama).
    pub env_key: Option<&'static str>,
}

impl OpenAiCompatWire {
    fn auth_header(&self, key: &str) -> Option<(String, String)> {
        self.env_key
            .map(|_| ("Authorization".to_string(), format!("Bearer {key}")))
    }
}

impl ProviderWire for OpenAiCompatWire {
    fn endpoint(&self, _model: &str) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn env_key(&self) -> &'static str {
        self.env_key.unwrap_or("")
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
        // Pair assistant tool_calls with their results the way the wire expects:
        // assistant turn carries `tool_calls`, each result is a `role: "tool"` turn.
        let mut pending_tool_results: Vec<&epsilon_contracts::message::ToolResultMessage> =
            Vec::new();
        for m in &ctx.messages {
            match m {
                Message::User(u) => {
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
        // Flush buffered tool results as `role: "tool"` turns (they trail the
        // assistant turn that requested the calls).
        for t in pending_tool_results.drain(..) {
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

        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": true,
            // pi: request usage explicitly; several providers omit it otherwise.
            "stream_options": {"include_usage": true},
        });
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
            return Some(vec![StreamEvent::Done {
                message: crate::stream_fn::error_message(format!(
                    "provider stream error: {err_msg}"
                )),
            }]);
        }
        for choice in v.get("choices").and_then(|c| c.as_array())?.iter() {
            if let Some(delta) = choice.get("delta") {
                if let Some(text) = delta.get("content").and_then(|t| t.as_str())
                    && !text.is_empty()
                {
                    return Some(vec![StreamEvent::TextDelta {
                        index: 0,
                        delta: text.to_string(),
                    }]);
                }
                if let Some(reasoning) = delta.get("reasoning_content").and_then(|t| t.as_str())
                    && !reasoning.is_empty()
                {
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
