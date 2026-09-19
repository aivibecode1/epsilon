# M48 — pi-parity: Google Vertex AI wire (`google-vertex.ts`)

Sixteenth port batch against the 2026-09-03 parity audit (providers
findings — Cloud auth transport). Vertex is pi's second Google path: same
`generateContent` request/response shape as the API-key wire, different
endpoint + auth. Bedrock remains (SigV4/event-stream binary framing — a
larger, standalone milestone); everything else named in the audit now has
its wire.

## What landed

1. **Shared Google body/event/stop mapping** (`providers/google.rs`):
   `google_request_body` (now returns the value directly — the old
   `Result` never errored), `google_on_event`, `google_map_stop`,
   `google_level`. `GoogleWire` delegates; behavior byte-identical
   (lib suite still 66/66, fixtures 10/10).
2. **New Vertex wire** (`providers/vertex.rs`, `GoogleVertexWire`):
   location-scoped publisher endpoint, `Authorization: Bearer` from a
   RESOLVED token, shared body/event/stop. `bearer_token` encodes pi's
   `resolveApiKey` fork (real key passes through verbatim — pi's
   `createClientWithApiKey` sends it as the bearer token; placeholder
   `<...>`, `gcp-vertex-credentials`, or empty means "mint from ADC").
   `adc_jwt_input` parses the service-account file (client_email +
   private_key, cloud-platform scope, oauth2 token URL) — the file shape
   pi's SDK reads, without the SDK dependency.
3. **Catalog**: `WireApi::GoogleVertex` + `GOOGLE_VERTEX_WIRE` + `wire_for`
   routing (project/location from stored credential `env`, ambient env
   fallback, telling error when both miss — pi `resolveProject/
   resolveLocation`); 3 hand-curated rows (`vertex/gemini-2.5-flash`,
   `vertex/gemini-3.5-flash`, `vertex/gemini-3-pro`). Ids are
   provider-qualified: bare `gemini-*` ids already belong to the API-key
   rows and catalog ids must be unique (pi disambiguates by provider
   object; our flat space uses the `vertex/` prefix — `resolve_model`
   still hits `ProviderQualified`, bare ids still hit the API-key rows).
4. **Generator**: `google-vertex` registry entry + `wire_expr` branch +
   gemini-only filter (pi skips the registry's Claude/OpenAI MaaS rows and
   the `3.1-flash-lite-preview`). Next refresh absorbs the hand rows.
5. **Login**: `epsilon auth login google-vertex [key]` prompts for
   key-or-ADC + project + location (env pre-seeds for scripts), stores all
   three in the credential `env` map (pi's `credential.env`), refuses to
   store a dead entry (no key AND no ADC file). Plus `save_credential_full`
   global-dir convenience in `auth.rs`.
6. **Tests**: 4 wire tests (endpoint, placeholder routing, ADC parsing,
   body-shape parity) + catalog pins (`vertex/gemini-2.5-flash`
   current-gen, `google-vertex` in priced + provider-coverage guards).

## Key decisions

- Reuse, don't fork (same rule as M47): the ONLY Vertex code is endpoint
   + auth. pi's own Vertex chunk loop is line-for-line the Gemini walk
   over the same SDK type — a separate implementation would be pure drift
   surface.
- No `@google/genai` SDK, no `gcloud` subprocess: the SDK does endpoint
   templating + OAuth minting; we do the templating directly and parse the
   ADC file for the mint inputs. The actual JWT-sign + token POST stays
   unimplemented (documented seam: `BearerSource::MintFromAdc`) — minting
   needs RS256 (new dep) + a token-cache discipline, which is its own
   milestone with Bedrock's SigV4 (both are "cloud credential → bearer"
   transports, both need the same cache-then-refresh shape).
- No Gemini-3 thinking-budget tables (`getGemini3ThinkingLevel`,
   `getGoogleBudget`): our ladder maps directly; pi's tables tune budgets
   per model id, which needs catalog fields that don't exist yet. Same
   deferral as M47's `thinkingLevelMap`.
- `x-affinity`/session, `onPayload` hooks: same non-port as M47 — no
   caller, no session concept at the wire layer.

## Gotchas

- `wire_fixtures.rs` broke: M44's `SseEvent.name` field (Responses
   `event:` support) landed without updating the integration-test
   constructor. Fixed with `name: None` + fmt reorder. Lesson: struct
   changes need a workspace-wide constructor grep, not just lib tests.
- M47's underscore-grouping warnings (`2654_435_761`) were still open —
   fixed in passing (regrouped to `2_654_435_761`), clippy back to zero.
- `.unwrap_or_else(|_| …)` on `Option` after `.ok().filter(…)` is
   `E0593` — the closure takes 0 args. Three sites fixed (two by script,
   one by hand).

## Verification

- `cargo test -p epsilon-llm --features real --lib`: 66 passed (4 new).
- `cargo test -p epsilon-llm --features real --test wire_fixtures`: 10/10.
- `cargo test -p epsilon-cli --features real`: 10 + 3 green.
- fmt clean, workspace clippy zero (all via `C:/tmp/eps_target` /
   `C:/tmp/eps_clippy` — the main-target linker lock persists; clean at
   milestone end).
