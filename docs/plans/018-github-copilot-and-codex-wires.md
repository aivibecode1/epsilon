# Plan 018: `github-copilot` wire + device login (and what blocks `openai-codex`)

> **Status**: PART DONE — the `github-copilot` half landed as M55 (wire,
> 27 catalog rows, device login + mint, `RefreshStyle::CopilotMint`, enterprise
> domain on the credential). The `openai-codex` half stays blocked on model
> data — see the section below. Remaining copilot caveat: the per-family wire
> choice (`claude-*` → Anthropic Messages, else completions) is derived from
> the id because models.dev publishes no per-model `api` hint; revisit with a
> live token if a model needs `/responses`.
> **Priority**: P1 (the last two pi provider gaps named in the milestone logs).
> **Depends on**: M52 (device-flow poller + login engine), M44 (Responses wire),
> M47–M49 (the "generator knows the registry" pattern).

## Why

M52 shipped OAuth logins for `anthropic`, `openrouter`, `xai` and deliberately
left `openai-codex` / `github-copilot` out, because a login is only worth
shipping when an Epsilon wire can *spend* the credential. These two are the
last pi provider workstreams with no Epsilon equivalent.

## What the references say

**github-copilot** (pi `providers/github-copilot.ts` + `auth/oauth/github-copilot.ts`):

- Base URL `https://api.individual.githubcopilot.com`, or — better — derived
  from the access token's `proxy-ep=…` claim (`proxy.` → `api.`), with
  `https://copilot-api.<enterprise-domain>` for enterprise domains.
- Models route over **three** wires by family: `claude-*` →
  Anthropic Messages, `gpt-*` → OpenAI Completions/Responses, Gemini-ish
  and others → OpenAI Completions. Epsilon's `WireApi` is per catalog row, so
  the generator picks the wire per model id.
- Every request carries Copilot headers (pi `COPILOT_HEADERS`):
  `User-Agent: GitHubCopilotChat/0.35.0`, `Editor-Version: vscode/1.107.0`,
  `Editor-Plugin-Version: copilot-chat/0.35.0`,
  `Copilot-Integration-Id: vscode-chat`, `X-GitHub-Api-Version: 2026-06-01`.
  Epsilon's wires build headers from the key alone, so this needs a small
  provider-level header hook (an `extra_headers` field on the compat wire, or a
  dedicated `CopilotWire` wrapper).
- Auth: API key via `COPILOT_GITHUB_TOKEN`, or the device flow
  (`github.com/login/device/code` → `login/oauth/access_token`) followed by the
  internal token mint (`api.github.com/copilot_internal/v2/token`) with the
  Copilot headers; the refresh is *not* a refresh-token rotation — it re-mints
  the bearer from the stored GitHub token, so `oauth.rs`'s refresh table needs a
  third style (or a provider-specific branch).
- Model availability is filtered by the token's `availableModelIds`; pi also
  fetches `/models` for the authoritative list.

**Catalog data is available.** models.dev carries a `github-copilot` registry
with **28 models** (context windows, output caps, prices, tiered costs — e.g.
`claude-opus-5` 1M ctx / 64k out, `gpt-5.3-codex`, `kimi-k2.7-code`, `grok-4.5`,
`gemini-3.8-flash`). `scripts/regen_catalog.py` already consumes models.dev, so
the port is: add a `"github-copilot"` entry to its `PROVIDERS` map (env key
`COPILOT_GITHUB_TOKEN`, base URL above), teach `wire_for_provider(...)` the
per-family wire choice, regenerate, and pin the new rows in
`catalog_is_current_generation`.

**openai-codex is blocked on data, not code.** models.dev has **no**
`openai-codex` registry (checked 2026-09-14: 213 providers, `github-copilot`
present, no codex), and `references/pi-0.84.2` does not vendor
`providers/data/openai-codex.json`. pi's own model list is generated from that
file. Implementing the wire blind would mean inventing model ids and prices —
against the repo's "verify against the source" rule. Options when someone wants
it: (a) user provides the model list, (b) the ChatGPT backend's `/models`-like
endpoint is probed with a real token, (c) drop the wire and keep the honest gap.
The wire itself is a known shape: `POST
https://chatgpt.com/backend-api/codex/responses` with `store: false`,
`instructions`, `include: ["reasoning.encrypted_content"]`, `prompt_cache_key`,
plus `chatgpt-account-id` (JWT claim `https://api.openai.com/auth` /
`chatgpt_account_id`), `originator`, `OpenAI-Beta: responses=experimental`,
`session-id` (pi builds it from the cache-session id; SSE transport is
sufficient — the websocket path is an optimisation).

## Steps (copilot) — all landed (M55)

1. ✅ `providers/github_copilot.rs`: `CopilotWire` wrapping the family inner
   wire (no identity headers on chat — pi sends those on the mint and `/models`
   only); `proxy-ep` derivation + enterprise fallback, unit-tested.
2. ✅ `WireApi::GitHubCopilot { base_url, env_key, anthropic }` + the
   `github_copilot(...)` shorthand; `ProviderWire::endpoint_for(model, key)`
   carries the token-derived host.
3. ✅ `oauth_login.rs`: GitHub device-flow row + `MintKind::GitHubCopilot`, the
   `copilot_internal/v2/token` mint, `RefreshStyle::CopilotMint`.
4. ✅ Generator: registry entry, per-id wire mapping, `github-copilot/` prefix,
   `-picker` exclusion + regenerate (`27` rows).
5. ✅ Tests: header shape, `proxy-ep` derivation, login/mint URLs, device-poll
   vocabulary, mint field errors + skew, refresh style contract, flow table.
6. ✅ Docs: `docs/milestones/M55-github-copilot.md`, history row, CHANGELOG,
   ADR amendment, `docs/qa/08`.

## Acceptance

- `epsilon auth login github-copilot --oauth` runs the device flow and stores a
  pair; the catalog lists copilot models; a request to a copilot model carries
  the Copilot headers and the token-derived base URL; refresh re-mints instead
  of rotating.
- Offline gate battery green, `--demo` completes, `target/` wiped after the
  round (see AGENTS.md disk rule).
