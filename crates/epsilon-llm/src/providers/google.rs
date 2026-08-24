//! Google Gemini `generateContent` over direct HTTP (pi port).
//!
//! Ported from pi-0.84.2 `packages/ai/src/api/google-generative-ai.ts`: key travels in
//! the `x-goog-api-key` header, streaming hits
//! `models/{model}:streamGenerateContent?alt=sse`, the system prompt rides
//! `systemInstruction`, and tool calls stream back as inline `functionCall` parts.

use super::{HttpEventStream, ProviderWire};
use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::message::{Message, StopReason};
use epsilon_contracts::stream::StreamEvent;
use epsilon_contracts::tool::ToolDescriptor;
use serde_json::json;

/// Unit struct implementing the Google Generative Language wire (fixture/test visibility).
pub struct GoogleWire;

impl ProviderWire for GoogleWire {
    fn endpoint(&self, model: &str) -> String {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse"
        )
    }

    fn env_key(&self) -> &'static str {
        "GEMINI_API_KEY"
    }

    fn headers(&self, key: &str) -> Vec<(String, String)> {
        vec![("x-goog-api-key".to_string(), key.to_string())]
    }

    fn request_body(
        &self,
        _model: &str,
        ctx: &crate::LlmContext,
    ) -> Result<serde_json::Value, String> {
        let mut contents = Vec::new();
        // Gemini has no cross-turn tool plumbing like OpenAI's role:"tool": results
        // replay as functionResponse parts inside user turns (pi's convertToGemini).
        let mut pending: Vec<(String, serde_json::Value)> = Vec::new();
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
                    contents.push(json!({"role": "user", "parts": [{"text": text}]}));
                }
                Message::Assistant(a) => {
                    let mut parts = Vec::new();
                    for b in &a.content {
                        match b {
                            ContentBlock::Text { text } if !text.is_empty() => {
                                parts.push(json!({"text": text}));
                            }
                            ContentBlock::ToolCall {
                                name, arguments, ..
                            } => {
                                parts.push(
                                    json!({"functionCall": {"name": name, "args": arguments}}),
                                );
                            }
                            _ => {}
                        }
                    }
                    if !parts.is_empty() {
                        contents.push(json!({"role": "model", "parts": parts}));
                    }
                }
                Message::ToolResult(t) => {
                    let body: serde_json::Value = t
                        .content
                        .iter()
                        .find_map(|c| match c {
                            ContentBlock::Text { text } => serde_json::from_str(text).ok(),
                            _ => None,
                        })
                        .unwrap_or_else(|| {
                            let joined: String = t
                                .content
                                .iter()
                                .filter_map(|c| match c {
                                    ContentBlock::Text { text } => Some(text.as_str()),
                                    _ => None,
                                })
                                .collect();
                            json!({"output": joined})
                        });
                    pending.push((t.tool_call_id.clone(), body));
                }
                Message::Custom(_) => {}
            }
        }
        if !pending.is_empty() {
            let parts: Vec<serde_json::Value> = pending
                .into_iter()
                .map(|(name, response)| json!({"functionResponse": {"name": name, "response": response}}))
                .collect();
            contents.push(json!({"role": "user", "parts": parts}));
        }

        let mut body = json!({ "contents": contents });
        if !ctx.system_prompt.is_empty() {
            body["systemInstruction"] = json!({"parts": [{"text": ctx.system_prompt}]});
        }
        if !ctx.tools.is_empty() {
            let decls: Vec<serde_json::Value> = ctx
                .tools
                .iter()
                .map(|d: &ToolDescriptor| {
                    json!({
                        "name": d.name,
                        "description": d.description,
                        "parameters": d.parameters_schema,
                    })
                })
                .collect();
            body["tools"] = json!([{"functionDeclarations": decls}]);
        }
        Ok(body)
    }

    fn on_event(&self, ev: &super::SseEvent, st: &mut HttpEventStream) -> Option<Vec<StreamEvent>> {
        let v: serde_json::Value = serde_json::from_str(&ev.data).ok()?;
        let candidate = v.pointer("/candidates/0")?;
        // Accumulate across ALL parts: a single chunk can carry text AND a functionCall
        // (model narrates then calls) — returning on the first text part would drop
        // the tool call and any usage metadata in the same frame.
        let mut batch: Vec<StreamEvent> = Vec::new();
        for part in candidate
            .pointer("/content/parts")
            .and_then(|p| p.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                batch.push(StreamEvent::TextDelta {
                    index: 0,
                    delta: text.to_string(),
                });
                continue;
            }
            if let Some(thought) = part.get("thought").and_then(|t| t.as_bool()).filter(|b| *b) {
                let _ = thought;
            }
            if let Some(call) = part.get("functionCall") {
                let name = call
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown_tool");
                let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
                let idx = st.open_tool_call(name.to_string());
                st.feed_tool_json_at(idx, &args.to_string());
                st.close_tool_call();
            }
        }
        // usageMetadata appears on chunks once generation completes.
        if let Some(usage) = v.get("usageMetadata").filter(|u| !u.is_null()) {
            st.seed_usage_compat(usage);
        }
        let done = candidate
            .get("finishReason")
            .and_then(|f| f.as_str())
            .is_some();
        if done {
            let wire = candidate.get("finishReason").and_then(|f| f.as_str());
            let has_calls = st.has_open_or_closed_tools();
            let (stop, err) = self.map_stop(wire, has_calls);
            batch.push(StreamEvent::Done {
                message: st.finish(stop, err),
            });
        }
        if batch.is_empty() { None } else { Some(batch) }
    }

    fn map_stop(&self, wire: Option<&str>, has_tool_calls: bool) -> (StopReason, Option<String>) {
        match wire {
            Some("MAX_TOKENS") => (
                StopReason::MaxTokens,
                Some("hit the max output token limit".into()),
            ),
            Some("SAFETY") | Some("PROHIBITED_CONTENT") | Some("BLOCKLIST") => {
                (StopReason::Error, Some("blocked by safety filters".into()))
            }
            Some("RECITATION") => (StopReason::Error, Some("recitation block".into())),
            _ if has_tool_calls => (StopReason::ToolUse, None),
            _ => (StopReason::EndTurn, None),
        }
    }

    fn finalize_usage_from_tail(&self) -> bool {
        false
    }
}
