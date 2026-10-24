//! Mistral Conversations over direct HTTP (pi port).
//!
//! Ported from pi-0.84.2 `packages/ai/src/api/mistral-conversations.ts`:
//! `POST {base}/v1/chat/completions` with `Bearer` auth, `stream: true` plus
//! `stream_options: {"include_usage": true}` so the final chunk carries usage,
//! tool calls as `tool_calls` arrays with indexed JSON-argument fragments,
//! `[DONE]` termination, and an OpenAI-shaped `finish_reason` vocabulary — so
//! far identical to [`super::openai_compat`], and deliberately sharing its
//! request framing. Five genuine Mistral deltas live here:
//!
//! 1. **snake_case wire payload** (pi `toMistralWirePayload`): Mistral accepts
//!    `top_p`, `max_tokens`, `random_seed`, `response_format`, `tool_choice`,
//!    `presence_penalty`, `frequency_penalty`, `parallel_tool_calls`,
//!    `reasoning_effort`, `prompt_mode`, `prompt_cache_key`, `safe_prompt`.
//!    We emit snake_case directly instead of building camelCase and remapping.
//! 2. **Codestral tool-call id normalization** (pi
//!    `createMistralToolCallIdNormalizer`): ids must be exactly 9
//!    `[a-zA-Z0-9]` chars; anything else is replaced by a stable per-stream
//!    `shortHash` (pi `utils/hash.ts`, reimplemented here — `Math.imul`
//!    semantics preserved via `wrapping_mul`) with collision-salted retries.
//!    `"null"` ids fall back to `derive(short id, 0)` like pi.
//! 3. **Reasoning routing** (pi `streamSimple` + `usesPromptModeReasoning`):
//!    Magistral-style models take `prompt_mode: "reasoning"`; the small/medium
//!    reasoning rows take `reasoning_effort` (mapped through the catalog's
//!    thinking-level map, defaulting to `"high"`); others get nothing.
//! 4. **Cached-token extraction** (pi `getMistralCachedPromptTokens`):
//!    `prompt_tokens_details.{cached_tokens,cachedTokens}`, the camelCase
//!    twins, or `num_{cached_}tokens` — capped at `prompt_tokens`, never below
//!    zero. The shared compat extractor doesn't know these keys.
//! 5. **Stop vocabulary, raw**: pi's `mapChatStopReason` maps `null` to `stop`
//!    and anything unknown (including `"error"`) to an error naming the reason.
//!    The compat mapper treats unknown as a clean turn end — wrong here.
//!
//! What we deliberately do NOT port: `x-affinity` session pinning
//! (`shouldUsePromptCaching` + `sessionId` plumbing — Epsilon has no session
//! concept at the wire layer; the `LlmContext` carries no session id),
//! `onPayload`/`onResponse` interception hooks (no caller uses them here),
//! per-model `thinkingLevelMap` tables (the wire maps our ladder directly;
//! see `reasoning_effort_for`), and the `image_url` message transform —
//! Epsilon's context carries text only (M43 images ride tool results).

use super::{HttpEventStream, ProviderWire};
use epsilon_contracts::message::StopReason;
use epsilon_contracts::stream::StreamEvent;
use serde_json::json;

/// Mistral Conversations wire descriptor (fixture/test visibility).
pub struct MistralConversationsWire {
    /// Base URL ending without a trailing slash (pi provider's `baseUrl`).
    pub base_url: String,
    /// Auth env var (`Some("MISTRAL_API_KEY")` for the catalog rows).
    pub env_key: Option<String>,
}

/// Tool-call ids must be exactly this long (pi `MISTRAL_TOOL_CALL_ID_LENGTH`).
const TOOL_CALL_ID_LENGTH: usize = 9;

impl ProviderWire for MistralConversationsWire {
    fn endpoint(&self, _model: &str) -> String {
        format!("{}/v1/chat/completions", self.base_url)
    }

    fn env_key(&self) -> &str {
        self.env_key.as_deref().unwrap_or("")
    }

    fn headers(&self, key: &str) -> Vec<(String, String)> {
        vec![("Authorization".to_string(), format!("Bearer {key}"))]
    }

    fn request_body(
        &self,
        model: &str,
        ctx: &crate::LlmContext,
    ) -> Result<serde_json::Value, String> {
        // Shared OpenAI framing (system prompt, interleaved tool results,
        // function tools, stream + usage flags, thinking-chunk message parts):
        // identical shape, so reuse the compat builder then apply the deltas.
        let helper = super::openai_compat::OpenAiCompatWire {
            base_url: self.base_url.clone(),
            env_key: self.env_key.clone(),
        };
        let mut body = helper.request_body(model, ctx)?;

        // Delta 3: reasoning routing. Wave D: the catalog row's
        // `reasoning_shape` decides (M3 binding property) — pi keys off its
        // row too (`model.reasoning`, `thinkingLevelMap`). Unlisted rows send
        // nothing, like the old family branches' fallthrough.
        body.as_object_mut()
            .expect("compat builder returns an object")
            .remove("reasoning_effort");
        if ctx.thinking_level.enabled() {
            apply_mistral_reasoning(
                &mut body,
                crate::catalog::reasoning_shape_for(model)
                    .unwrap_or(crate::catalog::ReasoningShape::None),
                ctx.thinking_level,
            );
        }

        // Delta 1 is structural (we never emit camelCase), delta 5 needs no
        // request change. Tool schema `strict` flag: pi sends
        // `strict: <resolveJsonSchemaStrictSampling(tool, true) ?? false>` —
        // our descriptors carry no sampling metadata, so `false` (pi's
        // fallback for tools without it).
        if let Some(tools) = body.get_mut("tools").and_then(|t| t.as_array_mut()) {
            for t in tools.iter_mut() {
                if let Some(f) = t.get_mut("function") {
                    f["strict"] = json!(false);
                }
            }
        }
        if super::is_first_party_host(&self.base_url, "api.mistral.ai")
            && let Some(key) = ctx.prompt_cache_key(model)
        {
            body["prompt_cache_key"] = json!(key);
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
        if let Some(err_msg) = v.pointer("/error/message").and_then(|x| x.as_str()) {
            let message = st.failure_message(format!("provider stream error: {err_msg}"));
            return Some(vec![StreamEvent::Done { message }]);
        }
        let choices = v.get("choices").and_then(|c| c.as_array());
        // Usage may arrive on its own frame, or on the choice. Read it before the
        // `choices` guard; the old `?` silently dropped the billed count.
        if let Some(usage) = v
            .get("usage")
            .or_else(|| choices.and_then(|c| c.iter().find_map(|choice| choice.get("usage"))))
            .filter(|u| !u.is_null())
        {
            st.seed_usage_mistral(usage);
        }
        let choices = choices?;
        for choice in choices.iter() {
            if let Some(delta) = choice.get("delta") {
                // Delta: content may be a string OR an array of typed chunks
                // (`{type:"text",text}` / `{type:"thinking",thinking:[{text}]}`).
                if let Some(content) = delta.get("content") {
                    for (text, thinking) in mistral_content_parts(content) {
                        if !text.is_empty() {
                            st.append_text(&text);
                            return Some(vec![StreamEvent::TextDelta {
                                index: 0,
                                delta: text,
                            }]);
                        }
                        if !thinking.is_empty() {
                            st.append_thinking(&thinking);
                            return Some(vec![StreamEvent::ThinkingDelta {
                                index: 0,
                                delta: thinking,
                            }]);
                        }
                    }
                }
                if let Some(calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                    for call in calls {
                        let idx = call.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                        let raw_id = call.get("id").and_then(|x| x.as_str()).unwrap_or("");
                        let name = call
                            .pointer("/function/name")
                            .and_then(|x| x.as_str())
                            .unwrap_or("");
                        // Delta 2: normalize on open AND on id-less continuation
                        // (pi normalizes the outgoing ids; the wire only ever
                        // carries the normalized form).
                        if !name.is_empty() {
                            let id = normalize_tool_call_id(raw_id, idx, st);
                            st.open_tool_call_at(idx, id, name.to_string());
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
        None
    }

    fn map_stop(&self, wire: Option<&str>, has_tool_calls: bool) -> (StopReason, Option<String>) {
        // pi `mapChatStopReason`, verbatim: null (no reason recorded) means a
        // clean stop; unknown reasons are errors that name the reason.
        match wire {
            None if !has_tool_calls => (StopReason::EndTurn, None),
            None => (StopReason::ToolUse, None),
            Some("stop") => (StopReason::EndTurn, None),
            Some("length") | Some("model_length") => (
                StopReason::MaxTokens,
                Some("hit the max output token limit".into()),
            ),
            Some("tool_calls") => (StopReason::ToolUse, None),
            Some(other) => (
                StopReason::Error,
                Some(format!("Provider stopped with: {other}")),
            ),
        }
    }

    fn finalize_usage_from_tail(&self) -> bool {
        false
    }
}

/// Shape → Mistral body bytes. Exhaustive on purpose (no wildcard arm),
/// mirroring the compat wire's mapper: a new `ReasoningShape` variant fails
/// to compile until both wires decide what it means. Compat-only shapes are
/// unreachable via `wire_for` (those rows ride the compat wire) and stay
/// silent here — the old branches sent nothing for non-Mistral families too.
fn apply_mistral_reasoning(
    body: &mut serde_json::Value,
    shape: crate::catalog::ReasoningShape,
    level: epsilon_contracts::ThinkingLevel,
) {
    use crate::catalog::ReasoningShape as S;
    if !level.enabled() {
        return;
    }
    match shape {
        S::MagistralPromptMode => {
            body["prompt_mode"] = json!("reasoning");
        }
        S::MistralEffort => {
            body["reasoning_effort"] = json!(effort_str(level));
        }
        S::None
        | S::StandardEffort
        | S::OpenRouterReasoning
        | S::DeepSeekThinking
        | S::GlmThinking => {}
    }
}

/// Ladder -> Mistral effort string. The small/medium rows take pi's default
/// (`thinkingLevelMap[level] ?? "high"`); our ladder has no per-model table,
/// so map Low->"low", Medium->"medium", High and above->"high" (pi's test pins
/// `medium` -> `"high"` for mistral-small-2603).
fn effort_str(level: epsilon_contracts::ThinkingLevel) -> &'static str {
    use epsilon_contracts::ThinkingLevel as T;
    match level {
        T::Off | T::Minimal => "low",
        T::Low | T::Medium => "medium",
        T::High | T::XHigh | T::Max => "high",
    }
}

/// Split pi's `delta.content` union into `(text, thinking)` fragments:
/// a bare string is one text fragment; an array holds `{type:"text",text}`
/// and `{type:"thinking",thinking:[{text}]}` chunks (pi `consumeChatStream`).
fn mistral_content_parts(content: &serde_json::Value) -> Vec<(String, String)> {
    if let Some(s) = content.as_str() {
        return vec![(s.to_string(), String::new())];
    }
    let mut out = Vec::new();
    for item in content.as_array().cloned().unwrap_or_default() {
        if let Some(s) = item.as_str() {
            out.push((s.to_string(), String::new()));
        } else if item.get("type").and_then(|t| t.as_str()) == Some("thinking") {
            let text = item
                .get("thinking")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<String>();
            out.push((String::new(), text));
        } else {
            out.push((
                item.get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                String::new(),
            ));
        }
    }
    out
}

/// Normalize one tool-call id to Mistral's 9-char alphanumeric form, tracking
/// used ids on the stream state for collision-salted retries (pi
/// `createMistralToolCallIdNormalizer` closure, re-expressed as a `st`
/// side-table since the trait gives us no per-stream struct of our own).
///
/// Callers pass `idx` (the chunk's tool index) as the fallback seed when the
/// wire id is missing or `"null"` (pi `deriveMistralToolCallId("toolcall:{index}", 0)`).
fn normalize_tool_call_id(raw_id: &str, idx: usize, st: &mut HttpEventStream) -> String {
    let normalized: String = raw_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    // Fast path: exact-length ids pass through (pi's `attempt === 0`
    // return) — UNLESS the candidate is already owned by another seed (pi's
    // `owner === id` check: id re-seen for the SAME input passes, a foreign
    // collision salts to the hash path). Memo key is the id itself.
    if normalized.len() == TOOL_CALL_ID_LENGTH {
        if let Some(known) = st.mistral_id_for(&normalized) {
            if known == normalized {
                return known;
            }
        } else {
            st.remember_mistral_id(&normalized, &normalized);
            return normalized;
        }
    }
    // Hash path: check the seed memo first for stability across chunks.
    // (`"null"` and `""` both normalize to the index seed — pi's
    // `deriveMistralToolCallId("toolcall:{index}", 0)` fallback covers the
    // missing AND the unusable-id cases: `"null"` strips to `"null"` (4
    // chars, fails the length check) and must NOT seed from itself, or two
    // spellings of "no usable id" would diverge.)
    let seed_key = if normalized.is_empty() || normalized == "null" {
        format!("toolcall:{idx}")
    } else {
        normalized.clone()
    };
    if let Some(known) = st.mistral_id_for(&seed_key) {
        return known;
    }
    let seed_base = seed_key;
    let mut attempt = 0;
    loop {
        let seed = if attempt == 0 {
            seed_base.clone()
        } else {
            format!("{seed_base}:{attempt}")
        };
        let candidate: String = short_hash(&seed)
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(TOOL_CALL_ID_LENGTH)
            .collect();
        if !st.mistral_id_taken(&candidate, &seed_base) {
            st.remember_mistral_id(&seed_base, &candidate);
            return candidate;
        }
        attempt += 1;
    }
}

/// pi `utils/hash.ts` `shortHash`: two `Math.imul` accumulators mixed down and
/// rendered as unsigned base-36, concatenated. `u32::wrapping_mul` is exactly
/// `Math.imul` (32-bit wrap); `>>>` is a logical shift (`>>` on `u32`).
fn short_hash(s: &str) -> String {
    let mut h1: u32 = 0xdead_beef;
    let mut h2: u32 = 0x41c6_ce57;
    for ch in s.encode_utf16() {
        h1 = (h1 ^ u32::from(ch)).wrapping_mul(2_654_435_761);
        h2 = (h2 ^ u32::from(ch)).wrapping_mul(1_597_334_677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h2 ^ (h2 >> 13)).wrapping_mul(3_266_489_909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h1 ^ (h1 >> 13)).wrapping_mul(3_266_489_909);
    format!("{}{}", to_base36(h2), to_base36(h1))
}

/// `Number.prototype.toString(36)` for a u32: lowercase alphanumerics.
fn to_base36(mut n: u32) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("base36 digits are ASCII")
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

    fn wire() -> MistralConversationsWire {
        MistralConversationsWire {
            base_url: "https://api.mistral.ai".into(),
            env_key: Some("MISTRAL_API_KEY".into()),
        }
    }

    fn sse(data: &serde_json::Value) -> crate::http::SseEvent {
        crate::http::SseEvent {
            name: None,
            data: data.to_string(),
        }
    }

    #[test]
    fn request_body_targets_mistral_endpoint_without_reasoning_by_default() {
        assert_eq!(
            wire().endpoint("mistral-large-latest"),
            "https://api.mistral.ai/v1/chat/completions"
        );
        let body = wire().request_body("x", &ctx_with("hi")).expect("body");
        assert_eq!(body["model"], "x");
        assert!(
            body["prompt_cache_key"].is_string(),
            "Mistral gets a stable cache-affinity key: {body}"
        );
        assert!(body.get("prompt_mode").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    /// A retired magistral row sends neither knob: `magistral-medium-latest`
    /// left the bundle as deprecated upstream, so the family routing falls
    /// back to silence for it. Name kept for continuity; the retired path is
    /// what this now pins (see the integration twin in reasoning_routing.rs).
    #[test]
    fn reasoning_routes_by_family_magistral_gets_prompt_mode() {
        let mut ctx = ctx_with("hi");
        ctx.thinking_level = epsilon_contracts::ThinkingLevel::Medium;
        let body = wire()
            .request_body("magistral-medium-latest", &ctx)
            .expect("body");
        assert!(body.get("prompt_mode").is_none());
        assert!(body.get("reasoning_effort").is_none());

        let body = wire()
            .request_body("mistral-small-2603", &ctx)
            .expect("body");
        assert_eq!(body["reasoning_effort"], "medium");
        assert!(body.get("prompt_mode").is_none());

        // Non-reasoning rows get neither knob.
        let body = wire()
            .request_body("mistral-large-latest", &ctx)
            .expect("body");
        assert!(body.get("prompt_mode").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn tool_call_id_normalizer_keeps_valid_rewrites_rest_stably() {
        let mut st = HttpEventStream::empty_for_tests();
        // Already-valid ids pass through untouched.
        assert_eq!(normalize_tool_call_id("abc123XYZ", 0, &mut st), "abc123XYZ");
        // Long ids hash to a stable 9-char alphanumeric form.
        let a = normalize_tool_call_id("call_very_long_id_1", 0, &mut st);
        let b = normalize_tool_call_id("call_very_long_id_1", 0, &mut st);
        assert_eq!(a, b);
        assert_eq!(a.len(), TOOL_CALL_ID_LENGTH);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        // Missing ids (`""`) derive from the tool index; pi's `"null"`-string
        // case feeds the literal `"null"` through the same path (it is NOT
        // empty after normalization) — both spellings must hash identically,
        // so normalize `"null"` to the index seed too.
        let c = normalize_tool_call_id("null", 2, &mut st);
        let d = normalize_tool_call_id("", 2, &mut st);
        assert_eq!(c, d);
        assert_ne!(c, "abc123XYZ");
    }

    #[test]
    fn stop_vocabulary_maps_pi_verbatim_including_unknown_as_error() {
        assert_eq!(
            wire().map_stop(Some("stop"), false),
            (StopReason::EndTurn, None)
        );
        assert_eq!(
            wire().map_stop(Some("tool_calls"), false),
            (StopReason::ToolUse, None)
        );
        assert!(matches!(
            wire().map_stop(Some("length"), false),
            (StopReason::MaxTokens, _)
        ));
        let (stop, err) = wire().map_stop(Some("bogus_reason"), false);
        assert_eq!(stop, StopReason::Error);
        assert!(err.unwrap().contains("bogus_reason"));
    }

    #[test]
    fn cached_tokens_read_from_mistral_detail_keys() {
        let mut st = HttpEventStream::empty_for_tests();
        st.seed_usage_mistral(&serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 10,
            "total_tokens": 110,
            "prompt_tokens_details": { "cached_tokens": 30 },
        }));
        let done = st.finish(StopReason::EndTurn, None);
        assert_eq!(done.usage.cache_read, 30);
        assert_eq!(done.usage.input, 70);
    }

    #[test]
    fn array_content_chunks_split_text_and_thinking() {
        let w = wire();
        let mut st = HttpEventStream::empty_for_tests();
        let ev = sse(&serde_json::json!({
            "choices": [{
                "delta": { "content": [
                    { "type": "text", "text": "hello " },
                    { "type": "thinking", "thinking": [{ "type": "text", "text": "hmm" }] },
                ] },
            }],
        }));
        let events = w.on_event(&ev, &mut st).expect("text event");
        assert!(matches!(events[0], StreamEvent::TextDelta { .. }));
    }

    /// The mapper never sees the id: a magistral-shaped request carrying the
    /// effort field gets effort (not prompt_mode), and compat-only shapes
    /// stay silent on this wire.
    #[test]
    fn mistral_routing_follows_the_field_not_the_id() {
        use crate::catalog::ReasoningShape as S;
        use epsilon_contracts::ThinkingLevel as T;
        let mut body = serde_json::json!({});
        apply_mistral_reasoning(&mut body, S::MistralEffort, T::Medium);
        assert_eq!(body["reasoning_effort"], "medium");
        assert!(body.get("prompt_mode").is_none(), "{body}");

        let mut body = serde_json::json!({});
        apply_mistral_reasoning(&mut body, S::MagistralPromptMode, T::Medium);
        assert_eq!(body["prompt_mode"], "reasoning");

        for shape in [
            S::None,
            S::StandardEffort,
            S::OpenRouterReasoning,
            S::DeepSeekThinking,
            S::GlmThinking,
        ] {
            let mut body = serde_json::json!({});
            apply_mistral_reasoning(&mut body, shape, T::Medium);
            assert!(
                body.as_object().is_some_and(|o| o.is_empty()),
                "{shape:?}: {body}"
            );
        }
    }

    /// Lookup miss stays silent end to end: the compat builder's default
    /// knob is stripped and the Mistral fallback sends nothing.
    #[test]
    fn mistral_unknown_ids_stay_silent() {
        let mut ctx = ctx_with("hi");
        ctx.thinking_level = epsilon_contracts::ThinkingLevel::Medium;
        let body = wire()
            .request_body("totally-unknown-model", &ctx)
            .expect("body");
        assert!(body.get("prompt_mode").is_none(), "{body}");
        assert!(body.get("reasoning_effort").is_none(), "{body}");
    }

    fn image_ctx(text: &str, mime: &str, data: &str) -> LlmContext {
        use epsilon_contracts::content::{ContentBlock, ImageRef};
        use epsilon_contracts::message::{Message, ToolResultMessage};
        use epsilon_contracts::new_id;
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
    fn tool_result_text_plus_image_inherits_compat_follow_up() {
        let body = wire()
            .request_body(
                "mistral-large-latest",
                &image_ctx("ok", "image/png", "iVBORw0KGgo="),
            )
            .expect("body");
        let s = body.to_string();
        assert!(
            s.contains("iVBORw0KGgo=") && s.contains("image_url"),
            "inherited compat follow-up with bytes missing: {s}"
        );
    }

    #[test]
    fn blind_model_omits_bytes_with_placeholder() {
        let body = wire()
            .request_body(
                "totally-unknown-model",
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
