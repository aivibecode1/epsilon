# M51 — pi-parity: OAuth refresh consumption (`resolve.ts` + `oauth/*.ts` exchange half)

Nineteenth port batch against the 2026-09-03 parity audit (providers/auth
findings — OAuth refresh). M46 stored the pair, M51-read landed the
OAuth-first resolution branch for `anthropic`, and the two-phase store
primitives (`oauth_needs_refresh_in` + `store_refreshed_oauth_in`,
`OauthRefreshed`/`OauthRefresh` types) were exported with a composition
test — but nothing performed the exchange or called it. `OauthToken::Expired`
was still a dead end: an expired subscription token fell through to the
api-key path and died with "no API key", even though a valid refresh token
sat in the store. Pi's `models.ts:47` contract ("OAuth credentials are
refreshed before network access") now holds.

## What landed

1. **Exchange module** (`epsilon-llm/src/oauth.rs`, new): `REFRESH_ENDPOINTS`
   (anthropic/xai/openai-codex — pi's per-file `TOKEN_URL` verbatim;
   openrouter's `refresh` is the identity and copilot mints via its device
   proxy, so neither gets a row), `parse_refresh_response` (pure, always
   compiled: `{access_token, refresh_token?, expires_in}` + pi's 5-min skew),
   `refresh_request_body` (JSON for Anthropic's `postJson`, form-encoded
   elsewhere), `exchange_refresh_token` (`real`-gated, 30s timeout, telling
   errors, empty-access refused, xai non-rotation backfill).
2. **Stream pre-flight** (`providers/mod.rs::refresh_oauth_before_use`):
   when the stored pair expires within `OAUTH_MIN_VALIDITY_MS`, exchange
   off-lock and persist the rotation BEFORE `resolve_api_key` runs — so the
   existing M51-read branch picks up the fresh token with zero changes.
   Every failure path returns silently: missing endpoint, non-oauth entry,
   unset client id, network/parse error, persist refusal — the normal
   stored/env key path runs and surfaces its own telling error. A stale
   token is never sent; a failed refresh never hard-fails the call.
3. **Tests**: 4 parser/body tests (skew math, missing-field errors,
   rotation-marker, per-provider encoding + non-row exclusion) + the
   existing two-phase composition test. Suite: 51 lib + 79 `real`, clippy
   zero both ways.

## Key decisions

- Client ids from env, NEVER pi's (`*_OAUTH_CLIENT_ID`): pi embeds its own
  per file (one base64-obfuscated). Copying another project's credential
  into our tree would be a supply-chain violation; the exchange errors
  tellingly when the var is unset, naming the exact var.
- Failure swallows at the pre-flight, surfaces at resolution: pi throws
  `ModelsError("oauth"/"auth")` with distinct kinds; our StreamFn
  never-fails contract means the error must surface as the existing
  login-hint `Done` event, not a new failure shape. Swallow-then-fall-
  through preserves that with no envelope change.
- No login ceremonies (unchanged from M46): PKCE callback servers and
  device-code polling still wait for `epsilon auth login --oauth`. The
  store accepts what flows will produce; refresh consumes what the store
  holds.
