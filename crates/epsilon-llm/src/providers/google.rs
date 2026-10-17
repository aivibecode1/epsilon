//! Google Gemini `generateContent` over direct HTTP (pi port).
//!
//! Ported from pi-0.84.2 `packages/ai/src/api/google-generative-ai.ts`: key travels in
//! the `x-goog-api-key` header, streaming hits
//! `models/{model}:streamGenerateContent?alt=sse`, the system prompt rides
//! `systemInstruction`, and tool calls stream back as inline `functionCall` parts.

use super::{HttpEventStream, ProviderWire};
use epsilon_contracts::message::StopReason;
use epsilon_contracts::stream::StreamEvent;
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
        model: &str,
        ctx: &crate::LlmContext,
    ) -> Result<serde_json::Value, String> {
        Ok(google_request_body(model, ctx))
    }

    fn on_event(&self, ev: &super::SseEvent, st: &mut HttpEventStream) -> Option<Vec<StreamEvent>> {
        google_on_event(ev, st)
    }

    fn map_stop(&self, wire: Option<&str>, has_tool_calls: bool) -> (StopReason, Option<String>) {
        google_map_stop(wire, has_tool_calls)
    }

    fn finalize_usage_from_tail(&self) -> bool {
        false
    }
}

/// Build the `generateContent` request body: shared by the API-key wire and
/// the Vertex wire (pi `buildParams` — request construction is identical, the
/// two only differ in endpoint + auth).
pub(crate) fn google_request_body(model: &str, ctx: &crate::LlmContext) -> serde_json::Value {
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::message::Message;
    use epsilon_contracts::tool::ToolDescriptor;

    let mut contents = Vec::new();
    // Gemini replays results as `functionResponse` parts inside user turns;
    // they must immediately follow their model turn, so flush eagerly.
    // Pending entries carry forwarded attachments with them: gemini-major>=3
    // inlines them beside the response (ADR-055), older majors take one
    // follow-up user turn (same `inlineData` shape, new turn).
    let flush_pending = |contents: &mut Vec<serde_json::Value>,
                         pending: &mut Vec<(
        String,
        serde_json::Value,
        Vec<super::ToolAttachment>,
        Vec<super::ToolAttachment>,
    )>| {
        if pending.is_empty() {
            return;
        }
        let inline = gemini_major(model).is_some_and(|m| m >= 3);
        if inline {
            let mut parts: Vec<serde_json::Value> = Vec::new();
            for (name, response, images, documents) in pending.drain(..) {
                parts.push(json!({"functionResponse": {"name": name, "response": response}}));
                for att in images.iter().chain(documents.iter()) {
                    parts.push(json!({"inlineData": {
                        "mimeType": att.mime,
                        "data": att.data,
                    }}));
                }
            }
            contents.push(json!({"role": "user", "parts": parts}));
        } else {
            let mut follow_images: Vec<super::ToolAttachment> = Vec::new();
            let mut follow_docs: Vec<super::ToolAttachment> = Vec::new();
            let parts: Vec<serde_json::Value> = pending
                .drain(..)
                .map(|(name, response, images, documents)| {
                    follow_images.extend(images);
                    follow_docs.extend(documents);
                    json!({"functionResponse": {"name": name, "response": response}})
                })
                .collect();
            contents.push(json!({"role": "user", "parts": parts}));
            if !follow_images.is_empty() || !follow_docs.is_empty() {
                let mut fparts = vec![json!({"text": super::TOOL_RESULT_IMAGE_LABEL})];
                for att in follow_images.iter().chain(follow_docs.iter()) {
                    fparts.push(json!({"inlineData": {
                        "mimeType": att.mime,
                        "data": att.data,
                    }}));
                }
                contents.push(json!({"role": "user", "parts": fparts}));
            }
        }
    };
    let mut pending: Vec<(
        String,
        serde_json::Value,
        Vec<super::ToolAttachment>,
        Vec<super::ToolAttachment>,
    )> = Vec::new();
    for m in &ctx.messages {
        match m {
            Message::User(u) => {
                flush_pending(&mut contents, &mut pending);
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
                flush_pending(&mut contents, &mut pending);
                let mut parts = Vec::new();
                for b in &a.content {
                    match b {
                        ContentBlock::Text { text } if !text.is_empty() => {
                            parts.push(json!({"text": text}));
                        }
                        ContentBlock::ToolCall {
                            name, arguments, ..
                        } => {
                            parts.push(json!({"functionCall": {"name": name, "args": arguments}}));
                        }
                        _ => {}
                    }
                }
                if !parts.is_empty() {
                    contents.push(json!({"role": "model", "parts": parts}));
                }
            }
            Message::ToolResult(t) => {
                let has_media = t
                    .content
                    .iter()
                    .any(|c| matches!(c, ContentBlock::Image(_)));
                if !has_media {
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
                    pending.push((t.tool_call_id.clone(), body, Vec::new(), Vec::new()));
                    continue;
                }
                // Attachments ride `inlineData` beside the response (ADR-055):
                // same turn on gemini-major>=3, one follow-up turn below.
                // PDFs are native here too (`mimeType: application/pdf`).
                let (text, images, documents) = super::tool_media_for(model, t);
                let show = if text.is_empty() {
                    super::TOOL_IMAGE_LEAD.to_string()
                } else {
                    text
                };
                let body: serde_json::Value =
                    serde_json::from_str(&show).unwrap_or_else(|_| json!({"output": show}));
                pending.push((t.tool_call_id.clone(), body, images, documents));
            }
            Message::Custom(_) => {}
        }
    }
    flush_pending(&mut contents, &mut pending);

    let mut body = json!({ "contents": contents });
    if !ctx.system_prompt.is_empty() {
        body["systemInstruction"] = json!({"parts": [{"text": ctx.system_prompt}]});
    }
    // Thinking config (pi google-generative-ai.ts:386-397): include thoughts
    // and pass the ladder level directly — Google's enum names match ours
    // (MINIMAL..HIGH). `Off` sends nothing; Gemini 3 Pro cannot disable
    // thinking anyway, so omitting the key is the honest default.
    if ctx.thinking_level.enabled() {
        body["generationConfig"] = json!({
            "thinkingConfig": {
                "includeThoughts": true,
                "thinkingLevel": google_level(ctx.thinking_level),
            }
        });
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
    body
}

/// Shared SSE frame mapping for both Google wires (pi's Vertex chunk loop is
/// the same `candidates[0].content.parts` walk as the API-key wire — the SDK
/// type is shared, and so is the code here).
pub(crate) fn google_on_event(
    ev: &super::SseEvent,
    st: &mut HttpEventStream,
) -> Option<Vec<StreamEvent>> {
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
            st.append_text(text);
            batch.push(StreamEvent::TextDelta {
                index: 0,
                delta: text.to_string(),
            });
            continue;
        }
        if let Some(text) = part.get("thought").and_then(|t| t.as_str()) {
            st.append_thinking(text);
            batch.push(StreamEvent::ThinkingDelta {
                index: 0,
                delta: text.to_string(),
            });
            continue;
        }
        if part.get("thought").and_then(|t| t.as_bool()).is_some() {
            let _ = part;
            continue;
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
        let (stop, err) = google_map_stop(wire, has_calls);
        batch.push(StreamEvent::Done {
            message: st.finish(stop, err),
        });
    }
    if batch.is_empty() { None } else { Some(batch) }
}

/// Shared stop mapping for both Google wires (same `finishReason` vocabulary).
pub(crate) fn google_map_stop(
    wire: Option<&str>,
    has_tool_calls: bool,
) -> (StopReason, Option<String>) {
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

/// Major generation of a Gemini model id (ADR-055).
///
/// Parses the `gemini-<major>` prefix of the bare id (`vertex/…` qualified
/// ids strip to the bare form first, mirroring `reasoning_shape_for`).
/// `gemini-2.5-flash` → 2, `gemini-3-flash-preview` → 3. `None` when the id
/// is not a Gemini id at all — callers treat unknown as the older shape
/// (a follow-up turn works on every generation; inlining does not).
pub(crate) fn gemini_major(model_id: &str) -> Option<u64> {
    let bare = model_id
        .split_once('/')
        .map(|(_, rest)| rest)
        .unwrap_or(model_id);
    let rest = bare.strip_prefix("gemini-")?;
    let major: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if major.is_empty() {
        return None;
    }
    major.parse().ok()
}

/// Ladder -> Google `thinkingLevel` enum (uppercase; xhigh/max clamp to HIGH,
/// pi's google-shared clamping for models that lack those values).
pub(crate) fn google_level(level: epsilon_contracts::ThinkingLevel) -> &'static str {
    use epsilon_contracts::ThinkingLevel as T;
    match level {
        T::Off => "OFF",
        T::Minimal => "MINIMAL",
        T::Low => "LOW",
        T::Medium => "MEDIUM",
        T::High | T::XHigh | T::Max => "HIGH",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_fn::LlmContext;
    use epsilon_contracts::content::{ContentBlock, ImageRef};
    use epsilon_contracts::message::{Message, ToolResultMessage};
    use epsilon_contracts::new_id;

    fn image_ctx(text: &str, mime: &str, data: &str) -> LlmContext {
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
    fn gemini3_inlines_in_same_turn_gemini2_uses_follow_up() {
        let wire = GoogleWire;
        let new_turn = wire
            .request_body(
                "gemini-3-flash-preview",
                &image_ctx("ok", "image/png", "iVBORw0KGgo="),
            )
            .expect("body");
        let s = new_turn.to_string();
        assert!(
            s.contains("inlineData") && s.contains("iVBORw0KGgo="),
            "gemini-3 must carry inlineData bytes: {s}"
        );
        let old_turn = wire
            .request_body(
                "gemini-2.5-flash",
                &image_ctx("ok", "image/png", "iVBORw0KGgo="),
            )
            .expect("body");
        let t = old_turn.to_string();
        assert!(
            t.contains("inlineData") && t.contains("iVBORw0KGgo="),
            "gemini-2 must still forward bytes (follow-up turn): {t}"
        );
        assert!(
            t.contains("Tool result image:"),
            "gemini-2 follow-up vocabulary missing: {t}"
        );
    }

    #[test]
    fn tool_result_pdf_names_mime_without_bytes_in_text() {
        let body = GoogleWire
            .request_body(
                "gemini-3-flash-preview",
                &image_ctx("ok", "application/pdf", "JVBERi0="),
            )
            .expect("body");
        let s = body.to_string();
        assert!(s.contains("application/pdf"), "PDF mime missing: {s}");
        assert!(
            !s.contains("\"text\":\"okJVBERi0="),
            "PDF base64 must never sit inside a text part: {s}"
        );
    }
}
