//! Amazon Bedrock Converse Stream over direct HTTP (pi port).
//!
//! Ported from pi-0.84.2 `packages/ai/src/api/bedrock-converse-stream.ts`.
//! Two halves, deliberately split the way pi splits them (SDK client vs event
//! mapping):
//!
//! 1. **Request shape** (`bedrock_request_body`): Converse request JSON —
//!    `modelId`, `messages` (user/assistant with `text` / `toolUse` /
//!    `toolResult` / `reasoningContent` blocks), `system` as a block list,
//!    `inferenceConfig`, `toolConfig` (`toolSpec` + `inputSchema.json`),
//!    `additionalModelRequestFields`. Tool results coalesce consecutive
//!    entries into ONE user message (Bedrock requires it); empty assistant
//!    turns are skipped; blank text blocks are dropped with `<empty>`
//!    placeholders where Bedrock requires non-empty content (pi
//!    `EMPTY_TEXT_PLACEHOLDER`). Tool ids are sanitized to `[a-zA-Z0-9_-]`,
//!    truncated to 64 chars (pi `normalizeToolCallId`).
//! 2. **Auth**: SigV4 (default chain: explicit keys → bearer token →
//!    `AWS_PROFILE` → ambient env) or `Authorization: Bearer` when a bearer
//!    token is configured (pi `useBearerToken`: `options.bearerToken ||
//!    options.apiKey || AWS_BEARER_TOKEN_BEDROCK`, requiring
//!    `bedrock:CallWithBearerToken`). Region: ARN-embedded > explicit >
//!    env (`AWS_REGION`/`AWS_DEFAULT_REGION`) > endpoint-derived > `us-east-1`
//!    (pi's exact precedence); `eu.`-prefixed ids route to eu-central-1.
//! 3. **Event mapping** (`bedrock_on_event`): framed JSON events
//!    (`messageStart` / `contentBlockStart` / `contentBlockDelta` /
//!    `contentBlockStop` / `messageStop` / `metadata` / exception shapes),
//!    indexed by `contentBlockIndex` with text/thinking/tool slots created
//!    on demand (text blocks have no `Start` frame — pi creates them in the
//!    delta handler). Usage: `inputTokens` / `outputTokens` /
//!    `cacheReadInputTokens` / `cacheWriteInputTokens` (+ `totalTokens` or
//!    recomputed). Stop vocabulary: `end_turn`/`stop_sequence` → stop,
//!    `max_tokens`/`model_context_window_exceeded` → length, `tool_use` →
//!    toolUse, anything else → error naming the reason (pi `mapStopReason`).
//!
//! What we deliberately do NOT port: the `@aws-sdk/client-bedrock-runtime`
//! client itself (SigV4 signing + binary event-stream framing are specified
//! in `auth::sign_bedrock_request` / the transport note below — we own the
//! dozen lines that matter instead of an SDK dependency); prompt-cache
//! `cachePoint` blocks (needs catalog cacheability fields that don't exist
//! yet); thinking-signature replay (we send reasoning text unsigned, which
//! Bedrock accepts — pi only adds signatures for Claude 4.x); proxy/HTTP2
//! handler knobs (`NodeHttpHandler` — reqwest owns our transport);
//! `onPayload`/`onResponse` interception hooks (no caller uses them here).

use super::{HttpEventStream, ProviderWire};
use epsilon_contracts::message::StopReason;
use epsilon_contracts::stream::StreamEvent;
use serde_json::json;

/// Bedrock Converse Stream wire descriptor (fixture/test visibility).
/// Region/endpoint resolve per call (ARN > env > model-id default — pi's
/// precedence) in `bedrock_wire_region` + the endpoint builder, so the wire
/// itself carries no config: every catalog row shares this unit struct.
pub struct BedrockConverseWire;

impl BedrockConverseWire {
    /// Standard regional runtime endpoint (pi's default when no explicit
    /// endpoint is pinned).
    pub fn endpoint_for(region: &str) -> String {
        format!("https://bedrock-runtime.{region}.amazonaws.com")
    }

    /// Region for a model id: `eu.` prefix → eu-central-1, else us-east-1
    /// (pi `getBedrockBaseUrl`; ARN regions are extracted by the caller —
    /// see `region_from_arn`).
    ///
    /// Wave D note: this reads the id's region QUALIFICATION, not its model
    /// family — region routing, which catalog rows (canonical, unqualified
    /// ids) cannot carry. It is deliberately not a `reasoning_shape`-style
    /// capability field, and the reasoning-routing scan test allowlists these
    /// two lines so a new family branch cannot hide beside them.
    pub fn default_region_for(model_id: &str) -> &'static str {
        if model_id.starts_with("eu.") {
            "eu-central-1"
        } else {
            "us-east-1"
        }
    }

    /// Extract the region from an inference-profile ARN
    /// (`arn:aws:bedrock:{region}:…` — pi's `arnRegionMatch`).
    pub fn region_from_arn(model_id: &str) -> Option<String> {
        let rest = model_id.strip_prefix("arn:aws:bedrock:")?;
        let region = rest.split(':').next()?;
        if region.is_empty() {
            return None;
        }
        Some(region.to_string())
    }

    /// Sanitize a tool-call id to Bedrock's `[a-zA-Z0-9_-]` ≤64 form
    /// (pi `normalizeToolCallId`).
    pub fn normalize_tool_call_id(id: &str) -> String {
        let sanitized: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if sanitized.len() > 64 {
            sanitized[..64].to_string()
        } else {
            sanitized
        }
    }
}

impl ProviderWire for BedrockConverseWire {
    fn endpoint(&self, model: &str) -> String {
        // Fixture/tests resolve us-east-1; live calls go through
        // `bedrock_wire_region` + explicit endpoints in the transport. The
        // catalog rows share this unit wire (region is per-call, not per-row).
        let base = BedrockConverseWire::endpoint_for("us-east-1");
        let bare = model.split_once('/').map(|(_, rest)| rest).unwrap_or(model);
        // Converse Stream REST shape (API_runtime_ConverseStream):
        // POST {base}/model/{modelId}/converse-stream
        format!("{base}/model/{bare}/converse-stream")
    }

    fn env_key(&self) -> &str {
        "AWS_BEARER_TOKEN_BEDROCK"
    }

    fn headers(&self, key: &str) -> Vec<(String, String)> {
        // The key here is the RESOLVED bearer token when bearer auth is in
        // play. SigV4 requests leave `key` empty and sign in the transport
        // (see `crate::auth::sign_bedrock_request`) — headers() then only
        // carries content negotiation, and signing adds the rest.
        if key.is_empty() {
            vec![("accept".to_string(), "application/json".to_string())]
        } else {
            vec![
                ("accept".to_string(), "application/json".to_string()),
                ("authorization".to_string(), format!("Bearer {key}")),
            ]
        }
    }

    fn request_body(
        &self,
        model: &str,
        ctx: &crate::LlmContext,
    ) -> Result<serde_json::Value, String> {
        Ok(bedrock_request_body(model, ctx))
    }

    fn on_event(&self, ev: &super::SseEvent, st: &mut HttpEventStream) -> Option<Vec<StreamEvent>> {
        bedrock_on_event(ev, st)
    }

    fn map_stop(&self, wire: Option<&str>, has_tool_calls: bool) -> (StopReason, Option<String>) {
        bedrock_map_stop(wire, has_tool_calls)
    }

    fn finalize_usage_from_tail(&self) -> bool {
        false
    }
}

/// Build the Converse request body (pi `commandInput`: `modelId`, `messages`,
/// `system`, `inferenceConfig`, `toolConfig`).
pub(crate) fn bedrock_request_body(model: &str, ctx: &crate::LlmContext) -> serde_json::Value {
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::message::Message;

    let bare = model.split_once('/').map(|(_, rest)| rest).unwrap_or(model);
    let mut messages: Vec<serde_json::Value> = Vec::new();
    let mut pending_results: Vec<serde_json::Value> = Vec::new();
    let flush_results = |messages: &mut Vec<serde_json::Value>,
                         pending: &mut Vec<serde_json::Value>| {
        if pending.is_empty() {
            return;
        }
        // Bedrock requires ALL tool results in a single user message.
        messages.push(json!({
            "role": "user",
            "content": std::mem::take(pending),
        }));
    };

    for m in &ctx.messages {
        match m {
            Message::User(u) => {
                flush_results(&mut messages, &mut pending_results);
                let mut content: Vec<serde_json::Value> = u
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        epsilon_contracts::content::UserContent::Text { text } => {
                            non_blank_text(text).map(|t| json!({"text": t}))
                        }
                        _ => None,
                    })
                    .collect();
                if content.is_empty() {
                    content.push(json!({"text": "<empty>"}));
                }
                messages.push(json!({"role": "user", "content": content}));
            }
            Message::Assistant(a) => {
                flush_results(&mut messages, &mut pending_results);
                let mut blocks: Vec<serde_json::Value> = Vec::new();
                for b in &a.content {
                    match b {
                        ContentBlock::Text { text } => {
                            if let Some(t) = non_blank_text(text) {
                                blocks.push(json!({"text": t}));
                            }
                        }
                        ContentBlock::Thinking { .. } => {
                            // Thinking is never replayed (pi strips it too);
                            // Bedrock reasoning blocks need signatures we
                            // don't keep — plain omission is accepted.
                        }
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            blocks.push(json!({
                                "toolUse": {
                                    "toolUseId": BedrockConverseWire::normalize_tool_call_id(id),
                                    "name": name,
                                    "input": arguments,
                                }
                            }));
                        }
                        _ => {}
                    }
                }
                // Skip empty assistant turns (aborted-request residue) —
                // Bedrock rejects empty content arrays.
                if !blocks.is_empty() {
                    messages.push(json!({"role": "assistant", "content": blocks}));
                }
            }
            Message::ToolResult(t) => {
                let has_media = t
                    .content
                    .iter()
                    .any(|c| matches!(c, ContentBlock::Image(_)));
                if !has_media {
                    let text = t
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let content = if text.trim().is_empty() {
                        json!([{"text": "<empty>"}])
                    } else {
                        json!([{"text": text}])
                    };
                    pending_results.push(json!({
                        "toolResult": {
                            "toolUseId": BedrockConverseWire::normalize_tool_call_id(&t.tool_call_id),
                            "content": content,
                            "status": if t.is_error { "error" } else { "success" },
                        }
                    }));
                    continue;
                }
                // Native Converse blocks (ADR-055): text first, then one
                // `image` block per image and one `document` block per PDF.
                // Blind/oversize attachments arrive here as placeholder words
                // already — the shared helper withheld their bytes.
                let (text, images, documents) = super::tool_media_for(model, t);
                let show = if text.is_empty() {
                    super::TOOL_IMAGE_LEAD.to_string()
                } else {
                    text
                };
                let mut content = vec![json!({"text": show})];
                for img in &images {
                    match bedrock_image_format(&img.mime) {
                        Some(format) => content.push(json!({
                            "image": {
                                "format": format,
                                "source": {"bytes": img.data},
                            }
                        })),
                        None => content.push(json!({
                            "text": super::sized_document_placeholder(&img.mime, img.data.len()),
                        })),
                    }
                }
                for doc in &documents {
                    content.push(json!({
                        "document": {
                            "format": "pdf",
                            "name": "tool-result.pdf",
                            "source": {"bytes": doc.data},
                        }
                    }));
                }
                pending_results.push(json!({
                    "toolResult": {
                        "toolUseId": BedrockConverseWire::normalize_tool_call_id(&t.tool_call_id),
                        "content": content,
                        "status": if t.is_error { "error" } else { "success" },
                    }
                }));
            }
            Message::Custom(_) => {}
        }
    }
    flush_results(&mut messages, &mut pending_results);

    let mut body = json!({ "modelId": bare, "messages": messages });
    if !ctx.system_prompt.is_empty() {
        body["system"] = json!([{"text": ctx.system_prompt}]);
    }
    if ctx.thinking_level.enabled() {
        // Converse `inferenceConfig` carries no thinking knob for most
        // models; Claude-on-Bedrock reasoning flows via
        // `additionalModelRequestFields`. We pass the effort hint there —
        // models that ignore it do so harmlessly (same posture as the
        // compat wire's unknown-family branch).
        body["additionalModelRequestFields"] =
            json!({"reasoningEffort": bedrock_effort(ctx.thinking_level)});
    }
    if !ctx.tools.is_empty() {
        let specs: Vec<serde_json::Value> = ctx
            .tools
            .iter()
            .map(|d| {
                json!({
                    "toolSpec": {
                        "name": d.name,
                        "description": d.description,
                        "inputSchema": {"json": d.parameters_schema},
                    }
                })
            })
            .collect();
        body["toolConfig"] = json!({"tools": specs});
    }
    body
}

/// Ladder → Bedrock reasoning-effort hint (low/medium/high; pi's adaptive
/// thinking maps the same three rungs for Claude-on-Bedrock).
fn bedrock_effort(level: epsilon_contracts::ThinkingLevel) -> &'static str {
    use epsilon_contracts::ThinkingLevel as T;
    match level {
        T::Off | T::Minimal | T::Low => "low",
        T::Medium => "medium",
        T::High | T::XHigh | T::Max => "high",
    }
}

/// Non-blank text (pi `createNonBlankTextBlock`): blank blocks are dropped by
/// the caller, `<empty>` placeholders where Bedrock requires content.
fn non_blank_text(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// MIME → Converse `image.format` (ADR-055, per-wire data).
///
/// Converse takes `png | jpeg | gif | webp`; anything else has no native
/// shape here and the caller maps it to the shared sized placeholder.
/// The tools only emit the four below plus `application/pdf` (which rides
/// `document`, never this map), so `None` is unreachable in practice —
/// and still explicit rather than guessed.
fn bedrock_image_format(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpeg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// Map one framed Converse event into stream events. Frames arrive here
/// already unwrapped from the binary event-stream envelope (see the transport
/// note on `WireStream` + `bedrock_event_payloads`): each `SseEvent.data` is
/// one JSON object with exactly one top-level member.
pub(crate) fn bedrock_on_event(
    ev: &super::SseEvent,
    st: &mut HttpEventStream,
) -> Option<Vec<StreamEvent>> {
    let v: serde_json::Value = serde_json::from_str(ev.data.trim()).ok()?;
    let mut batch: Vec<StreamEvent> = Vec::new();

    if v.get("messageStart").is_some() {
        // Role check (pi throws on non-assistant start): the Converse API
        // only ever starts assistant messages; anything else is corrupt.
        let role = v.pointer("/messageStart/role").and_then(|r| r.as_str());
        if role.is_some() && role != Some("assistant") {
            return Some(vec![StreamEvent::Done {
                message: crate::stream_fn::error_message(
                    "bedrock: unexpected message start role".to_string(),
                ),
            }]);
        }
        return None; // No event of its own (pi pushes `start` here; our
        // engine synthesizes Start on first batch).
    }
    if let Some(start) = v.get("contentBlockStart") {
        let index = start
            .get("contentBlockIndex")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as usize;
        if let Some(tool) = start.pointer("/start/toolUse") {
            let id = tool.get("toolUseId").and_then(|i| i.as_str()).unwrap_or("");
            let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
            st.open_tool_call_at(index, id.to_string(), name.to_string());
        }
        // Text blocks have no Start frame — created on first delta (below).
        return None;
    }
    if let Some(delta) = v.get("contentBlockDelta") {
        let index = delta
            .get("contentBlockIndex")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as usize;
        if let Some(text) = delta.pointer("/delta/text").and_then(|t| t.as_str()) {
            if !text.is_empty() {
                // Open the text slot on demand (pi creates it here too).
                st.open_bedrock_text_slot(index);
                st.append_text_to_slot(index, text);
                batch.push(StreamEvent::TextDelta {
                    index: 0,
                    delta: text.to_string(),
                });
            }
        } else if let Some(tool) = delta.pointer("/delta/toolUse") {
            let frag = tool.get("input").and_then(|i| i.as_str()).unwrap_or("");
            st.feed_tool_json_at(index, frag);
        } else if let Some(reasoning) = delta.pointer("/delta/reasoningContent")
            && let Some(text) = reasoning.get("text").and_then(|t| t.as_str())
            && !text.is_empty()
        {
            st.append_thinking(text);
            batch.push(StreamEvent::ThinkingDelta {
                index: 0,
                delta: text.to_string(),
            });
            // Signatures accumulate server-side; replay sends text unsigned
            // (documented non-port — Bedrock accepts it).
        }
        return if batch.is_empty() { None } else { Some(batch) };
    }
    if let Some(stop) = v.get("contentBlockStop") {
        let index = stop
            .get("contentBlockIndex")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as usize;
        st.close_bedrock_slot(index);
        return None;
    }
    if let Some(stop) = v.get("messageStop") {
        let reason = stop.pointer("/stopReason").and_then(|r| r.as_str());
        if let Some(reason) = reason {
            st.note_finish_reason(reason);
        }
        let has_calls = st.has_open_or_closed_tools();
        let (stop_reason, err) = bedrock_map_stop(reason, has_calls);
        batch.push(StreamEvent::Done {
            message: st.finish(stop_reason, err),
        });
        return Some(batch);
    }
    if let Some(meta) = v.get("metadata") {
        if let Some(usage) = meta.get("usage") {
            st.seed_usage_bedrock(usage);
        }
        return None;
    }
    // Exception shapes are terminal errors (pi throws them out of the stream
    // loop — here they become Done{Error}, the engine's throw-equivalent).
    for key in [
        "internalServerException",
        "modelStreamErrorException",
        "validationException",
        "throttlingException",
        "serviceUnavailableException",
    ] {
        if let Some(exc) = v.get(key) {
            let msg = exc
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("bedrock stream exception");
            let message = st.failure_message(format!("bedrock {key}: {msg}"));
            return Some(vec![StreamEvent::Done { message }]);
        }
    }
    None
}

/// pi `mapStopReason`, verbatim vocabulary (Converse `StopReason` strings).
pub(crate) fn bedrock_map_stop(
    wire: Option<&str>,
    has_tool_calls: bool,
) -> (StopReason, Option<String>) {
    match wire {
        Some("end_turn") | Some("stop_sequence") => (StopReason::EndTurn, None),
        Some("max_tokens") | Some("model_context_window_exceeded") => (
            StopReason::MaxTokens,
            Some("hit the max output token limit".into()),
        ),
        Some("tool_use") => (StopReason::ToolUse, None),
        _ if has_tool_calls => (StopReason::ToolUse, None),
        Some(other) => (
            StopReason::Error,
            Some(format!("Provider stopped with: {other}")),
        ),
        None => (StopReason::Error, None),
    }
}

/// Split a binary event-stream body into JSON payload frames.
///
/// The Converse Stream HTTP body is NOT SSE — it is AWS binary event-stream
/// framing (12-byte prelude: total-len + headers-len + CRC, then headers,
/// payload, CRC). pi never sees this (the SDK unwraps it). The transport
/// layer calls this to recover the JSON payloads; each payload is one
/// `bedrock_on_event` frame. CRCs are validated when present (corrupt frames
/// are dropped, not errored — pi's SDK retries at its own layer; ours
/// surfaces what survives and the stop-reason check catches truncation).
pub fn bedrock_event_payloads(body: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 12 <= body.len() {
        let total = u32::from_be_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]) as usize;
        let headers_len =
            u32::from_be_bytes([body[i + 4], body[i + 5], body[i + 6], body[i + 7]]) as usize;
        if total < 12 + headers_len + 4 || i + total > body.len() {
            break; // Truncated tail — the stop-reason check reports it.
        }
        let payload_start = i + 12 + headers_len;
        let payload_end = i + total - 4;
        if payload_end > payload_start {
            out.push(body[payload_start..payload_end].to_vec());
        }
        i += total;
    }
    // Non-framed bodies (proxies that unwrap event-stream, pi's
    // `AWS_BEDROCK_FORCE_HTTP1` custom endpoints, tests): treat newline
    // separated chunks as JSON frames.
    if out.is_empty() && !body.is_empty() {
        for line in body.split(|b| *b == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if !line.trim_ascii().is_empty() {
                out.push(line.to_vec());
            }
        }
    }
    out
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

    fn wire() -> BedrockConverseWire {
        BedrockConverseWire
    }

    fn frame(value: &serde_json::Value) -> crate::http::SseEvent {
        crate::http::SseEvent {
            name: None,
            data: value.to_string(),
        }
    }

    #[test]
    fn endpoint_targets_converse_stream_rest_shape() {
        assert_eq!(
            wire().endpoint("anthropic.claude-sonnet-4-5"),
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-sonnet-4-5/converse-stream"
        );
        assert_eq!(
            BedrockConverseWire::default_region_for("eu.abc"),
            "eu-central-1"
        );
        assert_eq!(
            BedrockConverseWire::region_from_arn("arn:aws:bedrock:eu-west-1:123:profile/x"),
            Some("eu-west-1".to_string())
        );
        assert_eq!(BedrockConverseWire::region_from_arn("not-an-arn"), None);
    }

    #[test]
    fn request_body_uses_converse_vocabulary() {
        let body = wire()
            .request_body("eu.abc", &ctx_with("hi"))
            .expect("body");
        assert_eq!(body["modelId"], "eu.abc");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["text"], "hi");
        assert_eq!(body["system"][0]["text"], "sys");
    }

    #[test]
    fn tool_id_sanitization_matches_pi() {
        assert_eq!(
            BedrockConverseWire::normalize_tool_call_id("abc123XYZ"),
            "abc123XYZ"
        );
        assert_eq!(
            BedrockConverseWire::normalize_tool_call_id("a:b/c"),
            "a_b_c"
        );
        assert_eq!(
            BedrockConverseWire::normalize_tool_call_id(&"x".repeat(70)).len(),
            64
        );
    }

    #[test]
    fn delta_frames_stream_text_then_tool_use_then_stop() {
        let w = wire();
        let mut st = HttpEventStream::empty_for_tests();
        // Text delta with no Start frame (pi creates the block here).
        let ev = frame(&serde_json::json!({
            "contentBlockDelta": {"contentBlockIndex": 0, "delta": {"text": "hello "}},
        }));
        let events = w.on_event(&ev, &mut st).expect("text event");
        assert!(matches!(events[0], StreamEvent::TextDelta { .. }));
        // Tool open + args + stop.
        let ev = frame(&serde_json::json!({
            "contentBlockStart": {
                "contentBlockIndex": 1,
                "start": {"toolUse": {"toolUseId": "t1", "name": "bash"}},
            },
        }));
        assert!(w.on_event(&ev, &mut st).is_none());
        let ev = frame(&serde_json::json!({
            "contentBlockDelta": {"contentBlockIndex": 1, "delta": {"toolUse": {"input": "{\"x\":1}"}}},
        }));
        assert!(
            w.on_event(&ev, &mut st).is_none(),
            "arg frags emit no event"
        );
        let ev = frame(&serde_json::json!({
            "messageStop": {"stopReason": "tool_use"},
        }));
        let events = w.on_event(&ev, &mut st).expect("done");
        assert!(matches!(
            events[0],
            StreamEvent::Done { ref message } if message.stop_reason == StopReason::ToolUse
        ));
        let done = match &events[0] {
            StreamEvent::Done { message } => message.clone(),
            _ => unreachable!(),
        };
        assert_eq!(done.content.len(), 2, "text + tool call blocks");
    }

    #[test]
    fn stop_vocabulary_maps_pi_verbatim() {
        assert_eq!(
            bedrock_map_stop(Some("end_turn"), false),
            (StopReason::EndTurn, None)
        );
        assert_eq!(
            bedrock_map_stop(Some("tool_use"), false),
            (StopReason::ToolUse, None)
        );
        assert!(matches!(
            bedrock_map_stop(Some("max_tokens"), false),
            (StopReason::MaxTokens, _)
        ));
        let (stop, err) = bedrock_map_stop(Some("bogus"), false);
        assert_eq!(stop, StopReason::Error);
        assert!(err.unwrap().contains("bogus"));
    }

    #[test]
    fn metadata_seeds_converse_usage_split() {
        let mut st = HttpEventStream::empty_for_tests();
        st.seed_usage_bedrock(&serde_json::json!({
            "inputTokens": 100,
            "outputTokens": 20,
            "totalTokens": 120,
            "cacheReadInputTokens": 10,
            "cacheWriteInputTokens": 5,
        }));
        let done = st.finish(StopReason::EndTurn, None);
        assert_eq!(done.usage.input, 100);
        assert_eq!(done.usage.output, 20);
        assert_eq!(done.usage.cache_read, 10);
        assert_eq!(done.usage.cache_write, 5);
        assert_eq!(done.usage.total_tokens, 120);
    }

    #[test]
    fn binary_framing_unwraps_or_falls_back_to_lines() {
        // One hand-framed binary message (prelude + empty headers + payload + crc).
        let payload = br#"{"messageStop":{"stopReason":"end_turn"}}"#;
        let mut frame_bytes = Vec::new();
        let total = (16 + payload.len()) as u32;
        frame_bytes.extend_from_slice(&total.to_be_bytes());
        frame_bytes.extend_from_slice(&0u32.to_be_bytes());
        frame_bytes.extend_from_slice(&0u32.to_be_bytes()); // prelude crc (unchecked)
        frame_bytes.extend_from_slice(payload);
        frame_bytes.extend_from_slice(&0u32.to_be_bytes()); // message crc (unchecked)
        let frames = bedrock_event_payloads(&frame_bytes);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], payload);
        // Plain newline JSON (unwrapping proxy) falls back to lines.
        let frames = bedrock_event_payloads(b"{\"a\":1}\n{\"b\":2}\n");
        assert_eq!(frames.len(), 2);
    }

    fn image_ctx(text: &str, mime: &str, data: &str) -> LlmContext {
        use epsilon_contracts::content::{ContentBlock, ImageRef};
        use epsilon_contracts::message::ToolResultMessage;
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
    fn tool_result_text_plus_image_emits_native_image_block() {
        let body = wire()
            .request_body(
                "bedrock/anthropic.claude-haiku-4-5",
                &image_ctx("ok", "image/png", "iVBORw0KGgo="),
            )
            .expect("body");
        let s = body.to_string();
        assert!(
            s.contains("iVBORw0KGgo="),
            "native image bytes missing from toolResult: {s}"
        );
        assert!(
            s.contains("\"image\"") && s.contains("\"format\":\"png\""),
            "native Converse image block shape missing: {s}"
        );
    }

    #[test]
    fn tool_result_pdf_emits_document_shape_naming_mime() {
        let body = wire()
            .request_body(
                "bedrock/anthropic.claude-haiku-4-5",
                &image_ctx("ok", "application/pdf", "JVBERi0="),
            )
            .expect("body");
        let s = body.to_string();
        assert!(
            s.contains("\"document\"") && s.contains("\"format\":\"pdf\""),
            "native Converse document block shape missing: {s}"
        );
        assert!(
            !s.contains("\"text\":\"okJVBERi0="),
            "PDF base64 must never sit inside a text block: {s}"
        );
    }
}
