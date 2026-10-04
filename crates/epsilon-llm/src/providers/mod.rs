//! Direct HTTP provider adapters (pi port).
//!
//! Each submodule implements one provider family's real REST API; this module owns
//! the shared streaming engine. Ported from pi-0.84.2 `packages/ai/src/api/` and
//! `packages/ai/src/providers/*.ts`: providers are plain data (base URL + auth env
//! var), the wire format is code we own, and every failure surfaces in-band as a
//! `Done { stop_reason: Error }` event per the StreamFn contract.

pub mod anthropic;
pub mod bedrock;
pub mod github_copilot;
pub mod google;
pub mod mistral;
pub mod openai_compat;
pub mod openai_responses;
pub mod vertex;
use anthropic::AnthropicWire;
use bedrock::BedrockConverseWire;
use google::GoogleWire;
use mistral::MistralConversationsWire;
use openai_compat::OpenAiCompatWire;
use openai_responses::OpenAiResponsesWire;

use std::collections::{BTreeMap, VecDeque};

// `Response::chunk()` is an inherent reqwest method, but the boxed `Stream` field is not:
// `.next()` is `futures_util::StreamExt`. Without this the `real` feature does not compile.
use futures_util::StreamExt;

use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::message::{AssistantMessage, StopReason, Usage};
use epsilon_contracts::stream::StreamEvent;

pub use crate::http::SseEvent;
use crate::http::{SseDecoder, http_client};
use crate::stream_fn::{BoxEventStream, EventStream, LlmContext, ModelRef, StreamFn};

// WireApi lives in `crate::catalog` (it is catalog data); re-exported here for the
// engine's internal use.
pub(crate) use crate::catalog::WireApi;

/// Credential-extra key holding a Copilot Business/Enterprise domain (M55).
/// Written by a login whose `[oauth.github-copilot] enterprise_domain` is set;
/// the wire builds `https://copilot-api.<domain>` from it when the minted token
/// carries no `proxy-ep` (pi `getGitHubCopilotBaseUrl` fallback order).
pub const COPILOT_ENTERPRISE_DOMAIN_KEY: &str = crate::auth::ENTERPRISE_DOMAIN_ENV_KEY;

/// Per-provider wire implementation: request building + SSE interpretation.
///
/// Port shape follows pi's `Api` interface (`buildParams` / stream-event mapping /
/// stop-reason mapping) as a Rust object-safety boundary.
pub trait ProviderWire: Send + Sync {
    /// Full streaming URL for this model.
    fn endpoint(&self, model: &str) -> String;
    /// Streaming URL when the credential can move it.
    ///
    /// Most providers have one fixed host, and the default answers exactly like
    /// [`ProviderWire::endpoint`]. GitHub Copilot mints its credential per
    /// account and encodes the API host inside it (`proxy-ep`), so its wire
    /// overrides this and the transport prefers it wherever a resolved key is
    /// available.
    fn endpoint_for(&self, model: &str, _key: &str) -> String {
        self.endpoint(model)
    }
    /// Auth env var consulted for the key.
    fn env_key(&self) -> &str;
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

/// Resolve the wire implementation for a model id via its catalog entry, with
/// user-declared custom providers (M28) as the fallback for unknown ids.
///
/// A custom id has the form `<name>/<model>` where `<name>` matches a
/// `[provider_custom.<name>]` row in the global config; it rides the
/// OpenAI-compat wire to that row's `base_url` (opencode's
/// `provider.options.baseURL`, Hermes' `base_url_override`). Known catalog ids
/// always win over customs — a custom row cannot shadow a real provider.
pub(crate) fn wire_for(
    model_id: &str,
    customs: &[crate::custom::CustomProvider],
) -> Result<Box<dyn ProviderWire>, String> {
    if let Some(info) = crate::catalog::lookup_model(model_id) {
        return Ok(match &info.wire {
            WireApi::AnthropicMessages => Box::new(AnthropicWire),
            WireApi::OpenAiCompletions { base_url, env_key } => Box::new(OpenAiCompatWire {
                base_url: base_url.to_string(),
                env_key: env_key.map(str::to_string),
            }),
            WireApi::OpenAiResponses { base_url, env_key } => Box::new(OpenAiResponsesWire {
                base_url: base_url.to_string(),
                env_key: env_key.map(str::to_string),
            }),
            WireApi::MistralConversations { base_url, env_key } => {
                Box::new(MistralConversationsWire {
                    base_url: base_url.to_string(),
                    env_key: env_key.map(str::to_string),
                })
            }
            WireApi::GitHubCopilot {
                base_url,
                env_key,
                anthropic,
            } => {
                // M55: Copilot re-serves both dialects behind one host. The
                // row already picked the family; build that inner wire (with
                // the Copilot base so any delegation slip still lands on the
                // right host) and layer Copilot auth + URL resolution on top.
                let inner: Box<dyn ProviderWire> = if *anthropic {
                    Box::new(AnthropicWire)
                } else {
                    Box::new(OpenAiCompatWire {
                        base_url: base_url.to_string(),
                        env_key: env_key.map(str::to_string),
                    })
                };
                // Enterprise logins store the host they authenticated against
                // (pi `credential.enterpriseUrl`); individual accounts have none.
                let enterprise = crate::auth::load_credentials_in(
                    &crate::auth::global_dir().unwrap_or_default(),
                )
                .get("github-copilot")
                .and_then(|c| c.env())
                .and_then(|env| env.get(COPILOT_ENTERPRISE_DOMAIN_KEY))
                .cloned();
                Box::new(github_copilot::CopilotWire::new(
                    base_url,
                    *env_key,
                    enterprise.as_deref(),
                    *anthropic,
                    inner,
                ))
            }
            WireApi::GoogleGenerativeAi => Box::new(GoogleWire),
            WireApi::GoogleVertex => {
                // Project/location ride the stored credential's `env` map (pi's
                // login stores them there); ambient env is the fallback; ADC
                // default path when no file is configured. Missing pieces die
                // here with a telling error, not at the endpoint.
                let dir = crate::auth::global_dir().unwrap_or_default();
                let stored_env = crate::auth::load_credentials_in(&dir)
                    .get("google-vertex")
                    .and_then(|c| c.env())
                    .cloned()
                    .unwrap_or_default();
                let pick = |k: &str| {
                    stored_env
                        .get(k)
                        .cloned()
                        .or_else(|| std::env::var(k).ok())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                };
                let (Some(project), Some(location)) = (
                    pick("GOOGLE_CLOUD_PROJECT").or_else(|| pick("GCLOUD_PROJECT")),
                    pick("GOOGLE_CLOUD_LOCATION"),
                ) else {
                    return Err("google-vertex needs GOOGLE_CLOUD_PROJECT and GOOGLE_CLOUD_LOCATION (or `epsilon auth login google-vertex` storing them) — pi resolveProject/resolveLocation".into());
                };
                Box::new(vertex::GoogleVertexWire {
                    project,
                    location,
                    credentials_file: pick("GOOGLE_APPLICATION_CREDENTIALS"),
                })
            }
            WireApi::BedrockConverse => Box::new(BedrockConverseWire),
        });
    }
    let Some((name, _rest)) = model_id.split_once('/') else {
        return Err(format!(
            "unknown model '{model_id}' - pick one from /models or declare \
             [provider_custom.<name>] with base_url"
        ));
    };
    match customs.iter().find(|c| c.name == name.to_ascii_lowercase()) {
        Some(c) => Ok(Box::new(OpenAiCompatWire {
            base_url: c.base_url.clone(),
            env_key: (!c.env_key.is_empty()).then(|| c.env_key.clone()),
        })),
        None => Err(format!(
            "unknown provider '{name}' in '{model_id}' - no catalog row and no \
             [provider_custom.{name}] declaration"
        )),
    }
}

/// Streaming accumulator shared by all wire implementations.
///
/// Owns the partial assistant message, tool-call argument assembly (indexed, since
/// OpenAI interleaves calls), usage capture, and final `Done` synthesis.
pub struct HttpEventStream {
    model: String,
    provider: &'static str,
    text: String,
    thinking: String,
    /// OpenAI-style indexed tool calls: index → (id, name, json buffer).
    tools_open: BTreeMap<usize, (String, String, String)>,
    /// Completed tool-call buffers awaiting finalization: (index, id, name,
    /// json buffer). The index keeps Converse emission order for wires whose
    /// text and tool blocks interleave (Bedrock); other wires ignore it.
    tools_closed: Vec<(usize, String, String, String)>,
    finish_reason: Option<String>,
    usage_seed: Option<Usage>,
    usage_tail: Option<Usage>,
    /// Wire-level preference: tail usage wins at Done time (Anthropic).
    prefer_tail_usage: bool,
    /// Mistral id-normalizer side-table: seed → assigned id, plus the
    /// reverse set for collision checks (pi's closure maps, re-expressed —
    /// see `mistral::normalize_tool_call_id`).
    mistral_ids: BTreeMap<String, String>,
}

impl HttpEventStream {
    /// Fresh accumulator for one provider response (`provider` is the wire's
    /// static name; `prefer_tail_usage` picks which usage shape wins when a wire
    /// emits both, see Mistral/Bedrock).
    pub fn new(model: &str, provider: &'static str, prefer_tail_usage: bool) -> Self {
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
            mistral_ids: BTreeMap::new(),
        }
    }

    /// Empty stream state for fixture tests (wire adapters drive on_event directly).
    pub fn empty_for_tests() -> Self {
        Self::new("test-model", "test", false)
    }

    /// Fresh accumulator state for a restarted stream body.
    pub(crate) fn reset_accumulator(&mut self) {
        self.text.clear();
        self.thinking.clear();
        self.tools_open.clear();
        self.tools_closed.clear();
        self.finish_reason = None;
        self.mistral_ids.clear();
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
        // K1: Anthropic always speaks about caching, so the mere presence of the
        // keys is the signal — not a non-zero count. A first call that legitimately
        // missed still reports both keys as 0, and that is a real 0% worth showing.
        u.cache_reported = usage.get("cache_read_input_tokens").is_some()
            || usage.get("cache_creation_input_tokens").is_some();
        if u.input + u.output > 0 {
            self.usage_seed = Some(u);
        }
    }

    /// Compat providers (OpenAI chunk.usage / Gemini usageMetadata): key-shape
    /// tolerant extraction into our Usage.
    ///
    /// The cached prefix is removed from `input`, because the wire format counts it in
    /// both places. pi states the rule in two places and cites the providers:
    ///
    /// > OpenAI includes cached and cache-write tokens in input_tokens, so subtract both.
    /// > — `openai-responses-shared.ts:567`; the same subtraction at
    /// > `openai-completions.ts:1534`, with the per-provider key shapes
    /// > (`prompt_tokens_details.cached_tokens` for OpenAI/OpenRouter,
    /// > `prompt_cache_hit_tokens` for DeepSeek, top-level `cached_tokens` for Kimi)
    /// > and the reason cache *writes* are not folded into the read count.
    ///
    /// Leaving the prefix in `input` is not a cosmetic disagreement. Three separate
    /// numbers downstream all read the buckets as disjoint, and each goes wrong in a way
    /// the user cannot see: the cache hit rate falls (its denominator is inflated), the
    /// context gauge rises (the anchor is the inflated prompt), and the cost rises twice
    /// over (the same tokens are charged at the input rate *and* at the cache-read rate).
    ///
    /// `saturating_sub` on each term, rather than one subtraction of the sum, because a
    /// provider may report a cached count larger than the prompt — a proxy aggregating
    /// several requests into one usage block will. Plain `u64` arithmetic wraps to about
    /// 1.8e19 there, and the header would print a garbage percentage rather than fail.
    pub(crate) fn seed_usage_compat(&mut self, usage: &serde_json::Value) {
        let get_u64 = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| usage.get(*k).and_then(|v| v.as_u64()))
                .unwrap_or(0)
        };
        let cache_read = usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                usage
                    .get("prompt_cache_hit_tokens")
                    .and_then(|v| v.as_u64())
            })
            .or_else(|| usage.get("cached_tokens").and_then(|v| v.as_u64()))
            .unwrap_or_else(|| get_u64(&["cache_read_input_tokens", "cachedContentTokenCount"]));
        let cache_write = usage
            .pointer("/prompt_tokens_details/cache_write_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| get_u64(&["cache_creation_input_tokens"]));
        let prompt = get_u64(&["prompt_tokens", "input_tokens", "promptTokenCount"]);
        // K1: did the body actually *carry* a cache key? Detection, not arithmetic —
        // `cache_read == 0` is equally what a wire that never mentions caching
        // produces, and only the key's presence tells the two apart.
        let cache_reported = [
            "/prompt_tokens_details/cached_tokens",
            "/prompt_tokens_details/cache_write_tokens",
            "/input_tokens_details/cached_tokens",
        ]
        .iter()
        .any(|p| usage.pointer(p).is_some())
            || [
                "prompt_cache_hit_tokens",
                "cached_tokens",
                "cache_read_input_tokens",
                "cache_creation_input_tokens",
                "cachedContentTokenCount",
            ]
            .iter()
            .any(|k| usage.get(*k).is_some());
        let u = Usage {
            input: prompt
                .saturating_sub(cache_read)
                .saturating_sub(cache_write),
            output: get_u64(&["completion_tokens", "output_tokens", "candidatesTokenCount"]),
            cache_read,
            cache_write,
            total_tokens: get_u64(&["total_tokens", "totalTokenCount"]),
            cache_reported,
        };
        if u.input + u.output > 0 {
            self.usage_tail = Some(u);
        }
    }

    /// Append a wire text fragment so `finish()` can rebuild `Done` (pi semantics + deepseek fix).
    pub(crate) fn append_text(&mut self, frag: &str) {
        self.text.push_str(frag);
    }

    /// Append a wire reasoning/thinking fragment.
    pub(crate) fn append_thinking(&mut self, frag: &str) {
        self.thinking.push_str(frag);
    }

    /// Anthropic `message_delta.usage`: fields present overwrite, absent keep seed
    /// (pi preserves `input_tokens` from message_start when proxies omit it here).
    pub(crate) fn update_usage_tail(&mut self, usage: &serde_json::Value) {
        let mut u = self.usage_seed.unwrap_or_default();
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

    /// Overwrite the argument buffer of an explicit tool call slot (Responses
    /// `output_item.done` carries the final arguments snapshot, which wins
    /// over delta fragments — pi parses `item.arguments || partialJson`).
    pub(crate) fn set_tool_json_at(&mut self, idx: usize, args: &str) {
        match self.tools_open.get_mut(&idx) {
            Some(entry) => {
                entry.2.clear();
                entry.2.push_str(args);
            }
            None => {
                self.tools_open.insert(
                    idx,
                    (
                        format!("call_{idx}"),
                        format!("tool_{idx}"),
                        args.to_string(),
                    ),
                );
            }
        }
    }

    /// Close the most recent open call (Anthropic/Gemini content_block_stop).
    /// Index recorded as `usize::MAX` (order-insensitive wires never mix text
    /// slots in, so close order is final order for them).
    pub(crate) fn close_tool_call(&mut self) {
        if let Some(last) = self.tools_open.keys().next_back().copied()
            && let Some((id, name, buf)) = self.tools_open.remove(&last)
        {
            self.tools_closed.push((usize::MAX, id, name, buf));
        }
    }

    /// Open a Bedrock text slot at an explicit index (Converse text blocks
    /// have no Start frame — pi creates them in the delta handler). Reuses
    /// the indexed tool-call table: text slots hold `( bed_text_id, "", buf )`
    /// with an empty name so `finish()` can tell them apart.
    pub(crate) fn open_bedrock_text_slot(&mut self, idx: usize) {
        self.tools_open
            .entry(idx)
            .or_insert_with(|| (format!("bedrock_text_{idx}"), String::new(), String::new()));
    }

    /// Append text to a Bedrock text slot (no-op when the slot isn't text).
    pub(crate) fn append_text_to_slot(&mut self, idx: usize, frag: &str) {
        if let Some(entry) = self.tools_open.get_mut(&idx)
            && entry.1.is_empty()
        {
            entry.2.push_str(frag);
        }
    }

    /// Close a Bedrock slot by index: text slots flush into `text` in index
    /// order at finish time; tool slots keep their index for emission-order
    /// merge. (Index order = content order — pi's `blocks` array is
    /// append-in-index-order too, since Converse emits block 0, 1, 2…
    /// sequentially.)
    pub(crate) fn close_bedrock_slot(&mut self, idx: usize) {
        if let Some((id, name, buf)) = self.tools_open.remove(&idx) {
            self.tools_closed.push((idx, id, name, buf));
        }
    }

    /// Converse usage split (pi `handleMetadata`): `inputTokens` /
    /// `outputTokens` / `cacheReadInputTokens` / `cacheWriteInputTokens`,
    /// `totalTokens` or recomputed.
    pub(crate) fn seed_usage_bedrock(&mut self, usage: &serde_json::Value) {
        let get = |k: &str| usage.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
        let mut u = Usage::default();
        u.input = get("inputTokens");
        u.output = get("outputTokens");
        u.cache_read = get("cacheReadInputTokens");
        u.cache_write = get("cacheWriteInputTokens");
        // **The presence of the keys is the signal, not a non-zero count** (K1). Bedrock
        // always speaks about caching when it speaks at all, so a usage block carrying
        // `cacheReadInputTokens` is a provider that reports cache traffic — and the header
        // printing `CH n/a` beside a live `R184k W12k` is the exact lie K1 was written to
        // end, reintroduced for the one parser that was not in that fix.
        //
        // The two sibling parsers (Anthropic `:277-281`, the compat family `:334-349`)
        // both set this. This one parsed the counts and forgot the flag, which is exactly
        // how a per-parser boolean rots: there is no trait member for it, so each wire
        // decides alone and only the ones someone thought about are right.
        u.cache_reported = usage.get("cacheReadInputTokens").is_some()
            || usage.get("cacheWriteInputTokens").is_some();
        u.total_tokens = usage
            .get("totalTokens")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| u.input + u.output);
        if u.input + u.output > 0 {
            self.usage_tail = Some(u);
        }
    }

    /// Any tool activity recorded (drives ToolUse stop mapping on [DONE]).
    /// Accumulated assistant text so far (M60: the stall salvage path must know
    /// whether anything user-visible was produced before the provider vanished).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Accumulated thinking text so far.
    pub fn thinking(&self) -> &str {
        &self.thinking
    }

    /// Whether any tool call has been opened or closed on this stream — the
    /// signal that a cut-off response may carry half-written arguments.
    pub fn has_open_or_closed_tools(&self) -> bool {
        !self.tools_open.is_empty() || !self.tools_closed.is_empty()
    }

    /// The terminal message for a FAILED stream (M60/M62): one salvage rule for
    /// every mid-stream failure — a transport reset, a provider error frame, the
    /// stall watchdog, a stream that ends with no terminal frame.
    ///
    /// The rule: **whatever the user already watched arrive stays.** Losing
    /// visible output is worse than an ugly transcript, and discarding it also
    /// throws away partial reasoning the next turn could build on. Two honesty
    /// constraints ride along:
    ///
    /// - the turn is flagged `Error`, never presented as a complete answer;
    /// - tool calls in flight are dropped, because their arguments may be
    ///   half-written JSON and executing a half-written `bash` call is worse than
    ///   failing the turn (any prose beside them is kept).
    ///
    /// Lives on the accumulator so wire fixtures can drive it directly
    /// (`tests/wire_fixtures.rs`); when nothing arrived it degrades to a plain
    /// error message.
    pub fn failure_message(
        &mut self,
        detail: impl Into<String>,
    ) -> epsilon_contracts::message::AssistantMessage {
        let detail = detail.into();
        if self.has_open_or_closed_tools() {
            let mut message = crate::stream_fn::error_message(detail);
            if !self.text.is_empty() {
                message.content = vec![epsilon_contracts::content::ContentBlock::text(&self.text)];
            }
            return message;
        }
        if !self.text.is_empty() || !self.thinking.is_empty() {
            // Flush what arrived and flag the turn as failed: salvage, never pretend.
            return self.finish(epsilon_contracts::message::StopReason::Error, Some(detail));
        }
        crate::stream_fn::error_message(detail)
    }

    /// [`HttpEventStream::failure_message`] with the stall watchdog's wording.
    pub fn stall_message(&mut self, secs: u64) -> epsilon_contracts::message::AssistantMessage {
        self.failure_message(format!(
            "provider stream timed out: no data for {secs}s — treating it as a network failure"
        ))
    }

    /// Remember a provider finish reason seen mid-stream (OpenAI finish_reason).
    pub(crate) fn note_finish_reason(&mut self, reason: &str) {
        self.finish_reason = Some(reason.to_string());
    }

    /// The stored wire-level finish reason, if any (read back at terminal mapping).
    pub fn finish_reason(&self) -> Option<&str> {
        self.finish_reason.as_deref()
    }

    /// Every key shape Mistral has used for cached prompt tokens (pi `getMistralCachedPromptTokens`).
    ///
    /// **One list, read twice.** The parse needs the first shape that carries a number;
    /// `cache_reported` needs to know whether *any* of them was present at all. Those are two
    /// different questions about one fact — whether this provider's usage block speaks about
    /// caching — and they were answered by two hand-written reads until the flag was added and
    /// immediately diverged from the parse sitting beside it.
    const MISTRAL_CACHE_POINTERS: &[&str] = &[
        "/prompt_tokens_details/cached_tokens",
        "/prompt_tokens_details/cachedTokens",
        "/promptTokensDetails/cachedTokens",
        "/promptTokensDetails/cached_tokens",
        "/prompt_token_details/cached_tokens",
        "/prompt_token_details/cachedTokens",
        "/promptTokenDetails/cachedTokens",
        "/promptTokenDetails/cached_tokens",
    ];

    /// Mistral cached-token usage (pi `getMistralCachedPromptTokens`): reads
    /// every key shape Mistral has used — `prompt_tokens_details.cached_tokens`
    /// (plus the camelCase twins `cachedTokens` / `promptTokensDetails` and
    /// `num_{cached_}tokens`) — clamps to `[0, prompt_tokens]`, and stores
    /// `input = prompt - cached`, `cache_read = cached` (pi's exact split).
    pub(crate) fn seed_usage_mistral(&mut self, usage: &serde_json::Value) {
        let prompt = usage
            .get("prompt_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let mut raw: u64 = 0;
        for ptr in Self::MISTRAL_CACHE_POINTERS.iter().copied() {
            if let Some(v) = usage.pointer(ptr).and_then(|v| v.as_u64()) {
                raw = v;
                break;
            }
        }
        if raw == 0 {
            for key in ["num_cached_tokens", "numCachedTokens"] {
                if let Some(v) = usage.get(key).and_then(|v| v.as_u64()) {
                    raw = v;
                    break;
                }
            }
        }
        // Non-numeric cached shapes (null, strings) read as 0 — pi's
        // `typeof rawCachedTokens === "number"` guard.
        let cached = raw.min(prompt);
        let mut u = Usage::default();
        u.input = prompt.saturating_sub(cached);
        u.output = usage
            .get("completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        u.cache_read = cached;
        // **Same rule as Anthropic and Bedrock (K1):** Mistral parsed eight spellings of
        // "cached tokens" here, and then reported that it does not report caching at all —
        // so a session with a working prompt cache printed `CH n/a` next to the tokens it
        // had just moved. The flag says the provider *speaks about* caching, which is a
        // property of its usage shape and not of whether this particular turn hit the cache.
        u.cache_reported = Self::MISTRAL_CACHE_POINTERS
            .iter()
            .any(|p| usage.pointer(p).is_some());
        u.total_tokens = usage
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| u.input + u.output + u.cache_read);
        if u.input + u.output > 0 {
            self.usage_tail = Some(u);
        }
    }

    /// Mistral id-normalizer side-table: has `candidate` been assigned to a
    /// seed other than `seed` (pi's `reverseMap` check)?
    pub(crate) fn mistral_id_taken(&self, candidate: &str, seed: &str) -> bool {
        self.mistral_ids
            .iter()
            .any(|(s, id)| id == candidate && s != seed)
    }

    /// Record a seed → id assignment (pi's `idMap` + `reverseMap` set).
    /// Both directions key off the SEED: `mistral_id_taken` reports a
    /// collision only when the same candidate was assigned to a DIFFERENT
    /// seed, and pass-through ids memo under themselves. Both conditions
    /// together reproduce pi's `owner === id` check.
    pub(crate) fn remember_mistral_id(&mut self, seed: &str, id: &str) {
        self.mistral_ids.insert(seed.to_string(), id.to_string());
    }

    /// Look up a previous seed → id assignment (stability across chunks).
    pub(crate) fn mistral_id_for(&self, seed: &str) -> Option<String> {
        self.mistral_ids.get(seed).cloned()
    }

    // ---- terminal assembly -------------------------------------------------------------

    /// Build the final assistant message for `Done`.
    pub(crate) fn finish(&mut self, stop: StopReason, err: Option<String>) -> AssistantMessage {
        // Flush any tool calls never explicitly closed ([DONE] without stop frames).
        // Bedrock text slots (empty name) collect separately for the
        // emission-order merge below; every other entry keeps close order.
        let mut last_keys: Vec<usize> = self.tools_open.keys().copied().collect();
        last_keys.sort_unstable();
        let mut bedrock_text: Vec<(usize, String)> = Vec::new();
        for key in last_keys {
            if let Some((id, name, buf)) = self.tools_open.remove(&key) {
                if name.is_empty() && id.starts_with("bedrock_text_") {
                    bedrock_text.push((key, buf));
                } else {
                    self.tools_closed.push((usize::MAX, id, name, buf));
                }
            }
        }

        let mut content: Vec<ContentBlock> = Vec::new();
        if !self.thinking.is_empty() {
            content.push(ContentBlock::Thinking {
                text: std::mem::take(&mut self.thinking),
                // No wire here parses a reasoning signature (Anthropic's `signature_delta` is
                // not read), so a block built from a live turn has none. The field carries what
                // a session file brought in, not a value invented at assembly time.
                signature: None,
            });
        }
        if !self.text.is_empty() {
            content.push(ContentBlock::text(std::mem::take(&mut self.text)));
        }
        // Emission-order merge: Bedrock text slots carry their Converse block
        // index (sorted); tool slots closed by index carry theirs too; slots
        // closed by recency (other wires, `usize::MAX`) keep close order at
        // the end. Converse emits block N's Stop before block N+1's Start, so
        // close order == index order within each run — the merge reproduces
        // pi's mixed `blocks` array ("narrate then call" interleavings intact).
        let mut merged: Vec<(usize, usize, ContentBlock)> = Vec::new();
        for (idx, buf) in bedrock_text.into_iter().filter(|(_, b)| !b.is_empty()) {
            merged.push((idx, 0, ContentBlock::text(buf)));
        }
        for (seq, (idx, id, name, buf)) in self.tools_closed.iter().enumerate() {
            // Sanitization (M70): recover what can be recovered, and when arguments cannot be
            // read, hand the raw text on instead of an empty object. `{}` used to make the tool
            // run with no arguments at all, which turned "the model sent malformed JSON" into
            // "you forgot the path field".
            let arguments = match crate::sanitize::parse_tool_arguments(buf) {
                crate::sanitize::ParsedArgs::Valid(v)
                | crate::sanitize::ParsedArgs::Repaired { value: v, .. } => v,
                crate::sanitize::ParsedArgs::Unrecoverable { raw, .. } => {
                    serde_json::Value::String(raw)
                }
            };
            // Sort key: real indices first (emission order), recency-closed
            // slots after in close order (seq breaks the MAX ties stably).
            let order = if *idx == usize::MAX { usize::MAX } else { *idx };
            merged.push((
                order,
                seq + 1,
                ContentBlock::ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments,
                },
            ));
        }
        merged.sort_by_key(|(order, seq, _)| (*order, *seq));
        // Duplicate ids within one message are rejected by providers and make result pairing
        // ambiguous; later duplicates get a deterministic suffix.
        let mut ids: Vec<String> = merged
            .iter()
            .filter_map(|(_, _, b)| match b {
                ContentBlock::ToolCall { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        if crate::sanitize::unique_call_ids(&mut ids) > 0 {
            let mut next = ids.into_iter();
            for (_, _, block) in merged.iter_mut() {
                if let ContentBlock::ToolCall { id, .. } = block {
                    *id = next.next().unwrap_or_default();
                }
            }
        }
        for (_, _, block) in merged {
            content.push(block);
        }
        self.tools_closed.clear();

        let usage = if self.prefer_tail_usage {
            self.usage_tail.or(self.usage_seed)
        } else {
            self.usage_seed.or(self.usage_tail)
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

/// No bytes for this long on a LIVE stream means the provider is gone: a
/// half-open socket (VPN drop, NAT timeout, proxy that swallowed the FIN) leaves
/// `chunk()` pending forever, so the turn would hang with a spinner and no way
/// out. Long streams are legitimate — silence is not.
const STREAM_IDLE_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

/// Environment override for [`STREAM_IDLE_LIMIT`], in seconds; `0` disables the
/// watchdog (for providers that legitimately go quiet for minutes, e.g. a
/// reasoning model holding its content back).
fn stream_idle_limit() -> std::time::Duration {
    match std::env::var("EPSILON_STREAM_IDLE_SECS") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => std::time::Duration::ZERO,
            Ok(secs) => std::time::Duration::from_secs(secs),
            Err(_) => STREAM_IDLE_LIMIT,
        },
        Err(_) => STREAM_IDLE_LIMIT,
    }
}

/// One live provider stream: the HTTP response body being decoded incrementally.
struct WireStream {
    wire: Box<dyn ProviderWire>,
    /// The response body as a byte stream rather than a `reqwest::Response`.
    ///
    /// F03 needed a test, and there was no way to write one: this was a concrete
    /// `reqwest::Response`, so a fixture could only be exercised over a real socket, and
    /// every existing test drives the *codecs* directly and never this type. It is now a
    /// boxed byte stream — `Response::bytes_stream()` is exactly `Stream<Item =
    /// Result<Bytes>>`, so production pays one `.boxed()` and the chunk-partition
    /// behaviour becomes drivable from a `Vec` of chunks in a unit test.
    inner: std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Option<reqwest::Result<bytes::Bytes>>> + Send>,
    >,
    decoder: SseDecoder,
    /// StreamFolder needs `Start` before any delta; emit it on first wire batch.
    started: bool,
    /// Line reassembly across chunk boundaries (SSE frames ignore TCP framing).
    /// Raw BYTES, not a String: a multi-byte UTF-8 char split across TCP chunks must
    /// not be lossy-decoded mid-char (plan 006) — decoding happens only on complete
    /// lines, where any replacement is the provider's own damage.
    line_buf: Vec<u8>,
    st: HttpEventStream,
    /// Events decoded ahead of the one already returned.
    queued: VecDeque<StreamEvent>,
    finished: bool,
    /// Watchdog: max silence tolerated between body chunks (M60). Zero = off.
    idle_limit: std::time::Duration,
}

impl WireStream {
    /// Feed received bytes; return the first newly produced event, queueing the rest.
    ///
    /// The loop must run to the end of the *chunk*, not stop at the first event-producing
    /// line. F03: it used to `return` from inside the loop, so a provider that delivered a
    /// whole response in one TCP chunk had every frame after the first left sitting in
    /// `line_buf` — and `next()` never re-checked `line_buf` before awaiting the next chunk,
    /// so those frames were not merely delayed. If the body then ended, the stream ended
    /// cleanly and the frames were simply gone. The visible symptom is a truncated answer
    /// that the harness reports as a *success*.
    ///
    /// So: drain every complete line in the chunk, push every event onto `queued`, and hand
    /// back one. `next()` already drains `queued` before it awaits, so the rest come out on
    /// the following calls without touching the network.
    fn ingest(&mut self, chunk: &[u8]) -> Option<StreamEvent> {
        self.line_buf.extend_from_slice(chunk);
        while let Some(pos) = self.line_buf.iter().position(|b| *b == b'\n') {
            let line_bytes: Vec<u8> = self.line_buf.drain(..=pos).collect();
            let trimmed: &[u8] = line_bytes
                .strip_suffix(b"\n")
                .unwrap_or(&line_bytes)
                .strip_suffix(b"\r")
                .unwrap_or(line_bytes.strip_suffix(b"\n").unwrap_or(&line_bytes));
            let line = String::from_utf8_lossy(trimmed).into_owned();
            if let Some(ev) = self.decoder.feed(&line)
                && let Some(mut batch) = self.wire.on_event(&ev, &mut self.st)
            {
                if !self.started
                    && !matches!(
                        batch.first(),
                        Some(epsilon_contracts::stream::StreamEvent::Start { .. })
                    )
                {
                    self.started = true;
                    batch.insert(
                        0,
                        epsilon_contracts::stream::StreamEvent::Start {
                            partial: self.st.partial(),
                        },
                    );
                } else {
                    self.started = true;
                }
                // Queue the whole batch, keep going. Returning here is the bug.
                self.queued.extend(batch);
            }
        }
        self.queued.pop_front()
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
            // M60 watchdog: a body that stops producing bytes must not hang the
            // turn forever. Silence longer than the limit is a network failure as
            // far as the retry policy is concerned — the message says "timed out"
            // so `retry::classify` routes it to `FailureKind::Network`.
            // `StreamExt::next` yields `Option<Item>`; `timeout` wraps that in a
            // `Result`. Flatten both levels into the same `Option<Result<Bytes, _>>`
            // the old `Response::chunk()` returned, so every arm below is unchanged.
            let chunk: Option<Result<bytes::Bytes, reqwest::Error>> = if self.idle_limit.is_zero() {
                self.inner.next().await.flatten()
            } else {
                match tokio::time::timeout(self.idle_limit, self.inner.next()).await {
                    Ok(inner) => inner.flatten(),
                    Err(_elapsed) => {
                        self.finished = true;
                        let message = self.st.stall_message(self.idle_limit.as_secs());
                        return Some(StreamEvent::Done { message });
                    }
                }
            };
            match chunk {
                Some(Ok(chunk)) => {
                    if let Some(ev) = self.ingest(&chunk) {
                        self.finished |= matches!(ev, StreamEvent::Done { .. });
                        return Some(ev);
                    }
                }
                // Body exhausted (stream yielded `None` or ended): flush a trailing
                // unterminated frame or unterminated SSE data line (DeepSeek sometimes
                // ends without a trailing newline), then deliver any queued events or the
                // buffered partial, and only then fail honestly.
                None => {
                    if let Some(ev) = self.queued.pop_front() {
                        self.finished |= matches!(ev, StreamEvent::Done { .. });
                        return Some(ev);
                    }
                    // Flush an unterminated `data: ...` line still sitting in line_buf
                    // (no trailing `\n` on the last chunk). This is what makes empty-
                    // history Hello die as `provider stream ended without a terminal
                    // frame` even though the provider did send a valid Done.
                    if !self.line_buf.is_empty() {
                        let trimmed: Vec<u8> = self.line_buf.drain(..).collect();
                        let trimmed: Vec<u8> =
                            trimmed.strip_suffix(b"\r").unwrap_or(&trimmed).to_vec();
                        let line = String::from_utf8_lossy(&trimmed).into_owned();
                        if let Some(ev) = self.decoder.feed(&line)
                            && let Some(mut batch) = self.wire.on_event(&ev, &mut self.st)
                        {
                            if !self.started
                                && !matches!(
                                    batch.first(),
                                    Some(epsilon_contracts::stream::StreamEvent::Start { .. })
                                )
                            {
                                self.started = true;
                                batch.insert(
                                    0,
                                    epsilon_contracts::stream::StreamEvent::Start {
                                        partial: self.st.partial(),
                                    },
                                );
                            } else {
                                self.started = true;
                            }
                            self.queued.extend(batch);
                            if let Some(ev1) = self.queued.pop_front() {
                                self.finished |= matches!(ev1, StreamEvent::Done { .. });
                                return Some(ev1);
                            }
                        }
                        if let Some(ev0) = self.decoder.feed("")
                            && let Some(mut batch) = self.wire.on_event(&ev0, &mut self.st)
                        {
                            if !self.started
                                && !matches!(
                                    batch.first(),
                                    Some(epsilon_contracts::stream::StreamEvent::Start { .. })
                                )
                            {
                                self.started = true;
                                batch.insert(
                                    0,
                                    epsilon_contracts::stream::StreamEvent::Start {
                                        partial: self.st.partial(),
                                    },
                                );
                            } else {
                                self.started = true;
                            }
                            self.queued.extend(batch);
                            if let Some(ev1) = self.queued.pop_front() {
                                self.finished |= matches!(ev1, StreamEvent::Done { .. });
                                return Some(ev1);
                            }
                        }
                    } else if let Some(ev0) = self.decoder.feed("") {
                        // No line_buf leftover but decoder has buffered data lines
                        // waiting for the blank-line terminator.
                        if let Some(mut batch) = self.wire.on_event(&ev0, &mut self.st) {
                            if !self.started
                                && !matches!(
                                    batch.first(),
                                    Some(epsilon_contracts::stream::StreamEvent::Start { .. })
                                )
                            {
                                self.started = true;
                                batch.insert(
                                    0,
                                    epsilon_contracts::stream::StreamEvent::Start {
                                        partial: self.st.partial(),
                                    },
                                );
                            } else {
                                self.started = true;
                            }
                            self.queued.extend(batch);
                            if let Some(ev1) = self.queued.pop_front() {
                                self.finished |= matches!(ev1, StreamEvent::Done { .. });
                                return Some(ev1);
                            }
                        }
                    }
                    // If we buffered real content/thinking, synthesize a Done from it
                    // rather than swallowing the reply as an opaque error.
                    if !self.st.text.is_empty()
                        || !self.st.thinking.is_empty()
                        || self.st.has_open_or_closed_tools()
                        || self.st.finish_reason().is_some()
                    {
                        let has_calls = self.st.has_open_or_closed_tools();
                        let (stop, err) = self.wire.map_stop(self.st.finish_reason(), has_calls);
                        let msg = self.st.finish(stop, err);
                        self.finished = true;
                        return Some(StreamEvent::Done { message: msg });
                    }
                    self.finished = true;
                    return Some(StreamEvent::Done {
                        message: crate::stream_fn::error_message(
                            "provider stream ended without a terminal frame",
                        ),
                    });
                }
                Some(Err(e)) => {
                    // M62: a socket that dies mid-answer must not take the answer
                    // with it — keep what was decoded and flag the turn as failed.
                    self.finished = true;
                    let message = self.st.failure_message(format!("stream read failed: {e}"));
                    return Some(StreamEvent::Done { message });
                }
            }
        }
    }
}

/// Degenerate one-event stream for pre-stream failures.
struct OnceStream {
    ev: Option<StreamEvent>,
}

/// Bedrock credential chain (pi `getConfiguredBedrockCredentials` + the SDK
/// default chain it defers to, minus network hops): explicit
/// `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` (+ optional
/// `AWS_SESSION_TOKEN`) win; `AWS_PROFILE` selects a shared-config profile
/// (parsed from `~/.aws/credentials` + `~/.aws/config` — no SDK needed for
/// the static-key case); ECS/EC2 metadata endpoints are runtime network and
/// stay out (documented gap — the error names them).
#[cfg(feature = "real")]
fn bedrock_chain_credentials() -> Option<(String, String, Option<String>)> {
    let key = std::env::var("AWS_ACCESS_KEY_ID")
        .ok()
        .filter(|v| !v.is_empty());
    let secret = std::env::var("AWS_SECRET_ACCESS_KEY")
        .ok()
        .filter(|v| !v.is_empty());
    if let (Some(access), Some(secret)) = (key, secret) {
        let token = std::env::var("AWS_SESSION_TOKEN")
            .ok()
            .filter(|v| !v.is_empty());
        return Some((access, secret, token));
    }
    if let Some(profile) = std::env::var("AWS_PROFILE").ok().filter(|v| !v.is_empty()) {
        return bedrock_profile_credentials(&profile);
    }
    None
}

/// Static-key lookup in the shared config files (the offline half of pi's
/// SDK chain: `~/.aws/credentials` `[profile]` + `aws_session_token`).
#[cfg(feature = "real")]
fn bedrock_profile_credentials(profile: &str) -> Option<(String, String, Option<String>)> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    for file in ["credentials", "config"] {
        let path = std::path::Path::new(&home).join(".aws").join(file);
        let text = std::fs::read_to_string(path).ok()?;
        // Config file prefixes sections with `profile ` (credentials doesn't).
        let section = if file == "config" && profile != "default" {
            format!("[profile {profile}]")
        } else {
            format!("[{profile}]")
        };
        let mut in_section = false;
        let (mut key, mut secret, mut token) = (None, None, None);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_section = line == section;
                continue;
            }
            if !in_section {
                continue;
            }
            let (k, v) = line.split_once('=')?;
            match k.trim() {
                "aws_access_key_id" => key = Some(v.trim().to_string()),
                "aws_secret_access_key" => secret = Some(v.trim().to_string()),
                "aws_session_token" => token = Some(v.trim().to_string()),
                _ => {}
            }
        }
        if let (Some(access), Some(secret)) = (key, secret) {
            return Some((access, secret, token.filter(|t| !t.is_empty())));
        }
    }
    None
}

/// `YYYYMMDDTHHMMSSZ` UTC stamp without a date crate: days-since-epoch →
/// civil date (Howard Hinnant's algorithm), time from the remainder.
#[cfg(feature = "real")]
fn bedrock_amz_datetime() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, tod) = ((secs / 86_400) as i64, (secs % 86_400) as i64);
    // Civil-from-days (Hinnant): days since 1970-01-01 → y/m/d.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        tod / 3_600,
        (tod % 3_600) / 60,
        tod % 60
    )
}

/// Path component of a URL (`https://host/model/x/converse-stream` →
/// `/model/x/converse-stream`) — SigV4 signs the path, not the authority.
#[cfg(feature = "real")]
fn url_path(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    match after_scheme.find('/') {
        Some(i) => after_scheme[i..].to_string(),
        None => "/".to_string(),
    }
}

/// Region for signing a Bedrock call: catalog row's region when the wire
/// carries one, else ARN-embedded, else `AWS_REGION`/`AWS_DEFAULT_REGION`,
/// else the model-id default (`eu.` → eu-central-1, else us-east-1) — pi's
/// precedence minus the SDK default chain.
#[cfg(feature = "real")]
fn bedrock_wire_region(model_id: &str) -> String {
    // Region is per-call, not per-row (the catalog wire is a unit): ARN >
    // env > model-id default (pi's precedence minus the SDK default chain).
    // (The `let ...` binding below keeps the match readable under fmt.)
    if let Some(arn_region) = crate::providers::bedrock::BedrockConverseWire::region_from_arn(
        model_id
            .split_once('/')
            .map(|(_, rest)| rest)
            .unwrap_or(model_id),
    ) {
        return arn_region;
    }
    for var in ["AWS_REGION", "AWS_DEFAULT_REGION"] {
        if let Ok(v) = std::env::var(var)
            && !v.trim().is_empty()
        {
            return v.trim().to_string();
        }
    }
    crate::providers::bedrock::BedrockConverseWire::default_region_for(model_id).to_string()
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

/// Attempt one direct-HTTP stream for `model_id` with an explicit key.
///
/// Returns the live event stream on success, or a classified failure detail
/// when the attempt failed BEFORE streaming started (bad model, auth, HTTP
/// error). This is the seam [`crate::fallback::FallbackProvider`] uses to
/// decide whether to try the next candidate in its chain.
///
/// M51: OAuth refresh-before-use (pi `resolveStoredOAuth` composition).
/// Reads the stored refresh token (lock-free), exchanges it OFF the auth
/// lock via [`crate::oauth::exchange_refresh_token`], then persists the
/// rotated pair through [`crate::auth::store_refreshed_oauth_in`] (locked
/// re-check: a concurrent rotation wins). Every failure is swallowed — the
/// caller's key resolution falls back to the stored/env key path, and a
/// telling error surfaces there, never a stale token on the wire.
#[cfg(feature = "real")]
async fn refresh_oauth_before_use(dir: &std::path::Path, provider: &str) {
    let endpoint = match crate::oauth::refresh_endpoint(provider) {
        Some(e) => e,
        None => return,
    };
    let refresh_token = match crate::auth::load_credentials_in(dir).get(provider) {
        Some(crate::auth::StoredCredential::Oauth { refresh, .. }) => refresh.clone(),
        _ => return,
    };
    let Ok(client_id) = std::env::var(endpoint.client_id_env) else {
        return;
    };
    let Ok(mut fresh) =
        crate::oauth::exchange_refresh_token(provider, &refresh_token, &client_id).await
    else {
        return;
    };
    if fresh.refresh.trim().is_empty() {
        fresh.refresh = refresh_token;
    }
    let _ = crate::auth::store_refreshed_oauth_in(dir, provider, &fresh);
}

/// M49: like [`DirectProvider::stream`], Bedrock with an empty key signs
/// SigV4 from the ambient chain instead of failing.
#[cfg(feature = "real")]
pub async fn try_stream_direct(
    model_id: &str,
    key: &str,
    ctx: crate::stream_fn::LlmContext,
) -> Result<BoxEventStream, String> {
    let customs = crate::custom::custom_providers();
    let wire = wire_for(model_id, &customs)?;
    let provider = crate::catalog::lookup_model(model_id)
        .map(|m| m.provider)
        .unwrap_or("");
    let body = wire.request_body(model_id, &ctx)?;
    let url = wire.endpoint_for(model_id, key);
    let mut req = http_client()
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream");
    if provider == "amazon-bedrock" && key.is_empty() {
        let Some((access, secret, token)) = bedrock_chain_credentials() else {
            return Err(
                "no AWS credentials for amazon-bedrock — set AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY, AWS_BEARER_TOKEN_BEDROCK, or run `epsilon auth login amazon-bedrock` (pi bedrockAuth resolve order)".to_string(),
            );
        };
        let datetime = bedrock_amz_datetime();
        let path = url_path(&url);
        let region = bedrock_wire_region(model_id);
        let body_str = body.to_string();
        let sign_input = crate::auth::BedrockSignInput {
            access_key: &access,
            secret_key: &secret,
            session_token: token.as_deref(),
            service: "bedrock-runtime",
            region: &region,
            endpoint: &url,
            path: &path,
            body: &body_str,
            datetime: &datetime,
        };
        for (name, value) in crate::auth::sign_bedrock_request(&sign_input) {
            req = req.header(name, value);
        }
    } else {
        for (name, value) in wire.headers(key) {
            req = req.header(name, value);
        }
    }
    let response = req
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let head = response.text().await.unwrap_or_default();
        let clipped: String = head.chars().take(400).collect();
        return Err(format!("HTTP {status}: {clipped}"));
    }

    let st = HttpEventStream::new(model_id, "", wire.finalize_usage_from_tail());
    Ok(Box::new(WireStream {
        wire,
        inner: Box::pin(futures_util::StreamExt::chain(
            response.bytes_stream().map(Some),
            futures_util::stream::once(std::future::ready(None)),
        )),
        decoder: SseDecoder::new(),
        started: false,
        line_buf: Vec::new(),
        st,
        queued: std::collections::VecDeque::new(),
        finished: false,
        idle_limit: stream_idle_limit(),
    }))
}

#[async_trait::async_trait]
impl StreamFn for DirectProvider {
    async fn stream(&self, model: &ModelRef, ctx: LlmContext) -> BoxEventStream {
        let customs = crate::custom::custom_providers();
        let wire = match wire_for(&model.id, &customs) {
            Ok(w) => w,
            Err(detail) => {
                return Box::new(OnceStream {
                    ev: Some(StreamEvent::Done {
                        message: crate::stream_fn::error_message(detail),
                    }),
                });
            }
        };
        // M22 fix: stored credentials (epsilon auth login) win, env var falls
        // back — same precedence as credentials_ready/live_models. Without this
        // the login flow set up everything EXCEPT actual chat requests.
        // M28: custom ids (`lmstudio/foo`) resolve keys under the custom
        // provider's own name (stored `auth.json` entry for `lmstudio`, then
        // its declared `env_key`).
        let customs_ref = &customs;
        let provider = crate::custom::lookup_custom(customs_ref, &model.id)
            .map(|c| c.name.as_str())
            .or_else(|| crate::catalog::lookup_model(&model.id).map(|m| m.provider))
            .unwrap_or("");
        // M51: OAuth refresh-before-use (pi `resolveStoredOAuth`). When the
        // stored pair expires soon, exchange off-lock and persist the rotated
        // pair so the key below is fresh. Failure falls through to the normal
        // path (stored/env key) — never a stale token, never a hard fail.
        if let Some(dir) = crate::auth::global_dir()
            && crate::auth::oauth_needs_refresh_in(
                &dir,
                provider,
                crate::oauth::OAUTH_MIN_VALIDITY_MS,
            )
        {
            refresh_oauth_before_use(&dir, provider).await;
        }
        let key = match crate::catalog::resolve_api_key(provider) {
            Some(k) => k,
            // M49: Bedrock resolves NO bearer token unless one is stored —
            // the SigV4 chain (env keys, profile, ECS…) carries no secret to
            // resolve, so "no key" means "sign", not "fail". Every other
            // provider still dies here with the login hint.
            None if provider == "amazon-bedrock" => String::new(),
            None => {
                let detail = format!(
                    "no API key for '{}' (model '{}') - run `epsilon auth login {provider}` or set {}",
                    provider,
                    model.id,
                    wire.env_key()
                );
                return Box::new(OnceStream {
                    ev: Some(StreamEvent::Done {
                        message: crate::stream_fn::error_message(detail),
                    }),
                });
            }
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

        let url = wire.endpoint_for(&model.id, &key);
        let mut req = http_client()
            .post(&url)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream");
        // M49: Bedrock SigV4. A resolved key means explicit bearer auth (pi
        // `useBearerToken`); an empty key means "sign with the AWS chain".
        // SigV4 needs the body + a clock stamp BEFORE send, so the Bedrock
        // branch signs here (pure `auth::sign_bedrock_request`) instead of
        // going through `wire.headers(&key)` alone.
        if provider == "amazon-bedrock" && key.is_empty() {
            let creds = bedrock_chain_credentials();
            match creds {
                Some((access, secret, token)) => {
                    let datetime = bedrock_amz_datetime();
                    let path = url_path(&url);
                    let region = bedrock_wire_region(&model.id);
                    let body_str = body.to_string();
                    let sign_input = crate::auth::BedrockSignInput {
                        access_key: &access,
                        secret_key: &secret,
                        session_token: token.as_deref(),
                        service: "bedrock-runtime",
                        region: &region,
                        endpoint: &url,
                        path: &path,
                        body: &body_str,
                        datetime: &datetime,
                    };
                    for (name, value) in crate::auth::sign_bedrock_request(&sign_input) {
                        req = req.header(name, value);
                    }
                }
                None => {
                    return Box::new(OnceStream {
                        ev: Some(StreamEvent::Done {
                            message: crate::stream_fn::error_message(
                                "no AWS credentials for amazon-bedrock — set AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY, AWS_BEARER_TOKEN_BEDROCK, or run `epsilon auth login amazon-bedrock` (pi bedrockAuth resolve order)".to_string(),
                            ),
                        }),
                    });
                }
            }
        } else {
            for (name, value) in wire.headers(&key) {
                req = req.header(name, value);
            }
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
            inner: Box::pin(futures_util::StreamExt::chain(
                response.bytes_stream().map(Some),
                futures_util::stream::once(std::future::ready(None)),
            )),
            decoder: SseDecoder::new(),
            started: false,
            line_buf: Vec::new(),
            st,
            queued: VecDeque::new(),
            finished: false,
            idle_limit: stream_idle_limit(),
        })
    }
}

/// Plan 005/006 test support: the same raw-byte line reassembly `WireStream` uses,
/// exposed so fixture tests can split provider bytes at arbitrary (even mid-char)
/// offsets and assert complete lines decode intact.
pub mod test_support {
    /// Byte-accurate SSE line reassembler.
    pub struct LineReassembler {
        buf: Vec<u8>,
    }

    impl LineReassembler {
        /// Build an empty reassembler (test-support convenience).
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            Self { buf: Vec::new() }
        }

        /// Feed raw chunk bytes; returns the next complete line if one finished.
        pub fn push(&mut self, chunk: &[u8]) -> Option<String> {
            self.buf.extend_from_slice(chunk);
            let pos = self.buf.iter().position(|b| *b == b'\n')?;
            let line_bytes: Vec<u8> = self.buf.drain(..=pos).collect();
            let trimmed: &[u8] = line_bytes
                .strip_suffix(b"\n")
                .unwrap_or(&line_bytes)
                .strip_suffix(b"\r")
                .unwrap_or(line_bytes.strip_suffix(b"\n").unwrap_or(&line_bytes));
            Some(String::from_utf8_lossy(trimmed).into_owned())
        }
    }

    /// Run a raw provider `usage` block through the shared compat decoder and return the
    /// [`Usage`] it produced.
    ///
    /// Exposed because the usage arithmetic is the one part of a provider adapter that no
    /// end-to-end test can reach: it happens mid-stream, on a JSON object, long before
    /// any `AssistantMessage` exists. Every provider that is not Anthropic, Bedrock or
    /// Mistral funnels through this one function, so a test here covers OpenAI, DeepSeek,
    /// Groq, xAI, OpenRouter, Gemini and the Responses API at once.
    ///
    /// M140: the `input` bucket must exclude the cached prefix. See
    /// [`crate::providers::HttpEventStream::seed_usage_compat`] for why.
    pub fn compat_usage_from_json(usage: &serde_json::Value) -> epsilon_contracts::message::Usage {
        let mut st = super::HttpEventStream::new("test-model", "test", false);
        st.seed_usage_compat(usage);
        st.usage_tail
            .expect("a usage block with tokens yields a tail")
    }

    /// Run a raw OpenAI Responses `usage` object through the Responses-specific
    /// reshaping and the compat decoder, returning the resulting [`Usage`].
    ///
    /// M140: the Responses wire carries its cache counts inside `input_tokens_details`,
    /// one level deeper than the compat shape. The reshaping step must forward that
    /// object, or the cached prefix silently disappears — `cache_read` is 0, the header
    /// prints a confident `cache 0%`, and the full prompt is billed at the input rate
    /// with no floor marker to suggest anything is missing.
    pub fn responses_usage_from_json(
        usage: &serde_json::Value,
    ) -> epsilon_contracts::message::Usage {
        let mut st = super::HttpEventStream::new("test-model", "test", false);
        st.seed_usage_compat(&super::openai_responses::responses_usage(usage));
        st.usage_tail
            .expect("a usage block with tokens yields a tail")
    }
}

#[cfg(test)]
mod http_stream_tests {
    use super::*;
    use crate::http::SseEvent;
    use crate::providers::openai_compat::OpenAiCompatWire;

    #[test]
    fn deepseek_text_chunk_produces_done_with_nonempty_content() {
        let wire = OpenAiCompatWire {
            base_url: "https://api.deepseek.com/v1".into(),
            env_key: Some("DEEPSEEK_API_KEY".into()),
        };
        let mut st = HttpEventStream::new("deepseek-chat", "deepseek", false);
        let ev = SseEvent {
            name: None,
            data: r#"{"choices":[{"delta":{"content":"hello"}}]}"#.into(),
        };
        let batch = wire.on_event(&ev, &mut st).expect("text batch");
        assert!(
            batch
                .iter()
                .any(|e| matches!(e, epsilon_contracts::stream::StreamEvent::TextDelta { .. }))
        );
        let done_ev = SseEvent {
            name: None,
            data: "[DONE]".into(),
        };
        let done = wire
            .on_event(&done_ev, &mut st)
            .expect("done batch")
            .pop()
            .unwrap();
        let epsilon_contracts::stream::StreamEvent::Done { message } = done else {
            panic!("expected Done")
        };
        assert_eq!(message.text(), "hello");
    }

    #[test]
    fn deepseek_wire_synthesizes_start_before_text() {
        let wire = OpenAiCompatWire {
            base_url: "https://api.deepseek.com/v1".into(),
            env_key: Some("DEEPSEEK_API_KEY".into()),
        };
        let mut st = HttpEventStream::new("deepseek-chat", "deepseek", false);
        let ev = SseEvent {
            name: None,
            data: r#"{"choices":[{"delta":{"content":"hi"}}]}"#.into(),
        };
        let _batch = wire.on_event(&ev, &mut st).expect("batch");
        assert_eq!(st.text, "hi");
    }

    // ---- M137 / F03: several SSE frames arriving in ONE HTTP chunk ----

    /// Build a `WireStream` over a fixed list of byte chunks, so a test can choose
    /// exactly how the network framed the response. This is the seam F03 needed and
    /// did not have: the field used to be a concrete `reqwest::Response`, so no test
    /// could put more than one SSE frame in a single chunk without a live socket.
    fn wire_over(chunks: Vec<&'static str>) -> WireStream {
        use futures_util::stream;
        let body: Vec<Option<Result<bytes::Bytes, reqwest::Error>>> = chunks
            .into_iter()
            .map(|c| Some(Ok(bytes::Bytes::from_static(c.as_bytes()))))
            .collect();
        WireStream {
            wire: Box::new(OpenAiCompatWire {
                base_url: "https://api.openai.com/v1".into(),
                env_key: Some("OPENAI_API_KEY".into()),
            }),
            // Mirror production exactly: chunks as `Some(..)`, then one `None` for
            // end-of-body, so the stream under test is shaped like the real one.
            inner: Box::pin(futures_util::StreamExt::chain(
                stream::iter(body),
                stream::once(std::future::ready(None)),
            )),
            decoder: SseDecoder::new(),
            started: false,
            line_buf: Vec::new(),
            st: HttpEventStream::new("gpt-x", "openai", false),
            queued: VecDeque::new(),
            finished: false,
            // No watchdog: these tests are about framing, not about silence, and a
            // timeout here could only turn a framing bug into a flaky one.
            idle_limit: std::time::Duration::ZERO,
        }
    }

    fn sse(data: &str) -> String {
        format!("data: {data}\n\n")
    }

    /// The F03 defect. A provider is free to send a whole short response in one TCP
    /// chunk; the decoder must yield *every* frame in it. Before the fix, `ingest`
    /// returned from inside the line loop after the first event-producing line, and
    /// `next()` never re-checked `line_buf` before awaiting the next chunk — so frames
    /// 2..n were never read. Since the body then ended, the stream ended cleanly and the
    /// harness reported a truncated answer as a *success*.
    #[tokio::test]
    async fn every_frame_in_one_chunk_is_delivered_not_just_the_first() {
        let whole = format!(
            "{}{}{}",
            sse(r#"{"choices":[{"delta":{"content":"one "}}]}"#),
            sse(r#"{"choices":[{"delta":{"content":"two "}}]}"#),
            sse(r#"{"choices":[{"delta":{"content":"three"}}]}"#),
        );
        // One chunk. One write. Three frames.
        let mut ws = wire_over(vec![Box::leak(whole.into_boxed_str())]);

        let mut text = String::new();
        let mut saw_done = false;
        // Bounded loop: if the stream never ends, the test fails rather than hangs.
        for _ in 0..32 {
            let Some(ev) = ws.next().await else { break };
            match ev {
                StreamEvent::TextDelta { delta, .. } => text.push_str(&delta),
                StreamEvent::Done { .. } => {
                    saw_done = true;
                    break;
                }
                _ => {}
            }
        }

        assert_eq!(
            text, "one two three",
            "every frame in a single chunk must reach the consumer, in order"
        );
        assert!(saw_done, "and the terminal frame must still arrive");
    }

    /// The same bytes, delivered one frame per chunk, must produce the identical result.
    ///
    /// This is the pair that makes the first test meaningful: if the chunked path and the
    /// single-chunk path disagree, the framing is still wrong somewhere. A fix that only
    /// made the single-chunk case pass would fail here.
    #[tokio::test]
    async fn one_frame_per_chunk_gives_the_same_answer_as_one_big_chunk() {
        let frames = [
            r#"{"choices":[{"delta":{"content":"one "}}]}"#,
            r#"{"choices":[{"delta":{"content":"two "}}]}"#,
            r#"{"choices":[{"delta":{"content":"three"}}]}"#,
        ];
        let boxed: Vec<&'static str> = frames
            .iter()
            .map(|f| {
                let s = sse(f);
                Box::leak(s.into_boxed_str()) as &'static str
            })
            .collect();
        let mut ws = wire_over(boxed);

        let mut text = String::new();
        for _ in 0..32 {
            let Some(ev) = ws.next().await else { break };
            match ev {
                StreamEvent::TextDelta { delta, .. } => text.push_str(&delta),
                StreamEvent::Done { .. } => break,
                _ => {}
            }
        }
        assert_eq!(text, "one two three", "the chunked path must agree");
    }

    /// A frame split *mid-line* across two chunks is the case that already worked
    /// (that is what the `line_buf` reassembly is for) and must keep working after the
    /// loop stopped returning early. Without this, a fix that dropped the reassembly
    /// entirely would pass the two tests above.
    #[tokio::test]
    async fn a_frame_split_mid_line_still_reassembles() {
        let frame = sse(r#"{"choices":[{"delta":{"content":"split me"}}]}"#);
        let (a, b) = frame.split_at(20);
        let a: &'static str = Box::leak(a.to_string().into_boxed_str());
        let b: &'static str = Box::leak(b.to_string().into_boxed_str());
        let mut ws = wire_over(vec![a, b]);

        let mut text = String::new();
        for _ in 0..32 {
            let Some(ev) = ws.next().await else { break };
            match ev {
                StreamEvent::TextDelta { delta, .. } => text.push_str(&delta),
                StreamEvent::Done { .. } => break,
                _ => {}
            }
        }
        assert_eq!(
            text, "split me",
            "a line split across chunks must still decode once complete"
        );
    }
}
