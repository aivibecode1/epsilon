//! Static model catalog with explicit capability + wire-routing metadata.
//!
//! Port of pi's generated-catalog approach (pi-0.84.2 `packages/ai/src/providers/`):
//! a bundled snapshot of the models.dev registry so browsing needs no network, with
//! each row carrying its wire API (`ModelInfo::wire`) the way pi's models carry
//! `model.api`.
//!
//! The rows themselves live in ``catalog_data`` — **generated** by
//! `scripts/regen_catalog.py` from a live fetch of <https://models.dev/api.json>
//! (DIRECTION-05 closure). Never edit that file by hand: refresh it with
//! `python scripts/regen_catalog.py`, run the gates, fix any moved pins in
//! `catalog_is_current_generation`, and commit script + data together.
//!
//! Binding property since M3: behavior selection reads *capability fields*, never id
//! substrings (anti-pattern from opencode, see comparative-analysis §4). Routing reads
//! `wire`; reasoning-request shaping reads `reasoning_shape` (Wave D); nothing
//! string-matches model families.

mod catalog_data;

/// The date of the models.dev snapshot this catalog was generated from, as
/// `YYYY-MM-DD`.
///
/// **One value, not two.** The same date also appears in the header comment of
/// `catalog_data.rs`, which the generator writes, and in the doc comment of
/// `catalog_is_current_generation`. It had drifted to 2026-08-24 while the data
/// carried 2026-09-21 — a small disagreement that made a reader distrust both files.
/// This constant is the one to read, and ``catalog_snapshot_date_matches_the_data``
/// is the control that fails when the header disagrees with it.
///
/// Surfacing the date is criterion 5's "clearly indicate when the catalog was last
/// successfully updated"; the catalog is a committed file, so this date *is* the last
/// successful update, and it was previously unreadable at runtime.
pub const CATALOG_SNAPSHOT_DATE: &str = "2026-09-21";

/// The catalog's size as `(model rows, distinct providers)`.
///
/// Derived, not stored. The generator already knows both numbers and they appear in a
/// test's doc comment as prose; deriving them here means the `/catalog` output cannot
/// disagree with the data it describes — the same reason [`CATALOG_SNAPSHOT_DATE`] is a
/// value and not a comment.
pub fn snapshot_shape() -> (usize, usize) {
    let providers = MODELS
        .iter()
        .map(|m| m.provider)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    (MODELS.len(), providers)
}

/// Which wire format + endpoint family a model speaks (catalog-driven routing,
/// pi's `model.api` field). Catalog data by design: present in every build, but only
/// `real` builds ever construct a transport from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireApi {
    /// Anthropic Messages (`/v1/messages`, x-api-key).
    AnthropicMessages,
    /// OpenAI-compatible `/chat/completions` (OpenAI, DeepSeek, Groq, xAI, ...).
    OpenAiCompletions {
        /// Provider base URL without trailing slash.
        base_url: &'static str,
        /// Auth env var name (`None`: local Ollama needs no key).
        env_key: Option<&'static str>,
    },
    /// Google Generative Language (`:streamGenerateContent?alt=sse`).
    GoogleGenerativeAi,
    /// OpenAI Responses (`/responses`, SSE `response.*` events). pi routes
    /// this for the `openai`, `openai-codex`, and `azure-openai-responses`
    /// providers; it is a free choice per catalog row here.
    OpenAiResponses {
        /// Provider base URL without trailing slash.
        base_url: &'static str,
        /// Auth env var name.
        env_key: Option<&'static str>,
    },
    /// Mistral Conversations (`/v1/chat/completions`, Bearer auth,
    /// OpenAI-shaped deltas plus Mistral's snake_case payload, 9-char tool
    /// ids, family-routed reasoning, and cached-token usage keys).
    MistralConversations {
        /// Provider base URL without trailing slash.
        base_url: &'static str,
        /// Auth env var name.
        env_key: Option<&'static str>,
    },
    /// GitHub Copilot subscription (`api.individual.githubcopilot.com`, or the
    /// host the credential was minted for — the wire resolves `proxy-ep` from
    /// the key). One host re-serves both dialects, so `anthropic` picks the
    /// inner wire for the row: Claude rows speak Anthropic Messages, every
    /// other family the OpenAI-compatible surface (pi `githubCopilotProvider`).
    GitHubCopilot {
        /// Default API base URL (the individual-host fallback).
        base_url: &'static str,
        /// Auth env var (`COPILOT_GITHUB_TOKEN`); a stored OAuth pair wins.
        env_key: Option<&'static str>,
        /// Whether this row's family rides Anthropic Messages.
        anthropic: bool,
    },
    /// Google Vertex AI (`{location}-aiplatform.googleapis.com`,
    /// `generateContent` body identical to [`WireApi::GoogleGenerativeAi`],
    /// `Authorization: Bearer` from an API key or minted ADC token).
    GoogleVertex,
    /// Amazon Bedrock Converse Stream (`bedrock-runtime.{region}.…`,
    /// Converse JSON over SigV4 or bearer auth, framed JSON events).
    /// Region is the row's `env_key`-adjacent routing hint: the catalog
    /// shorthand pins us-east-1 and the transport re-resolves per call
    /// (ARN > env > model-id default — pi's precedence), so rows stay valid
    /// when the caller overrides region by env/ARN.
    BedrockConverse,
}

const ANTHROPIC_WIRE: WireApi = WireApi::AnthropicMessages;
const GOOGLE_WIRE: WireApi = WireApi::GoogleGenerativeAi;
const GOOGLE_VERTEX_WIRE: WireApi = WireApi::GoogleVertex;
const BEDROCK_WIRE: WireApi = WireApi::BedrockConverse;
const OLLAMA_WIRE: WireApi = openai_compat("http://localhost:11434/v1", None);
const FAUX_WIRE: WireApi = OLLAMA_WIRE;

/// OpenAI-compatible shorthand (pi's "same API, different endpoint/key" providers).
const fn openai_compat(base_url: &'static str, env_key: Option<&'static str>) -> WireApi {
    WireApi::OpenAiCompletions { base_url, env_key }
}

/// OpenAI Responses shorthand (pi's `openai` / `openai-codex` providers).
const fn openai_responses(base_url: &'static str, env_key: Option<&'static str>) -> WireApi {
    WireApi::OpenAiResponses { base_url, env_key }
}

/// GitHub Copilot shorthand (pi's `github-copilot` provider). `anthropic`
/// selects the family's inner wire — the generator emits it per model id.
const fn github_copilot(
    base_url: &'static str,
    env_key: Option<&'static str>,
    anthropic: bool,
) -> WireApi {
    WireApi::GitHubCopilot {
        base_url,
        env_key,
        anthropic,
    }
}

/// Mistral Conversations shorthand (pi's `mistral` provider,
/// `https://api.mistral.ai` + `MISTRAL_API_KEY`).
const fn mistral_conversations(base_url: &'static str, env_key: Option<&'static str>) -> WireApi {
    WireApi::MistralConversations { base_url, env_key }
}

/// Which reasoning-request body shape a model takes (Wave D capability field).
///
/// models.dev does not publish request shapes, so the generator assigns these
/// from a curated, reviewed family table
/// (`scripts/regen_catalog.py::reasoning_shape`) — families only, never
/// per-model invention. The OpenAI-compat and Mistral wires match on this
/// field exhaustively (no wildcard arm), so adding a variant fails to compile
/// until both wires decide what it means on their protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningShape {
    /// Plain `reasoning_effort`. Also the fallback for unlisted rows and for
    /// unknown/custom ids: today's compat default branch, byte for byte.
    /// Wires that never read this field ignore it.
    StandardEffort,
    /// OpenRouter's nested `reasoning: { effort }`.
    OpenRouterReasoning,
    /// DeepSeek reasoning rows: `thinking: { type: "enabled" }` plus effort.
    DeepSeekThinking,
    /// Z.ai GLM rows: `thinking: { type: "enabled", clear_thinking: false }`
    /// plus effort.
    GlmThinking,
    /// Magistral rows (Mistral wire only): `prompt_mode: "reasoning"`. The
    /// compat wire sends no knob for this shape — the Mistral wire owns it.
    MagistralPromptMode,
    /// Mistral small/medium/devstral rows: `reasoning_effort` (the same key
    /// the compat wire uses; the Mistral wire re-emits it through its own
    /// ladder mapping).
    MistralEffort,
    /// No reasoning knob even when thinking is enabled (chat-only rows such
    /// as non-thinking Kimi and Mistral large). Also the Mistral wire's
    /// fallback for unknown ids — today it sends nothing there either.
    None,
}

/// Static description of one model.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Catalog id = the provider-native model name sent on the wire.
    pub id: &'static str,
    /// Provider identifier (picker grouping + auth family).
    pub provider: &'static str,
    /// Human display name for pickers.
    pub label: &'static str,
    /// Context window in tokens.
    pub context_window: u64,
    /// Maximum output tokens.
    pub max_output_tokens: u64,
    /// Input price, micro-USD per million tokens (models.dev list price).
    pub input_cost_micro_per_mtok: u64,
    /// Output price, micro-USD per million tokens (models.dev list price).
    pub output_cost_micro_per_mtok: u64,
    /// Price for a prompt token served **from** the prompt cache, micro-USD per million
    /// tokens. `0` means the registry publishes no separate price for this model, which is
    /// not the same as "free": a caller reporting a cost must say when it could not price
    /// cache traffic rather than quietly counting it as nothing (G18).
    pub cache_read_cost_micro_per_mtok: u64,
    /// Price for a prompt token **written to** the prompt cache. Usually higher than a plain
    /// input token, which is why leaving it out under-reports a session that fills a cache.
    /// `0` means unpublished, as above.
    pub cache_write_cost_micro_per_mtok: u64,
    /// Whether tool/function calling works.
    pub supports_tools: bool,
    /// Env var holding this provider's API key (empty string = none needed).
    pub env_key: &'static str,
    /// Wire format + endpoint family this model speaks (pi's `model.api`).
    pub wire: WireApi,
    /// Reasoning-request body shape for this row (Wave D capability field).
    /// Read by the OpenAI-compat and Mistral wires instead of
    /// string-matching the model id (M3 binding property); every other wire
    /// ignores it.
    pub reasoning_shape: ReasoningShape,
}

use catalog_data::CATALOG_ROWS as MODELS;

/// Bundled catalog slice (picker/browse source; live fetch merges on top).
///
/// Generated from models.dev by `scripts/regen_catalog.py`; includes the offline
/// reference rows (faux) and local ollama rows alongside every current-generation
/// chat/tool model on the nine direct-API providers.
pub fn curated() -> &'static [ModelInfo] {
    MODELS
}

/// Index the catalog by exact id, built once.
///
/// The scan this replaces ran on the TUI's draw path — the header prices a cost
/// from the catalog, and the footer's label comes from the same table, so a
/// frame did two or three linear walks of several hundred rows. The rows are a
/// `&'static` slice that never changes after this is built, so a `OnceLock` is
/// the whole cache: one allocation at first use, none afterwards.
///
/// `or_insert` rather than `collect`, deliberately. The rows are generated data
/// and a duplicate id is a generation bug, but the two implementations must
/// still agree about which row a duplicate resolves to while it is there: the
/// scan returned the **first**, and `HashMap::from_iter` returns the **last**.
/// `by_id_agrees_with_a_linear_scan` pins that.
fn by_id() -> &'static std::collections::HashMap<&'static str, &'static ModelInfo> {
    static INDEX: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static ModelInfo>> =
        std::sync::OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = std::collections::HashMap::with_capacity(MODELS.len());
        for row in MODELS {
            index.entry(row.id).or_insert(row);
        }
        index
    })
}

/// Look up a model by exact catalog id.
pub fn lookup_model(id: &str) -> Option<&'static ModelInfo> {
    by_id().get(id).copied()
}

/// The reasoning-request shape for a request-time model id: the row's
/// capability field when the id is catalogued, `None` otherwise. Each wire
/// substitutes its own default for the miss — compat keeps today's default
/// branch (`StandardEffort`), Mistral keeps today's silence (`None`) — so
/// unknown/custom models behave exactly as they did before this field
/// existed (documented, not silently changed).
pub fn reasoning_shape_for(model_id: &str) -> Option<ReasoningShape> {
    lookup_model(model_id).map(|m| m.reasoning_shape)
}

/// First bundled row for `provider` (P-063: the model a fresh login selects).
///
/// Catalog order, case-insensitive on the provider — the same order the picker
/// lists, so the selection is never a name the UI would not offer. `None` when
/// the provider has no bundled rows (customs, unknown ids): the caller, not
/// this lookup, owns the guidance for that case.
pub fn first_model_for_provider(provider: &str) -> Option<&'static ModelInfo> {
    MODELS
        .iter()
        .find(|m| m.provider.eq_ignore_ascii_case(provider))
}

/// How `resolve_model` found its answer (for diagnostics and picker display).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelMatchKind {
    /// Exact catalog id (`claude-opus-4-6`).
    Exact,
    /// Canonical `provider/id` reference (`anthropic/claude-opus-4-6`;
    /// case-insensitive, unambiguous — pi's canonical match).
    ProviderQualified,
    /// Case-insensitive exact id (pi matches ids case-insensitively).
    CaseInsensitive,
    /// Substring fallback: alias preferred over dated suffixed rows, then
    /// highest-sorting id (pi's alias-over-dated rule).
    Partial,
}

/// A resolved model reference: the catalog row plus how it matched.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedModel {
    /// The catalog row to run.
    pub info: &'static ModelInfo,
    /// How the input matched.
    pub kind: ModelMatchKind,
}

/// Resolve a user-supplied model reference to a catalog row
/// (pi `model-resolver.ts`: exact → canonical `provider/id` → partial).
///
/// Matching rules, in order:
/// 1. exact catalog id;
/// 2. canonical `provider/id` (case-insensitive; ambiguous ⇒ `None`);
/// 3. bare id case-insensitively (ambiguous across providers ⇒ `None`,
///    mirroring pi's rejection of ambiguous bare matches);
/// 4. case-insensitive substring over id + label, preferring alias rows
///    (no `-YYYYMMDD` date suffix) over dated versions, then the
///    highest-sorting id (pi's alias-over-dated rule).
///
/// `None` = no match OR an ambiguous match. Callers that need the reason
/// (picker "did you mean?" vs CLI "unknown model") distinguish via
/// [`resolve_model_candidates`]: empty = no match, multiple = ambiguous.
pub fn resolve_model(reference: &str) -> Option<ResolvedModel> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(info) = MODELS.iter().find(|m| m.id == trimmed) {
        return Some(ResolvedModel {
            info,
            kind: ModelMatchKind::Exact,
        });
    }
    let lowered = trimmed.to_ascii_lowercase();
    // Canonical `provider/id` (+ bare-id fallback) must be UNAMBIGUOUS:
    // collect every row matching either form and accept only a single hit.
    let mut qualified: Vec<&'static ModelInfo> = Vec::new();
    let mut bare: Vec<&'static ModelInfo> = Vec::new();
    for m in MODELS {
        if format!("{}/{}", m.provider, m.id).to_ascii_lowercase() == lowered {
            qualified.push(m);
        } else if m.id.to_ascii_lowercase() == lowered {
            bare.push(m);
        }
    }
    if qualified.len() == 1 {
        return Some(ResolvedModel {
            info: qualified[0],
            kind: ModelMatchKind::ProviderQualified,
        });
    }
    if qualified.is_empty() && bare.len() == 1 {
        return Some(ResolvedModel {
            info: bare[0],
            kind: ModelMatchKind::CaseInsensitive,
        });
    }
    if !qualified.is_empty() || !bare.is_empty() {
        return None; // ambiguous — pi rejects these too
    }
    // Substring fallback over id + label.
    let mut matches: Vec<&'static ModelInfo> = MODELS
        .iter()
        .filter(|m| {
            m.id.to_ascii_lowercase().contains(&lowered)
                || m.label.to_ascii_lowercase().contains(&lowered)
        })
        .collect();
    if matches.is_empty() {
        return None;
    }
    matches.sort_by(|a, b| {
        alias_rank(b.id)
            .cmp(&alias_rank(a.id))
            .then_with(|| b.id.cmp(a.id))
    });
    Some(ResolvedModel {
        info: matches[0],
        kind: ModelMatchKind::Partial,
    })
}

/// All rows matching `reference` under [`resolve_model`]'s rules (exact,
/// canonical, bare, substring) — empty means "no match", several means
/// "ambiguous". Picker diagnostics and CLI errors share this one source.
pub fn resolve_model_candidates(reference: &str) -> Vec<&'static ModelInfo> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Some(exact) = MODELS.iter().find(|m| m.id == trimmed) {
        return vec![exact];
    }
    let lowered = trimmed.to_ascii_lowercase();
    let out: Vec<&'static ModelInfo> = MODELS
        .iter()
        .filter(|m| {
            format!("{}/{}", m.provider, m.id).to_ascii_lowercase() == lowered
                || m.id.to_ascii_lowercase() == lowered
        })
        .collect();
    if !out.is_empty() {
        return out;
    }
    MODELS
        .iter()
        .filter(|m| {
            m.id.to_ascii_lowercase().contains(&lowered)
                || m.label.to_ascii_lowercase().contains(&lowered)
        })
        .collect()
}

/// Alias rows (no trailing `-YYYYMMDD` date suffix, pi's `isAlias`) sort
/// above dated versions; `-latest` suffixed rows count as aliases too.
fn alias_rank(id: &str) -> u8 {
    if id.ends_with("-latest") || !has_date_suffix(id) {
        1
    } else {
        0
    }
}

fn has_date_suffix(id: &str) -> bool {
    id.len() > 9
        && id.as_bytes()[id.len() - 9] == b'-'
        && id[id.len() - 8..].bytes().all(|b| b.is_ascii_digit())
}

// Custom OpenAI-compatible providers (M28; opencode `provider.options.baseURL`,
// Hermes `base_url_override`) live in `crate::custom`. A byte-identical copy of
// `CustomProvider`, `from_config`, `matches_id` and `lookup_custom` used to sit
// here, reachable as `catalog::CustomProvider`, and nothing called it: every
// production caller went through `custom::`. Two spellings of one type is not
// neutral — the one nobody uses is the one that stops being updated, and a
// reader who found it would reasonably be reading a dead branch.

/// Display label for an arbitrary id: catalog label when known, else the id itself
/// (custom entries always render as typed).
pub fn display_label(id: &str) -> &str {
    lookup_model(id).map(|m| m.label).unwrap_or(id)
}

/// Whether the provider credential for `model_id` is present in the environment.
///
/// Models with an empty [`ModelInfo::env_key`] (faux/local) are always ready. Unknown
/// ids conservatively report `false` — the picker shows them under custom entries where
/// the user has already taken responsibility for auth.
///
/// M22: readiness consults [`resolve_api_key`], so a key saved by
/// `epsilon auth login` counts even when no env var is set (stored wins, env
/// fallback — opencode's model).
pub fn credentials_ready(model_id: &str) -> bool {
    match lookup_model(model_id) {
        Some(info) => row_credentials_ready(info),
        None => false,
    }
}

/// [`credentials_ready`] for a row we already hold (no second lookup).
///
/// A row with an empty `env_key` is local (faux, ollama) and needs no credential.
pub fn row_credentials_ready(info: &ModelInfo) -> bool {
    info.env_key.is_empty() || resolve_api_key(info.provider).is_some()
}

/// Provider names that have a usable credential right now (stored or env),
/// in catalog order, deduplicated.
///
/// Used for diagnostics — "you have keys for: deepseek, anthropic" is the line
/// that turns "provider: faux" from a mystery into an instruction. Names only;
/// never a key.
///
/// Resolves per *provider*, not per row: the credential store is a file read, and
/// the catalog has hundreds of rows across a dozen providers.
pub fn credentialed_providers() -> Vec<String> {
    let mut seen: Vec<&'static str> = Vec::new();
    for m in MODELS {
        if m.env_key.is_empty() || seen.contains(&m.provider) {
            continue;
        }
        seen.push(m.provider);
    }
    seen.into_iter()
        .filter(|p| resolve_api_key(p).is_some())
        .map(str::to_string)
        .collect()
}

/// A model the user can actually run right now: the first tool-calling row of the
/// first provider that has credentials, in catalog order. `None` when no provider
/// has a credential.
///
/// This exists to make the "no credentials for provider X" message actionable:
/// naming the missing provider is only half an answer, and the user has no way to
/// know which of their keys maps to a usable row.
pub fn suggested_ready_model() -> Option<&'static ModelInfo> {
    for provider in credentialed_providers() {
        if let Some(row) = MODELS
            .iter()
            .find(|m| m.provider == provider && m.supports_tools)
        {
            return Some(row);
        }
    }
    None
}

/// A credential-aware resolution result.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedModelForRun {
    /// The row to run.
    pub info: &'static ModelInfo,
    /// How the user's input matched.
    pub kind: ModelMatchKind,
    /// The row a plain [`resolve_model`] would have picked, when this one
    /// replaced it because only this one has credentials.
    pub replaced: Option<&'static ModelInfo>,
}

/// Like [`resolve_model`], but when the input is a loose *reference* rather than an
/// exact catalog id and the row it lands on cannot be called for lack of
/// credentials, prefer a sibling row for the same reference that can be.
///
/// The failure this exists for: `--model deepseek-chat` resolves (by substring)
/// to openrouter's `deepseek/deepseek-chat` row. A user with a stored `deepseek`
/// key then got "provider: faux (offline scripted)" and a banner naming no
/// provider — the model they asked for was a shell of its name, not a missing
/// key. Picking the row whose provider the user actually holds is what they
/// meant by typing it.
///
/// Exact ids are never overridden (typing `deepseek/deepseek-chat` is explicit),
/// and an unresolvable reference still returns the original row so callers keep
/// today's error path.
pub fn resolve_model_for_run(reference: &str) -> Option<ResolvedModelForRun> {
    resolve_model_for_run_with(reference, &|info| row_credentials_ready(info))
}

/// [`resolve_model_for_run`] with the readiness test injected, so the selection
/// rule is testable without touching a real credential store or the process
/// environment.
pub fn resolve_model_for_run_with(
    reference: &str,
    ready: &dyn Fn(&ModelInfo) -> bool,
) -> Option<ResolvedModelForRun> {
    let base = resolve_model(reference)?;
    if matches!(base.kind, ModelMatchKind::Exact) || ready(base.info) {
        return Some(ResolvedModelForRun {
            info: base.info,
            kind: base.kind,
            replaced: None,
        });
    }
    let alternative = resolve_model_candidates(reference)
        .into_iter()
        .filter(|c| c.id != base.info.id && ready(c))
        // Tool-calling rows first: this harness streams tool calls, so a
        // credential-ready chat-only row is a worse answer than a tooled one.
        .min_by_key(|c| (!c.supports_tools, c.id));
    match alternative {
        Some(alt) => Some(ResolvedModelForRun {
            info: alt,
            kind: base.kind,
            replaced: Some(base.info),
        }),
        None => Some(ResolvedModelForRun {
            info: base.info,
            kind: base.kind,
            replaced: None,
        }),
    }
}

/// The env var name a custom picker row should instruct the user to set, if known.
pub fn env_key_for(model_id: &str) -> Option<&'static str> {
    lookup_model(model_id)
        .map(|m| m.env_key)
        .filter(|k| !k.is_empty())
}

/// Whether the provider exposes a live `GET /models` listing at all.
///
/// Static knowledge (no network, no feature gate) so UIs can decide whether a
/// live fetch is even worth attempting before injecting a `real`-feature fetcher.
pub fn live_listing_available(provider: &str) -> bool {
    listing_endpoint(provider).is_some()
}

/// Resolve the API key for a provider: stored credential first
/// (`~/.config/epsilon/auth.json`, opencode's auth pattern), then the
/// provider's env var. Returns `None` when neither is present.
///
/// Stored-wins matters because `epsilon auth login` is the documented path;
/// env vars stay as the fallback for CI and users who prefer them. An empty
/// stored key is treated as absent.
pub fn resolve_api_key(provider: &str) -> Option<String> {
    resolve_api_key_in(&crate::auth::global_dir()?, provider, &|name| {
        std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
    })
}

/// The env var name to display when `provider` is authenticated purely from
/// the environment — `Some("ANTHROPIC_API_KEY")`, never the value.
///
/// Stored entries win over env (the ``resolve_api_key_in`` precedence): a
/// provider with a stored credential reads "key", one authenticated purely by
/// its environment variable reads "key (env: NAME)", and one with neither
/// reads "—" via the existing `auth_ready` gate. A blank stored key counts as
/// absent (the trim-then-check contract). An empty `env_key` (local providers)
/// never names a variable. `dir`/`env_lookup` are injected so tests pin the
/// precedence without touching `$HOME` or the process env; `None` dir means no
/// store was readable.
pub fn env_credential_name(
    dir: Option<&std::path::Path>,
    provider: &str,
    env_key: &str,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if env_key.is_empty() {
        return None;
    }
    let stored = dir.is_some_and(
        |d| match crate::auth::load_credentials_in(d).get(provider) {
            Some(crate::auth::StoredCredential::Api { key, .. }) => !key.trim().is_empty(),
            Some(_) => true,
            None => false,
        },
    );
    if stored {
        return None;
    }
    env_lookup(env_key)
        .filter(|v| !v.trim().is_empty())
        .map(|_| env_key.to_string())
}

/// [`env_credential_name`] against the real store directory and process env.
pub fn env_credential_name_live(provider: &str, env_key: &str) -> Option<String> {
    env_credential_name(
        crate::auth::global_dir().as_deref(),
        provider,
        env_key,
        &|name| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned()),
    )
}

/// Whether a stored OAuth credential can never be used for `provider`.
///
/// A stored OAuth pair is consumed only through a refresh endpoint
/// (`providers::refresh_oauth_before_use`, gated on `oauth::refresh_endpoint`)
/// or a login flow — everywhere else the resolver skips it (OAuth never
/// resolves to an API key). A pair stored for a provider with neither is dead
/// configuration that reads as "logged in": the picker warns rather than let
/// it sit. Stored API keys never trigger this (they always resolve or fall
/// back), so the warning cannot fire on a working setup. `dir` is injected
/// for the same reason as above; `None` means no store was readable.
pub fn oauth_credential_unused(dir: Option<&std::path::Path>, provider: &str) -> bool {
    let Some(d) = dir else {
        return false;
    };
    if !matches!(
        crate::auth::load_credentials_in(d).get(provider),
        Some(crate::auth::StoredCredential::Oauth { .. })
    ) {
        return false;
    }
    crate::oauth_login::login_flow(provider).is_none()
        && crate::oauth::refresh_endpoint(provider).is_none()
}

/// The precedence core, with the store directory and env lookup injected so
/// tests can pin stored-wins/env-fallback without touching `$HOME`.
///
/// A stored key is itself a config value (pi `AuthStorage::read`): literals
/// pass through, `$ENV`/`${ENV}` templates resolve against the credential's
/// `env` map then the process env, and `!commands` run cached. Unresolvable
/// values fall through to the provider env var — and a stored command value is
/// never executed here unless it is actually the stored key (pi's
/// `isCommandConfigValue` skip: resolution happens only on read, not on list).
fn resolve_api_key_in(
    dir: &std::path::Path,
    provider: &str,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    // OAuth credentials never resolve to an api key here: the wire takes the
    // access token directly (pi `toAuth`), and its expiry is checked at the
    // call site, not in key resolution.
    //
    // M51: `anthropic` consults the OAuth store BEFORE the api-key path (pi
    // `anthropicApiKeyAuth::resolve`: stored key → `ANTHROPIC_AUTH_TOKEN` →
    // `ANTHROPIC_OAUTH_TOKEN`/api key). A live OAuth access token IS the
    // request key — the wire's `headers` switches on `is_oauth_token` to emit
    // Bearer + Claude Code identity headers. Expired/Missing fall through to
    // the normal path (the caller that needs OAuth freshness surfaces the
    // telling error, not this resolver).
    //
    // M49: `amazon-bedrock` never resolves a BEARER token here either. pi's
    // resolve tries (in order): stored key → `AWS_BEARER_TOKEN_BEDROCK` →
    // profile → access-keys → ECS → web-identity, and the latter five carry
    // NO secret — they select the SigV4 chain, whose material lives in env /
    // files the SDK reads itself. Returning `Some` for them would send the
    // literal string "AWS access keys" as a bearer token. Only a stored key
    // (explicit bearer token) resolves; everything else falls to `None` and
    // the transport signs SigV4 (or errors tellingly when nothing exists).
    //
    // Marker rule: the chain/profile login choices store the literal key
    // `USE_AMBIENT_AWS_CHAIN` — it must NEVER resolve as a bearer token
    // (sending it would be a confusing 403). Any other stored key resolves.
    if provider == "amazon-bedrock" {
        return match crate::auth::load_credentials_in(dir).get(provider) {
            Some(crate::auth::StoredCredential::Api { key, env }) => {
                let k = key.trim();
                if k.is_empty()
                    || k == "USE_AMBIENT_AWS_CHAIN"
                    || crate::config_value::is_command_value(k)
                {
                    None
                } else {
                    crate::config_value::resolve_config_value(k, env.as_ref())
                        .map(|r| r.trim().to_string())
                        .filter(|r| !r.is_empty() && r != "USE_AMBIENT_AWS_CHAIN")
                }
            }
            _ => None,
        };
    }
    if let Some(crate::auth::StoredCredential::Api { key, env }) =
        crate::auth::load_credentials_in(dir).get(provider)
    {
        let k = key.trim();
        if !k.is_empty()
            && !crate::config_value::is_command_value(k)
            && let Some(resolved) = crate::config_value::resolve_config_value(k, env.as_ref())
            && !resolved.trim().is_empty()
        {
            return Some(resolved.trim().to_string());
        }
    }
    // M51 (pi `anthropicApiKeyAuth::resolve`: stored key → ANTHROPIC_AUTH_TOKEN →
    // ANTHROPIC_OAUTH_TOKEN/api key): a LIVE stored OAuth access token is the
    // request key for `anthropic` — the wire's `headers` switches on
    // `is_oauth_token` to emit Bearer + Claude Code identity headers.
    // Expired/Missing fall through (the caller surfaces freshness, not this
    // resolver).
    //
    // M52: `xai` joins it. pi `xaiOAuth.toAuth` returns `{apiKey: access}`,
    // and api.x.ai accepts the SuperGrok/X-Premium access token as the Bearer
    // key on the same OpenAI-compatible wire, so a stored pair logs in exactly
    // like an API key (refresh row already exists in `oauth.rs`).
    // `openrouter` needs no branch: its OAuth exchange mints a permanent API
    // key and stores it through the Api path above.
    // M55: `github-copilot` too — pi `githubCopilotOAuth.toAuth` returns the
    // minted Copilot bearer, and that bearer (not the GitHub token) is what the
    // proxy authenticates. An expired/missing pair falls through to
    // `COPILOT_GITHUB_TOKEN`, which the wire sends as the bearer as-is.
    if matches!(provider, "anthropic" | "xai" | "github-copilot")
        && let crate::auth::OauthToken::Live(access) =
            crate::auth::oauth_access_token_in(dir, provider)
    {
        return Some(access);
    }
    let env_key = lookup_model_first_env(provider)?;
    env_lookup(env_key).filter(|v| !v.is_empty())
}

/// The canonical env var of a provider, taken from any catalog row of that
/// provider (rows share it by construction — see `wire_auth_matches_row_auth`).
fn lookup_model_first_env(provider: &str) -> Option<&'static str> {
    MODELS
        .iter()
        .find(|m| m.provider == provider && !m.env_key.is_empty())
        .map(|m| m.env_key)
}

/// (endpoint URL, auth env var) for a provider's live model listing.
/// Pure static routing data — the network call itself lives in [`live_models`].
#[cfg(feature = "real")]
fn listing_endpoint(provider: &str) -> Option<(&'static str, &'static str)> {
    match provider {
        "anthropic" => Some(("https://api.anthropic.com/v1/models", "ANTHROPIC_API_KEY")),
        "openai" => Some(("https://api.openai.com/v1/models", "OPENAI_API_KEY")),
        "deepseek" => Some(("https://api.deepseek.com/v1/models", "DEEPSEEK_API_KEY")),
        "google" => Some((
            "https://generativelanguage.googleapis.com/v1beta/models",
            "GEMINI_API_KEY",
        )),
        "xai" => Some(("https://api.x.ai/v1/models", "XAI_API_KEY")),
        "groq" => Some(("https://api.groq.com/openai/v1/models", "GROQ_API_KEY")),
        "moonshot" => Some(("https://api.moonshot.ai/v1/models", "MOONSHOT_API_KEY")),
        "zai" => Some(("https://api.z.ai/api/paas/v4/models", "ZAI_API_KEY")),
        "openrouter" => Some(("https://openrouter.ai/api/v1/models", "OPENROUTER_API_KEY")),
        _ => None,
    }
}

#[cfg(not(feature = "real"))]
fn listing_endpoint(_provider: &str) -> Option<(&'static str, &'static str)> {
    None
}

/// List a provider's live model ids via plain authenticated GET `/models`
/// (`real` feature only) — pi/opencode convention for the OpenAI-compatible
/// listing endpoint. Anthropic and Google expose the same shape at their own
/// base URLs. Errors arrive in-band as strings.
#[cfg(feature = "real")]
pub async fn live_models(provider: &str) -> Result<Vec<String>, String> {
    let (url, _env_key) = listing_endpoint(provider)
        .ok_or_else(|| format!("no live listing endpoint for provider '{provider}'"))?;
    // M22: stored credential wins, env var falls back (same precedence as
    // request auth in DirectProvider).
    let key = resolve_api_key(provider).ok_or_else(|| {
        format!(
            "no API key for '{provider}' (run `epsilon auth login {provider}` or set {_env_key})"
        )
    })?;
    let client = crate::http::http_client();
    let mut req = client.get(url);
    match provider {
        "anthropic" => {
            req = req
                .header("x-api-key", &key)
                .header("anthropic-version", "2023-06-01");
        }
        "google" => {
            req = req.header("x-goog-api-key", &key);
        }
        _ => {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("listing failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("bad listing payload: {e}"))?;
    let ids: Vec<String> = body
        .pointer("/data")
        .and_then(|d| d.as_array())
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
        .map(str::to_string)
        .collect();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The index has to answer exactly what the scan it replaced answered.
    ///
    /// An index and a scan disagree in one case: when the same id appears twice,
    /// a scan returns the first row and a map built with `collect` returns the
    /// last. The rows are generated, so a duplicate is a `regen_catalog.py` bug
    /// — but "first" is the answer the product has been giving, and changing it
    /// silently would be the kind of change that only shows up as a wrong price.
    /// `or_insert` is what keeps that true; this is what says so.
    ///
    /// **Compared by value, not by address.** The first version of this test
    /// compared `&ModelInfo` pointers and failed on every row: `CATALOG_ROWS` is
    /// a `const`, and a `const` is materialised separately at each use site, so
    /// the `MODELS` in this test and the `MODELS` inside `by_id` are two equal
    /// copies at two addresses. The assertion was measuring where the compiler
    /// put a constant, which is not a property of the catalog — and it produced a
    /// convincing, entirely false "duplicate id" report.
    ///
    /// The two fields compared are the ones every consumer reads: the label the
    /// picker shows and the window the header divides by. A divergence in either
    /// is the whole failure this test exists to prevent.
    #[test]
    fn by_id_agrees_with_a_linear_scan() {
        for row in MODELS {
            let scanned = MODELS.iter().find(|m| m.id == row.id);
            let indexed = lookup_model(row.id);
            assert_eq!(
                scanned.map(|m| (m.label, m.context_window)),
                indexed.map(|m| (m.label, m.context_window)),
                "id {:?} resolved to a different row than a scan would find",
                row.id
            );
        }
        // And a miss is a miss, rather than a panic or a default row.
        assert!(lookup_model("definitely-not-a-model-id").is_none());
    }

    /// P-063: post-login selection resolves to the provider's first bundled
    /// row, case-insensitively — and to nothing for providers with no rows.
    #[test]
    fn first_model_for_provider_is_the_first_bundled_row_or_nothing() {
        for row in MODELS {
            let first = first_model_for_provider(row.provider).expect("its own provider resolves");
            assert_eq!(
                first.id,
                MODELS
                    .iter()
                    .find(|m| m.provider == row.provider)
                    .unwrap()
                    .id,
                "provider {:?} must resolve to its first bundled row",
                row.provider
            );
        }
        assert!(first_model_for_provider("definitely-not-a-provider").is_none());
        let upper = MODELS[0].provider.to_uppercase();
        assert!(
            first_model_for_provider(&upper).is_some(),
            "resolution is case-insensitive like the rest of the ladder"
        );
    }

    /// The rows themselves must be unique for the index to be the whole story.
    /// If this fails, `by_id_agrees_with_a_linear_scan` above is what is being
    /// protected, and this is the line that says the catalog needs regenerating.
    #[test]
    fn catalog_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        let dupes: Vec<&str> = MODELS
            .iter()
            .filter(|m| !seen.insert(m.id))
            .map(|m| m.id)
            .collect();
        assert!(dupes.is_empty(), "duplicate catalog ids: {dupes:?}");
    }

    #[test]
    fn resolve_api_key_stored_wins_then_env_fallback() {
        // Injected dir + env lookup: no $HOME touched, no env::set_var needed
        // (that's forbidden under unsafe_code=forbid in edition 2024).
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let env = |want: &'static str| {
            move |name: &str| {
                if name == want {
                    Some("env-key".to_string())
                } else {
                    None
                }
            }
        };

        // Neither present -> None.
        let none: Option<String> = None;
        assert_eq!(
            resolve_api_key_in(dir, "anthropic", &|_| none.clone()),
            None,
            "no store, no env"
        );

        // Env only -> env value.
        assert_eq!(
            resolve_api_key_in(dir, "anthropic", &env("ANTHROPIC_API_KEY")),
            Some("env-key".into())
        );

        // Stored beats env.
        crate::auth::save_credential_in(dir, "anthropic", "stored-key").expect("save");
        assert_eq!(
            resolve_api_key_in(dir, "anthropic", &env("ANTHROPIC_API_KEY")),
            Some("stored-key".into()),
            "stored credential must win over the env var"
        );

        // Blank stored key falls back to env (trim-then-check contract). The
        // store refuses blank keys, so simulate a legacy/corrupt file directly.
        std::fs::write(
            crate::auth::auth_path_in(dir),
            serde_json::json!({ "anthropic": { "type": "api", "key": "   " } }).to_string(),
        )
        .expect("write legacy blank key");
        assert_eq!(
            resolve_api_key_in(dir, "anthropic", &env("ANTHROPIC_API_KEY")),
            Some("env-key".into())
        );
    }

    #[test]
    fn env_credential_name_pins_the_display_precedence() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let env = |name: &str| {
            if name == "ANTHROPIC_API_KEY" {
                Some("env-key".to_string())
            } else {
                None
            }
        };

        // Env only, no store -> name the variable, never the value.
        assert_eq!(
            env_credential_name(Some(dir), "anthropic", "ANTHROPIC_API_KEY", &env),
            Some("ANTHROPIC_API_KEY".to_string())
        );

        // Stored beats env -> no suffix (the row reads "key").
        crate::auth::save_credential_in(dir, "anthropic", "stored-key").expect("save");
        assert_eq!(
            env_credential_name(Some(dir), "anthropic", "ANTHROPIC_API_KEY", &env),
            None,
            "a stored credential reads key, not key (env: ...)"
        );

        // Neither -> None.
        assert_eq!(
            env_credential_name(Some(dir), "openai", "OPENAI_API_KEY", &|_| None),
            None
        );

        // Local providers name no variable, even with a same-named value around.
        assert_eq!(env_credential_name(Some(dir), "faux", "", &env), None);

        // A whitespace-only env value is not a credential.
        assert_eq!(
            env_credential_name(Some(dir), "openai", "OPENAI_API_KEY", &|_| Some(
                "   ".to_string()
            )),
            None
        );

        // Blank stored keys count as absent (trim-then-check, same contract as
        // resolution) — so env still gets named rather than hidden.
        std::fs::write(
            crate::auth::auth_path_in(dir),
            serde_json::json!({ "anthropic": { "type": "api", "key": "   " } }).to_string(),
        )
        .expect("write legacy blank key");
        assert_eq!(
            env_credential_name(Some(dir), "anthropic", "ANTHROPIC_API_KEY", &env),
            Some("ANTHROPIC_API_KEY".to_string())
        );

        // No readable store -> env still names itself.
        assert_eq!(
            env_credential_name(None, "anthropic", "ANTHROPIC_API_KEY", &env),
            Some("ANTHROPIC_API_KEY".to_string())
        );
    }

    #[test]
    fn oauth_credential_unused_fires_only_on_dead_logins() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let oauth = |refresh: &str| {
            serde_json::json!({ "type": "oauth", "refresh": refresh, "access": "a", "expires": 0 })
                .to_string()
        };

        // Nothing stored -> silent, for every provider shape.
        for provider in ["mistral", "anthropic", "openai-codex", "faux"] {
            assert!(
                !oauth_credential_unused(Some(dir), provider),
                "no stored credential, no warning: {provider}"
            );
        }

        // Stored API key -> silent everywhere, including providers with flows:
        // an API key always resolves or falls back, so warning there would fire
        // on working setups (anthropic + API key is the common one).
        crate::auth::save_credential_in(dir, "anthropic", "sk-test").expect("save");
        crate::auth::save_credential_in(dir, "mistral", "sk-test").expect("save");
        assert!(!oauth_credential_unused(Some(dir), "anthropic"));
        assert!(!oauth_credential_unused(Some(dir), "mistral"));

        // Stored OAuth where a flow or endpoint exists -> silent (it is used).
        std::fs::write(
            crate::auth::auth_path_in(dir),
            serde_json::json!({
                "anthropic": { "type": "oauth", "refresh": "r", "access": "a", "expires": 0 },
                "openai-codex": serde_json::from_str::<serde_json::Value>(&oauth("r")).unwrap(),
            })
            .to_string(),
        )
        .expect("write oauth pair");
        assert!(!oauth_credential_unused(Some(dir), "anthropic"));
        assert!(!oauth_credential_unused(Some(dir), "openai-codex"));

        // Stored OAuth where neither exists -> warning (nothing will ever read it).
        std::fs::write(
            crate::auth::auth_path_in(dir),
            serde_json::json!({ "mistral": serde_json::from_str::<serde_json::Value>(&oauth("r")).unwrap() }).to_string(),
        )
        .expect("write dead oauth");
        assert!(
            oauth_credential_unused(Some(dir), "mistral"),
            "an OAuth pair for a provider with no flow and no endpoint warns"
        );

        // No readable store -> silent (absence of evidence either way).
        assert!(!oauth_credential_unused(None, "mistral"));
    }

    #[test]
    fn resolve_api_key_honors_key_indirection() {
        // Pi `AuthStorage::read`: the stored key is itself a config value.
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let none = |_: &str| None;

        // `$VAR` template resolves against the credential env map.
        let mut extra = std::collections::BTreeMap::new();
        extra.insert(
            "EPSILON_TEST_S14_VAULT".to_string(),
            "vault-key".to_string(),
        );
        crate::auth::save_credential_full_in(
            dir,
            "anthropic",
            "prefix-$EPSILON_TEST_S14_VAULT",
            Some(extra),
        )
        .expect("save template key");
        assert_eq!(
            resolve_api_key_in(dir, "anthropic", &none),
            Some("prefix-vault-key".into())
        );

        // Unresolvable template falls through to the provider env var.
        crate::auth::save_credential_full_in(
            dir,
            "openai",
            "$EPSILON_TEST_DEFINITELY_MISSING_S14",
            None,
        )
        .expect("save dangling template");
        assert_eq!(
            resolve_api_key_in(dir, "openai", &|name| {
                if name == "OPENAI_API_KEY" {
                    Some("env-fallback".to_string())
                } else {
                    None
                }
            }),
            Some("env-fallback".into())
        );

        // `!command` keys never execute on this path (pi's guard): they fall
        // through instead of spawning a process from a read.
        crate::auth::save_credential_full_in(dir, "google", "!echo must-not-run", None)
            .expect("save command key");
        assert_eq!(resolve_api_key_in(dir, "google", &none), None);
    }

    #[test]
    fn lookup_is_exact_match() {
        assert!(lookup_model("claude-sonnet-4-5").is_some());
        assert!(
            lookup_model("Claude-Sonnet-4-5").is_none(),
            "no case-insensitive guessing"
        );
        assert!(lookup_model("gpt").is_none(), "no substring matching");
    }

    #[test]
    fn resolve_model_follows_pi_match_order() {
        // 1. exact id wins over every other interpretation.
        let exact = resolve_model("claude-sonnet-4-5").expect("exact");
        assert_eq!(exact.kind, ModelMatchKind::Exact);
        assert_eq!(exact.info.id, "claude-sonnet-4-5");

        // 2. canonical provider/id, case-insensitive.
        let qualified = resolve_model("ANTHROPIC/claude-sonnet-4-5").expect("qualified");
        assert_eq!(qualified.kind, ModelMatchKind::ProviderQualified);
        assert_eq!(qualified.info.id, "claude-sonnet-4-5");

        // 3. bare id case-insensitively when unambiguous.
        let folded = resolve_model("Claude-Sonnet-4-5").expect("folded");
        assert_eq!(folded.kind, ModelMatchKind::CaseInsensitive);
        assert_eq!(folded.info.id, "claude-sonnet-4-5");

        // 4. substring fallback prefers aliases over dated rows.
        let partial = resolve_model("sonnet-4-5").expect("partial");
        assert_eq!(partial.kind, ModelMatchKind::Partial);
        assert!(
            !has_date_suffix(partial.info.id),
            "alias beats dated: {}",
            partial.info.id
        );

        // No match, blank, and ambiguous all resolve to None…
        assert!(resolve_model("no-such-model-xyz").is_none());
        assert!(resolve_model("   ").is_none());
        // …but candidates tell the caller WHY.
        assert!(resolve_model_candidates("no-such-model-xyz").is_empty());
        let ambiguous = resolve_model_candidates("claude");
        assert!(
            ambiguous.len() > 1,
            "claude must be ambiguous, got {ambiguous:?}"
        );
        assert!(
            resolve_model("claude").is_some(),
            "partial still picks best"
        );
    }

    #[test]
    fn catalog_groups_and_covers_promised_providers() {
        let mut providers: Vec<&str> = MODELS.iter().map(|m| m.provider).collect();
        providers.sort_unstable();
        providers.dedup();
        for expected in [
            "faux",
            "anthropic",
            "openai",
            "deepseek",
            "google",
            "google-vertex",
            "amazon-bedrock",
            "xai",
            "groq",
            "moonshot",
            "zai",
            "openrouter",
            "ollama",
        ] {
            assert!(
                providers.contains(&expected),
                "provider {expected} missing from bundled catalog"
            );
        }
    }

    #[test]
    fn every_entry_has_label_unique_id_and_wire() {
        let mut ids: Vec<&str> = MODELS.iter().map(|m| m.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "catalog ids must be unique");
        for m in MODELS {
            assert!(!m.label.is_empty());
            assert!(m.context_window > 0);
            let _ = &m.wire; // presence is structural, no Option involved
        }
    }

    /// Every cloud row routes through its provider's own credentials — the wire's
    /// env var must agree with the row's env_key (regression guard for the M17
    /// direct-HTTP port; genai-era OPENROUTER/OPEN_ROUTER mismatch class).
    #[test]
    fn wire_auth_matches_row_auth() {
        for m in MODELS {
            if m.provider == "faux" || m.provider == "ollama" {
                continue;
            }
            if let WireApi::OpenAiCompletions { env_key, .. } = &m.wire {
                assert_eq!(*env_key, Some(m.env_key), "{}", m.id);
            }
        }
    }

    /// Prices come from the models.dev snapshot: local/offline rows are exactly
    /// zero, and every cloud provider carries at least one really-priced row —
    /// proving real registry data flowed in (M3 all-placeholder regression guard).
    /// Individual unpriced rows are allowed through as-is (e.g. google gemma
    /// previews have no cost in the registry); we render what upstream states,
    /// never invented rates.
    #[test]
    fn prices_are_real_registry_rates_or_zero_for_local() {
        let mut paid_providers = std::collections::BTreeSet::new();
        for m in MODELS {
            if m.provider == "faux" || m.provider == "ollama" {
                assert_eq!(m.input_cost_micro_per_mtok, 0, "{}", m.id);
                assert_eq!(m.output_cost_micro_per_mtok, 0, "{}", m.id);
            } else if m.input_cost_micro_per_mtok > 0 || m.output_cost_micro_per_mtok > 0 {
                paid_providers.insert(m.provider);
            }
        }
        for p in [
            "anthropic",
            "openai",
            "deepseek",
            "google",
            "google-vertex",
            "amazon-bedrock",
            "xai",
            "groq",
            "moonshot",
            "zai",
            "openrouter",
            "mistral",
        ] {
            assert!(
                paid_providers.contains(p),
                "provider {p} has no priced row - snapshot looks placeholder"
            );
        }
    }

    /// Current-generation sanity pins: regeneration must not regress to older eras.
    /// Updated whenever `scripts/regen_catalog.py` pulls a newer snapshot
    /// (2026-09-21 snapshot: 383 rows across 9 direct-API providers, plus 7
    /// hand-curated native mistral rows — M47, until the next refresh absorbs
    /// the `mistral` registry).
    ///
    /// The date here must agree with the header of `catalog/catalog_data.rs`,
    /// which the generator writes on every run. It had drifted to 2026-08-24
    /// while the data carried 2026-09-21, which is the kind of small
    /// disagreement that makes a reader distrust both files — the same evidence
    /// discipline the repository asks for everywhere else.
    /// **The date is one value, and this is the control that keeps it one.**
    ///
    /// The generator writes the snapshot date into `catalog_data.rs`'s header comment
    /// and into the doc comment of `catalog_is_current_generation`. Those are prose
    /// and nothing checked them against each other, so they drifted — 2026-08-24 in
    /// one file while the data carried 2026-09-21. Reading the header here turns that
    /// into a test failure instead of a reader's distrust.
    #[test]
    fn catalog_snapshot_date_matches_the_data() {
        let data = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/catalog/catalog_data.rs"
        ))
        .expect("the generated data file is in the tree");
        assert!(
            data.contains(CATALOG_SNAPSHOT_DATE),
            "catalog_data.rs's header must carry the same snapshot date as              CATALOG_SNAPSHOT_DATE ({CATALOG_SNAPSHOT_DATE}) — regenerate, or fix              the constant"
        );
    }

    /// **Criterion 5: the shape the `/catalog` command prints is the data's shape.**
    ///
    /// These numbers were prose in a doc comment ("383 rows across 9 direct-API
    /// providers") and nothing checked the printout against the table. Deriving them
    /// is the same discipline as `CATALOG_SNAPSHOT_DATE`: one source, no second copy.
    #[test]
    fn snapshot_shape_is_derived_not_transcribed() {
        let (rows, providers) = super::snapshot_shape();
        assert_eq!(rows, super::MODELS.len(), "rows come from the table itself");
        assert!(
            providers > 0 && providers < rows,
            "providers partition the rows"
        );
        // A real snapshot, not the empty case the offline build would produce.
        assert!(rows > 100, "the bundled catalog is large: {rows}");
    }

    #[test]
    fn catalog_is_current_generation() {
        // Newest flagship of each direct-API family at regen time:
        assert!(lookup_model("claude-opus-5").is_some());
        assert!(lookup_model("claude-fable-5").is_some());
        assert!(lookup_model("gpt-5.6").is_some());
        assert!(lookup_model("gemini-3.7-flash").is_some());
        assert!(lookup_model("grok-4.6").is_some());
        assert!(lookup_model("kimi-k3").is_some());
        assert!(lookup_model("glm-5.2").is_some());
        assert!(lookup_model("deepseek-v4-pro").is_some());
        assert!(lookup_model("mistral-large-latest").is_some());
        assert!(lookup_model("vertex/gemini-2.5-flash").is_some());
        // M55: bedrock ids are canonical now (region + version qualifiers
        // stripped by the generator, absorbing M49's hand rows).
        assert!(lookup_model("bedrock/anthropic.claude-sonnet-4-5").is_some());
        // Long-gone rows must stay gone...
        assert!(lookup_model("claude-sonnet-4").is_none(), "stale row");
        // ...but anything the registry still lists belongs in the snapshot, even
        // legacy workhorses the old hand-curated table skipped (registry-faithful,
        // not taste-curated):
        assert!(lookup_model("gpt-4o").is_some());
        // Dated snapshots are excluded in favor of canonical aliases:
        assert!(
            lookup_model("claude-haiku-4-5-20251001").is_none(),
            "dated snapshot row leaked into the catalog"
        );
    }

    #[test]
    fn local_models_need_no_credentials() {
        assert!(credentials_ready("faux-1"));
        assert!(credentials_ready("llama3.1:8b"));
        assert!(
            !credentials_ready("claude-sonnet-4-5"),
            "test env has no keys"
        );
        assert_eq!(env_key_for("claude-sonnet-4-5"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(env_key_for("kimi-k2.6"), Some("MOONSHOT_API_KEY"));
        assert_eq!(env_key_for("faux-1"), None);
    }

    #[test]
    fn display_label_falls_back_to_id_for_custom_entries() {
        // Label comes verbatim from the models.dev `name` field. (Pin moved at
        // the M55 refresh: the `deepseek` registry retired the bare
        // `deepseek-chat` row — the alias survives only as openrouter's
        // `deepseek/deepseek-chat` — so the verbatim case reads a row the
        // registry still lists.)
        assert_eq!(display_label("deepseek-v4-pro"), "DeepSeek V4 Pro");
        assert_eq!(display_label("deepseek/deepseek-chat"), "DeepSeek Chat");
        assert_eq!(
            display_label("totally-custom-model"),
            "totally-custom-model"
        );
    }

    /// M63: the credential-aware resolution rule, tested with an injected
    /// readiness predicate so it does not depend on this machine's keys.
    fn deepseek_key_only(info: &ModelInfo) -> bool {
        info.provider == "deepseek"
    }

    #[test]
    fn a_loose_reference_prefers_the_provider_the_user_can_call() {
        // `kimi-k2` substring-matches rows from moonshot, openrouter,
        // amazon-bedrock and github-copilot. A user holding only a Moonshot key
        // must land on a Moonshot row, and the substitution must be reported.
        let resolved =
            resolve_model_for_run_with("kimi-k2", &|i| i.provider == "moonshot").expect("resolves");
        assert_eq!(resolved.info.provider, "moonshot");
        match resolved.replaced {
            Some(replaced) => assert_ne!(
                replaced.provider, "moonshot",
                "a substitution that changes nothing must not be reported"
            ),
            None => assert_eq!(
                resolve_model("kimi-k2").map(|r| r.info.provider),
                Some("moonshot"),
                "no substitution reported means the literal match was already callable"
            ),
        }
    }

    #[test]
    fn an_exact_catalog_id_is_never_overridden() {
        // Typing the full id is an explicit choice, even when a sibling row for
        // the same family would be callable.
        let exact = "deepseek/deepseek-chat";
        assert_eq!(
            resolve_model(exact).map(|r| r.info.id),
            Some(exact),
            "fixture assumption: this id exists verbatim"
        );
        let resolved = resolve_model_for_run_with(exact, &deepseek_key_only).expect("resolves");
        assert_eq!(resolved.info.id, exact, "exact ids win verbatim");
        assert!(resolved.replaced.is_none());
    }

    #[test]
    fn a_reference_with_no_callable_sibling_keeps_its_literal_match() {
        // `deepseek-chat` exists only as OpenRouter rows. A stored DeepSeek key
        // cannot call any of them, and quietly running a different DeepSeek model
        // would be worse than saying so — so the literal match stands and the CLI
        // explains which provider is missing and which keys the user does have.
        let resolved =
            resolve_model_for_run_with("deepseek-chat", &deepseek_key_only).expect("resolves");
        assert_eq!(resolved.info.provider, "openrouter");
        assert!(resolved.replaced.is_none());
    }

    #[test]
    fn nothing_callable_keeps_the_literal_match_so_the_cli_can_explain() {
        let resolved = resolve_model_for_run_with("deepseek-chat", &|_| false).expect("resolves");
        assert_eq!(
            resolved.info.id,
            resolve_model("deepseek-chat").unwrap().info.id
        );
        assert!(resolved.replaced.is_none());
    }

    #[test]
    fn a_ready_literal_match_is_left_alone() {
        let resolved = resolve_model_for_run_with("deepseek-chat", &|_| true).expect("resolves");
        assert!(resolved.replaced.is_none());
    }
    /// The ambient-chain marker must never resolve as a bearer token (M49): sending the
    /// literal string would be a confusing 403, and the transport signs SigV4 instead.
    #[test]
    fn bedrock_ambient_chain_marker_never_resolves_as_a_key() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        crate::auth::save_credential_in(dir, "amazon-bedrock", "USE_AMBIENT_AWS_CHAIN")
            .expect("save");
        assert_eq!(
            resolve_api_key_in(dir, "amazon-bedrock", &|_| None),
            None,
            "the marker selects the ambient chain; it is not a key"
        );
    }

    /// Wave D: every row carries the reasoning-request shape its wire reads,
    /// assigned by the generator's family table (`scripts/regen_catalog.py::
    /// reasoning_shape`). Spot-checks pin each family; the structural guard
    /// below means a refresh cannot silently widen a provider's shape set
    /// (the generator and the data are reviewed together or not at all).
    #[test]
    fn reasoning_shapes_follow_the_family_table() {
        use ReasoningShape as S;
        assert_eq!(
            lookup_model("openai/gpt-5-mini").map(|m| m.reasoning_shape),
            Some(S::OpenRouterReasoning),
            "openrouter rows nest the effort, keyed on provider"
        );
        assert_eq!(
            lookup_model("deepseek-v4-pro").map(|m| m.reasoning_shape),
            Some(S::DeepSeekThinking)
        );
        assert_eq!(
            lookup_model("glm-5.2").map(|m| m.reasoning_shape),
            Some(S::GlmThinking)
        );
        assert_eq!(
            lookup_model("kimi-k2.6").map(|m| m.reasoning_shape),
            Some(S::None),
            "non-thinking kimi sends no knob"
        );
        assert_eq!(
            lookup_model("magistral-medium-latest").map(|m| m.reasoning_shape),
            Some(S::MagistralPromptMode)
        );
        assert_eq!(
            lookup_model("mistral-small-2603").map(|m| m.reasoning_shape),
            Some(S::MistralEffort)
        );
        assert_eq!(
            lookup_model("mistral-large-latest").map(|m| m.reasoning_shape),
            Some(S::None)
        );
        assert_eq!(
            lookup_model("github-copilot/gpt-5.4").map(|m| m.reasoning_shape),
            Some(S::StandardEffort)
        );
        assert_eq!(
            lookup_model("faux-1").map(|m| m.reasoning_shape),
            Some(S::StandardEffort)
        );
        assert_eq!(
            lookup_model("llama3.1:8b").map(|m| m.reasoning_shape),
            Some(S::StandardEffort),
            "local rows ride the compat wire's default branch"
        );
        for m in MODELS {
            match m.provider {
                "openrouter" => assert_eq!(m.reasoning_shape, S::OpenRouterReasoning, "{}", m.id),
                "mistral" => assert!(
                    matches!(
                        m.reasoning_shape,
                        S::MagistralPromptMode | S::MistralEffort | S::None
                    ),
                    "{}",
                    m.id
                ),
                "moonshot" => assert!(
                    matches!(m.reasoning_shape, S::StandardEffort | S::None),
                    "{}",
                    m.id
                ),
                "deepseek" => assert!(
                    matches!(m.reasoning_shape, S::DeepSeekThinking | S::StandardEffort),
                    "{}",
                    m.id
                ),
                "zai" => assert!(
                    matches!(m.reasoning_shape, S::GlmThinking | S::StandardEffort),
                    "{}",
                    m.id
                ),
                _ => assert_eq!(m.reasoning_shape, S::StandardEffort, "{}", m.id),
            }
        }
        assert_eq!(reasoning_shape_for("definitely-not-a-model-id"), None);
    }
}
