# M55 — GitHub Copilot: wire, catalog rows, device login

The last pi provider with no Epsilon equivalent that could actually be finished
with verifiable data. Copilot is a subscription proxy rather than a provider:
one host re-serves several model families, and the credential is a short-lived
bearer *minted* from a long-lived GitHub token. Three layers had to exist
before a stored token could be spent, and all three landed here:

**Wire** (`crates/epsilon-llm/src/providers/github_copilot.rs`). `CopilotWire`
wraps the family wire — Anthropic Messages for `claude-*`, OpenAI-compatible
for the rest — and layers Copilot's two peculiarities on top:

- **Token-derived host.** pi `getBaseUrlFromToken` reads `proxy-ep=<host>` out
  of the credential and rewrites the `proxy.` label to `api.`; an enterprise
  domain is the fallback. `endpoint()` cannot see the key, so the
  `ProviderWire` trait gained `endpoint_for(model, key)` (default: delegate to
  `endpoint`) and both request sites now prefer it. Precedence is pi's:
  `proxy-ep` → `copilot-api.<enterprise>` → `api.individual.githubcopilot.com`.
- **Bearer on both dialects.** Claude rows behind the proxy take
  `Authorization: Bearer` + `anthropic-version` — never `x-api-key`, never the
  Claude Code identity headers (that branch is keyed on `sk-ant-oat`, which a
  Copilot bearer never is).
- Paths follow the SDKs pi hands requests to: `/v1/messages` for Claude,
  `/chat/completions` for everything else.

**Catalog** (`scripts/regen_catalog.py` → `catalog_data.rs`). models.dev
carries a `github-copilot` registry — 28 tool-calling models with real context
windows and prices — and the generator now consumes it: a `PROVIDERS` entry,
a per-row wire decision (Claude ids → Anthropic Messages), the
`github-copilot/…` id prefix (without it the first-provider-wins dedupe drops
every copilot row as a "dup of anthropic"), and `*-picker` aliases excluded
against a row that is not a servable id. Result: **27 rows**, 8 of them Claude.
Regeneration was verified delta-clean — diffing a run with and without the
entry shows the copilot block and nothing else.

**Login + refresh.** `LoginFlow` gained two fields and a `MintKind`, because
GitHub's device flow does not end where xAI's does:

- device code + poll on `github.com` (or the enterprise host), with GitHub's
  editor `User-Agent` and `Accept: application/json` — its success body is a
  bare `access_token`, so `parse_device_poll_for` routes copilot flows to
  `parse_github_device_token` and wraps the result as
  `ExchangeOutcome::Device`. The RFC 8628 vocabulary (pending / slow_down /
  denied / expired) is now shared by both shapes via `device_poll_error`
  instead of being copied.
- the mint (`copilot_internal/v2/token`, GET, editor headers,
  `{token, expires_at}`) runs inside the device half before `persist`, so an
  unminted token can never be stored — `persist` refuses `Device` outright.
- **refresh is a third style.** `RefreshStyle::CopilotMint` models what pi
  does: no body, no client id, bearer = the stored GitHub token, response
  `{token, expires_at}` rather than a rotated pair. `refresh_request_body`
  returns `None` for it, so there is one branch point, not a flag soup.
- enterprise accounts: `[oauth.github-copilot] enterprise_domain = "…"` drives
  the ceremony host *and* the mint host, and is stored **with** the credential
  (`StoredCredential::Oauth` gained an optional `env` map, pi's
  `credential.enterpriseUrl`) because the wire and every later refresh need it.
  A rotation carries those extras forward — they describe the account, not the
  pair.

`resolve_api_key` treats `github-copilot` like `anthropic`/`xai`: a live stored
bearer is the request key, `COPILOT_GITHUB_TOKEN` is the fallback.

Client ids are **not** embedded: `GITHUB_COPILOT_OAUTH_CLIENT_ID` or
`[oauth.github-copilot] client_id` (pi base64-decode-hides its own; the policy
in `docs/qa/08-oauth-login-client-ids.md` stands).

## Key decisions

- **Wrap, don't fork.** One `CopilotWire` delegating body/SSE/stop mapping to
  the family wire keeps Copilot's quirks (host + auth) in one file; a second
  copy of the Anthropic parser would have rotted.
- **`endpoint_for` on the trait**, not a special case in the transport: any
  future provider whose host depends on its credential gets it for free, and
  the default keeps every existing wire untouched.
- **`CopilotMint` as a refresh *style*** rather than a `mint: bool` — the
  three shapes are mutually exclusive, and the enum makes an impossible
  combination unrepresentable.
- **The enterprise domain rides the credential**, not the config, once the
  login is done: config can change under a stored account, and the wire must
  keep talking to the host the token was minted for.
- **No identity headers on chat requests.** pi sends `COPILOT_HEADERS` on the
  mint and `/models` calls only; the chat path carries the bearer alone. If a
  live account ever disagrees, the header set is already in the module.

## Drift absorbed at this refresh

The M55 regeneration also absorbed older debt, because a refresh cannot be
partial:

- **Bedrock ids are canonical now.** The registry lists every model region- and
  version-qualified (`us.anthropic.claude-sonnet-4-5-20250929-v1:0` — five
  near-identical rows per model, and the `-v1:0` tail slipped dated snapshots
  past the `-YYYYMMDD$` rule). The generator strips the region label and the
  dated version tail, which both absorbs M49's hand rows and collapses 155 rows
  to 78. Region routing never depended on the row id —
  `BedrockWire::default_region_for` is a hint and env/ARN win — but a user
  pinned to a dated id will now match through `resolve_model`'s partial rule.
- **Pins moved**: `display_label("deepseek-chat")` → the `deepseek` registry
  retired the bare row at this snapshot, and the vendor's own docs confirm it:
  `deepseek-chat`/`deepseek-reasoner` were discontinued 2026-07-24 (they routed
  to V4-Flash's non-thinking/thinking modes) — so the drop is registry-faithful
  *and* vendor-correct, not a lost alias worth hand-adding. The label pin now
  reads rows the registry still lists; a config pinned to the old name resolves
  through `resolve_model`'s partial rule, which names the row it matched. Registry-faithful, not taste-curated — same posture as every refresh
  since M20. The TUI picker tests were built on the same retired ids
  (`deepseek-chat`, `deepseek-reasoner`): they now use `deepseek-v4-pro` /
  `deepseek-flash`'s label, keeping every case they were written to cover
  (fuzzy subsequence, multi-token, label-only match, ranking, known-id dedupe).
  `deepseek-chat` itself is *not* hand-added back — a row nothing verifies the
  prices or window for is exactly what the registry-faithful rule forbids.

## Gotchas

- **A fuzzy patch silently rewrote the generator's id scheme.** The
  prefix logic for `vertex/` and `bedrock/` rows ("provider-qualified ids",
  M48/M49) was a few lines above the block being edited; the patch tool's fuzzy
  match accepted the edit and dropped it, so the first regenerated snapshot had
  no vertex rows at all and bare bedrock ids. The two failing catalog pins
  (`catalog_is_current_generation`: `vertex/gemini-2.5-flash`,
  `bedrock/anthropic.claude-sonnet-4-5`) are what caught it. Lesson: after any
  script edit, regenerate and run the pins BEFORE believing a self-consistent
  delta check — two runs of the same broken script agree with each other.
- `Cargo`'s `head`/`grep`-filtered test runs hide compile errors behind a
  `|` — the first real signal here was a cold build that had `--features real`
  only half-checked.
- The generator's per-provider `wire_expr(prov)` was not enough: copilot needed
  the model id, and the id carries the emitted registry prefix, so the family
  check must strip `github-copilot/` before looking at the name.
- `parse_exchange_response` requires `refresh_token` + `expires_in`; GitHub's
  device response has neither, which is exactly why the copilot path parses
  the device token separately instead of loosening the shared parser.

## Verification

- `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets`
  zero diagnostics.
- `cargo test --workspace`: **416 passed, 0 failed** (410 before; the wire's own
  tests only compile under `real`, which is why the default lane gains six).
- `cargo test -p epsilon-llm --features real`: **125 passed, 0 failed**
  (88 lib incl. the 7 Copilot wire tests + 27 ceremony + 10 fixtures);
  `cargo check -p epsilon-cli --features real` clean.
- `cargo run -q -p epsilon-cli -- --demo` completes.
- Offline tests cover: `proxy-ep` derivation (with/without the `proxy.` label,
  empty), base-URL precedence, both endpoint paths, header shape (bearer-only,
  `anthropic-version` for Claude, never `x-api-key`), delegation of request
  body + stop mapping, login/mint URL derivation (individual + enterprise),
  the GitHub device poll vocabulary, the mint's field errors + skew math, the
  refresh row's style/no-body contract, the config-resolved enterprise domain,
  the login-flow table invariants, and the per-flow poll-parser dispatch
  (`parse_device_poll_for`: pair shape for xAI, GitHub shape for copilot).
- Live-account caveat: no Copilot subscription was used to exercise the
  request path; the port follows pi's shapes and models.dev's numbers, and the
  plan records the assumption to revisit (per-model `/responses` routing).
