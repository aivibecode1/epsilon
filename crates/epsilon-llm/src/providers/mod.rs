//! Direct HTTP provider adapters (pi port).
//!
//! Each submodule implements one provider family's real REST API; this module owns
//! the shared streaming engine. Ported from pi-0.84.2 `packages/ai/src/api/` and
//! `packages/ai/src/providers/*.ts`: providers are plain data (base URL + auth env
//! var), the wire format is code we own, and every failure surfaces in-band as a
//! `Done { stop_reason: Error }` event per the StreamFn contract.

mod anthropic;
mod google;
mod openai_compat;

use std::collections::{BTreeMap, VecDeque};

use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::message::{AssistantMessage, StopReason, Usage};
use epsilon_contracts::stream::StreamEvent;

use crate::http::{SseDecoder, SseEvent, http_client};
use crate::stream_fn::{BoxEventStream, EventStream, LlmContext, ModelRef, StreamFn};

pub(crate) use anthropic::AnthropicWire;
pub(crate) use google::GoogleWire;
pub(crate) use openai_compat::OpenAiCompatWire;

// WireApi lives in `crate::catalog` (it is catalog data); re-exported here for the
// engine's internal use.
pub(crate) use crate::catalog::WireApi;

/// Per-provider wire implementation: request building + SSE interpretation.
///
/// Port shape follows pi's `Api` interface (`buildParams` / stream-event mapping /
/// stop-reason mapping) as a Rust object-safety boundary.
pub(crate) trait ProviderWire: Send + Sync {
    /// Full streaming URL for this model.
    fn endpoint(&self, model: &str) -> String;
    /// Auth env var consulted for the key.
    fn env_key(&self) -> &'static str;
    /// Auth/version headers derived from the raw key.
    fn headers(&self, key: &str) -> Vec<(String, String)>;
    /// Build the JSON request body for one call.
    fn request_body(&self, model: &str, ctx: &LlmContext) -> Result<serde_json::Value, String>;
    /// Map one decoded SSE frame into zero or more stream events (never empty).
    fn on_event(
        &self,
        ev: &crate::http::SseEvent,
        st: &mut HttpEventStream,
    ) -> Option<Vec<StreamEvent>>;
    /// Translate the provider's finish vocabulary to ours.
    fn map_stop(&self, wire: Option<&str>, has_tool_calls: bool) -> (StopReason, Option<String>);
    /// Whether the FINAL usage lives in the tail frame (Anthropic `message_delta`),
    /// so Done-time usage prefers it over the seed.
    fn finalize_usage_from_tail(&self) -> bool;
}

/// Resolve the wire implementation for a model id via its catalog entry. Unknown ids
/// error honestly — no silent default provider.
pub(crate) fn wire_for(model_id: &str) -> Result<Box<dyn ProviderWire>, String> {
    let info = crate::catalog::lookup_model(model_id)
        .ok_or_else(|| format!("unknown model '{model_id}' - pick one from /models"))?;
    Ok(match info.wire {
        WireApi::AnthropicMessages => Box::new(AnthropicWire),
        WireApi::OpenAiCompletions { base_url, env_key } => {
            Box::new(OpenAiCompatWire { base_url, env_key })
        }
        WireApi::GoogleGenerativeAi => Box::new(GoogleWire),
    })
}

/// Streaming accumulator shared by all wire implementations.
///
/// Owns the partial assistant message, tool-call argument assembly (indexed, since
/// OpenAI interleaves calls), usage capture, and final `Done` synthesis.
pub(crate) struct HttpEventStream {
    model: String,
    provider: &'static str,
    text: String,
    thinking: String,
    /// OpenAI-style indexed tool calls: index → (id, name, json buffer).
    tools_open: BTreeMap<usize, (String, String, String)>,
    /// Completed tool-call buffers awaiting finalization.
    tools_closed: Vec<(String, String, String)>,
    finish_reason: Option<String>,
    usage_seed: Option<Usage>,
    usage_tail: Option<Usage>,
    /// Wire-level preference: tail usage wins at Done time (Anthropic).
    prefer_tail_usage: bool,
}

impl HttpEventStream {
    fn new(model: &str, provider: &'static str, prefer_tail_usage: bool) -> Self {
        Self {
            model: model.to_string(),
            provider,
            text: String::new(),
            thinking: String::new(),
            tools_open: BTreeMap::new(),
            tools_closed: Vec::new(),
            finish_reason: None,
            usage_seed: None,
            usage_tail: None,
            prefer_tail_usage,
        }
    }

    /// Fresh accumulator state for a restarted stream body.
    pub(crate) fn reset_accumulator(&mut self) {
        self.text.clear();
        self.thinking.clear();
        self.tools_open.clear();
        self.tools_closed.clear();
        self.finish_reason = None;
    }

    /// Snapshot of the partial assistant message (Start events).
    pub(crate) fn partial(&self) -> AssistantMessage {
        self.build(Vec::new(), StopReason::EndTurn, None, Usage::default())
    }

    // ---- wire callbacks ---------------------------------------------------------------

    /// Anthropic `message_start`: authoritative initial usage.
    pub(crate) fn seed_usage(&mut self, usage: &serde_json::Value) {
        let get = |k: &str| usage.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
        let mut u = Usage::default();
        u.input = get("input_tokens");
        u.output = get("output_tokens");
        u.cache_read = get("cache_read_input_tokens");
        u.cache_write = get("cache_creation_input_tokens");
        u.total_tokens = u.input + u.output + u.cache_read + u.cache_write;
        if u.input + u.output > 0 {
            self.usage_seed = Some(u);
        }
    }

    /// Compat providers (OpenAI chunk.usage / Gemini usageMetadata): key-shape
    /// tolerant extraction into our Usage.
    pub(crate) fn seed_usage_compat(&mut self, usage: &serde_json::Value) {
        let get_u64 = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| usage.get(*k).and_then(|v| v.as_u64()))
                .unwrap_or(0)
        };
        let mut u = Usage::default();
        u.input = get_u64(&["prompt_tokens", "input_tokens", "promptTokenCount"]);
        u.output = get_u64(&["completion_tokens", "output_tokens", "candidatesTokenCount"]);
        u.cache_read = usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| get_u64(&["cache_read_input_tokens", "cachedContentTokenCount"]));
        u.cache_write = get_u64(&["cache_creation_input_tokens"]);
        u.total_tokens = get_u64(&["total_tokens", "totalTokenCount"]);
        if u.input + u.output > 0 {
            self.usage_tail = Some(u);
        }
    }

    /// Anthropic `message_delta.usage`: fields present overwrite, absent keep seed
    /// (pi preserves `input_tokens` from message_start when proxies omit it here).
    pub(crate) fn update_usage_tail(&mut self, usage: &serde_json::Value) {
        let mut u = self.usage_seed.clone().unwrap_or_default();
        let mut any = false;
        for (key, slot) in [
            ("input_tokens", &mut u.input),
            ("output_tokens", &mut u.output),
            ("cache_read_input_tokens", &mut u.cache_read),
            ("cache_creation_input_tokens", &mut u.cache_write),
        ] {
            if let Some(v) = usage.get(key).and_then(|x| x.as_u64()) {
                *slot = v;
                any = true;
            }
        }
        u.total_tokens = u.input + u.output + u.cache_read + u.cache_write;
        if any {
            self.usage_tail = Some(u);
        }
    }

    /// Open a tool call slot (single-call wires: Anthropic, Gemini).
    pub(crate) fn open_tool_call(&mut self, name: String) -> usize {
        let idx = self.next_tool_index();
        self.tools_open
            .insert(idx, (format!("call_{}", idx + 1), name, String::new()));
        idx
    }

    /// Open a tool call at an explicit index (OpenAI interleaved deltas).
    pub(crate) fn open_tool_call_at(&mut self, idx: usize, id: String, name: String) {
        self.tools_open
            .entry(idx)
            .or_insert((id, name, String::new()));
    }

    /// Append an argument fragment to the most recent open tool call.
    pub(crate) fn feed_tool_json(&mut self, frag: &str) {
        if let Some(last) = self.tools_open.keys().next_back().copied()
            && let Some(entry) = self.tools_open.get_mut(&last)
        {
            entry.2.push_str(frag);
        }
    }

    /// Append an argument fragment to an explicit tool call slot.
    pub(crate) fn feed_tool_json_at(&mut self, idx: usize, frag: &str) {
        match self.tools_open.get_mut(&idx) {
            Some(entry) => entry.2.push_str(frag),
            None => {
                // Deltas may arrive without a prior opening frame; synthesize the slot.
                self.tools_open.insert(
                    idx,
                    (
                        format!("call_{idx}"),
                        format!("tool_{idx}"),
                        frag.to_string(),
                    ),
                );
            }
        }
    }

    /// Close the most recent open call (Anthropic/Gemini content_block_stop).
    pub(crate) fn close_tool_call(&mut self) {
        if let Some(last) = self.tools_open.keys().next_back().copied()
            && let Some(entry) = self.tools_open.remove(&last)
        {
            self.tools_closed.push(entry);
        }
    }

    /// Any tool activity recorded (drives ToolUse stop mapping on [DONE]).
    pub(crate) fn has_open_or_closed_tools(&self) -> bool {
        !self.tools_open.is_empty() || !self.tools_closed.is_empty()
    }

    /// Remember a provider finish reason seen mid-stream (OpenAI finish_reason).
    pub(crate) fn note_finish_reason(&mut self, reason: &str) {
        self.finish_reason = Some(reason.to_string());
    }

    // ---- terminal assembly -------------------------------------------------------------

    /// Build the final assistant message for `Done`.
    pub(crate) fn finish(&mut self, stop: StopReason, err: Option<String>) -> AssistantMessage {
        // Flush any tool calls never explicitly closed ([DONE] without stop frames).
        let last_keys: Vec<usize> = self.tools_open.keys().copied().collect();
        for key in last_keys {
            if let Some(entry) = self.tools_open.remove(&key) {
                self.tools_closed.push(entry);
            }
        }

        let mut content: Vec<ContentBlock> = Vec::new();
        if !self.thinking.is_empty() {
            content.push(ContentBlock::Thinking {
                text: std::mem::take(&mut self.thinking),
            });
        }
        if !self.text.is_empty() {
            content.push(ContentBlock::text(std::mem::take(&mut self.text)));
        }
        for (id, name, buf) in &self.tools_closed {
            let arguments: serde_json::Value =
                serde_json::from_str(buf).unwrap_or_else(|_| serde_json::json!({}));
            content.push(ContentBlock::ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments,
            });
        }
        self.tools_closed.clear();

        let usage = if self.prefer_tail_usage {
            self.usage_tail.clone().or(self.usage_seed.clone())
        } else {
            self.usage_seed.clone().or(self.usage_tail.clone())
        }
        .unwrap_or_default();

        self.build(content, stop, err, usage)
    }

    fn build(
        &self,
        content: Vec<ContentBlock>,
        stop: StopReason,
        err: Option<String>,
        usage: Usage,
    ) -> AssistantMessage {
        AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            provider: self.provider.to_string(),
            model_id: self.model.clone(),
            content,
            stop_reason: stop,
            error_message: err,
            usage,
        }
    }

    fn next_tool_index(&self) -> usize {
        self.tools_open
            .keys()
            .next_back()
            .copied()
            .map(|i| i + 1)
            .unwrap_or(0)
    }
}

/// One live provider stream: the HTTP response body being decoded incrementally.
struct WireStream {
    wire: Box<dyn ProviderWire>,
    inner: reqwest::Response,
    decoder: SseDecoder,
    /// Line reassembly across chunk boundaries (SSE frames ignore TCP framing).
    line_buf: String,
    st: HttpEventStream,
    /// Events decoded ahead of the one already returned.
    queued: VecDeque<StreamEvent>,
    finished: bool,
}

impl WireStream {
    /// Feed received bytes; return the first newly produced event, queueing the rest.
    fn ingest(&mut self, chunk: &[u8]) -> Option<StreamEvent> {
        self.line_buf.push_str(&String::from_utf8_lossy(chunk));
        while let Some(pos) = self.line_buf.find('\n') {
            let line: String = self.line_buf.drain(..=pos).collect();
            let line = line.trim_end_matches(['\n', '\r']);
            if let Some(ev) = self.decoder.feed(line)
                && let Some(mut batch) = self.wire.on_event(&ev, &mut self.st)
            {
                let first = batch.remove(0);
                self.queued.extend(batch);
                return Some(first);
            }
        }
        None
    }
}

#[async_trait::async_trait]
impl EventStream for WireStream {
    async fn next(&mut self) -> Option<StreamEvent> {
        if let Some(ev) = self.queued.pop_front() {
            self.finished |= matches!(ev, StreamEvent::Done { .. });
            return Some(ev);
        }
        if self.finished {
            return None;
        }
        loop {
            match self.inner.chunk().await {
                Ok(Some(chunk)) => {
                    if let Some(ev) = self.ingest(&chunk) {
                        self.finished |= matches!(ev, StreamEvent::Done { .. });
                        return Some(ev);
                    }
                }
                Ok(None) => {
                    // Body exhausted: flush a trailing unterminated frame, then either
                    // deliver what the wire queued or fail honestly.
                    if let Some(ev) = self.queued.pop_front() {
                        self.finished |= matches!(ev, StreamEvent::Done { .. });
                        return Some(ev);
                    }
                    self.finished = true;
                    return Some(StreamEvent::Done {
                        message: crate::stream_fn::error_message(
                            "provider stream ended without a terminal frame",
                        ),
                    });
                }
                Err(e) => {
                    self.finished = true;
                    return Some(StreamEvent::Done {
                        message: crate::stream_fn::error_message(format!(
                            "stream read failed: {e}"
                        )),
                    });
                }
            }
        }
    }
}

/// Degenerate one-event stream for pre-stream failures.
struct OnceStream {
    ev: Option<StreamEvent>,
}

#[async_trait::async_trait]
impl EventStream for OnceStream {
    async fn next(&mut self) -> Option<StreamEvent> {
        self.ev.take()
    }
}

/// The direct-HTTP [`StreamFn`] provider (pi port of `Provider.stream`).
///
/// Routing is catalog-driven: every catalog row carries its [`WireApi`], so one
/// instance serves every listed provider. Auth comes from each model's standard env
/// var at call time; Epsilon never stores keys.
pub struct DirectProvider;

impl DirectProvider {
    /// Stateless construction (the HTTP client is process-wide pooled).
    pub fn new() -> Self {
        Self
    }
}

impl Default for DirectProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl StreamFn for DirectProvider {
    async fn stream(&self, model: &ModelRef, ctx: LlmContext) -> BoxEventStream {
        let wire = match wire_for(&model.id) {
            Ok(w) => w,
            Err(detail) => {
                return Box::new(OnceStream {
                    ev: Some(StreamEvent::Done {
                        message: crate::stream_fn::error_message(detail),
                    }),
                });
            }
        };
        let Some(key) = std::env::var_os(wire.env_key())
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string_lossy().into_owned())
        else {
            let detail = format!(
                "missing {} for model '{}' - set it in your environment",
                wire.env_key(),
                model.id
            );
            return Box::new(OnceStream {
                ev: Some(StreamEvent::Done {
                    message: crate::stream_fn::error_message(detail),
                }),
            });
        };

        let body = match wire.request_body(&model.id, &ctx) {
            Ok(b) => b,
            Err(detail) => {
                return Box::new(OnceStream {
                    ev: Some(StreamEvent::Done {
                        message: crate::stream_fn::error_message(format!(
                            "request build failed: {detail}"
                        )),
                    }),
                });
            }
        };

        let url = wire.endpoint(&model.id);
        let mut req = http_client()
            .post(&url)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream");
        for (name, value) in wire.headers(&key) {
            req = req.header(name, value);
        }

        let response = match req.body(body.to_string()).send().await {
            Ok(r) => r,
            Err(e) => {
                let kind = crate::retry::classify(&e.to_string());
                return Box::new(OnceStream {
                    ev: Some(StreamEvent::Done {
                        message: crate::stream_fn::error_message(format!(
                            "request failed ({kind:?}): {e}"
                        )),
                    }),
                });
            }
        };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let head = response.text().await.unwrap_or_default();
            let clipped: String = head.chars().take(400).collect();
            let kind = crate::retry::classify(&format!("{status} {clipped}"));
            return Box::new(OnceStream {
                ev: Some(StreamEvent::Done {
                    message: crate::stream_fn::error_message(format!(
                        "HTTP {status} ({kind:?}): {clipped}"
                    )),
                }),
            });
        }

        let st = HttpEventStream::new(&model.id, "", wire.finalize_usage_from_tail());
        Box::new(WireStream {
            wire,
            inner: response,
            decoder: SseDecoder::new(),
            line_buf: String::new(),
            st,
            queued: VecDeque::new(),
            finished: false,
        })
    }
}
