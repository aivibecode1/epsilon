# M49 — pi-parity: Bedrock Converse Stream wire (`bedrock-converse-stream.ts`)

Seventeenth port batch against the 2026-09-03 parity audit (providers
findings — the LAST named wire family). With this, every provider family pi
ships has its Epsilon wire: Anthropic, Google (API-key + Vertex), OpenAI
completions + Responses, Mistral, Bedrock. Remaining port work is
auth ceremonies (OAuth flows, ADC/JWT mint, ECS/EC2 metadata) and the
protocol-compat decision — no more wire vocabularies.

## What landed

1. **Converse request body** (`providers/bedrock.rs`,
   `bedrock_request_body`): `modelId` + `messages` + `system` block list +
   `inferenceConfig` + `toolConfig` (`toolSpec`/`inputSchema.json`) +
   `additionalModelRequestFields` reasoning hint. Consecutive tool results
   coalesce into ONE user message (Bedrock requires it); empty assistant
   turns skipped; blank text dropped with `<empty>` placeholders where
   Bedrock requires content; tool ids sanitized to `[a-zA-Z0-9_-]` ≤64.
2. **Event mapping** (`bedrock_on_event`): framed JSON events
   (`messageStart` role-checked, `contentBlockStart/Delta/Stop` indexed,
   `messageStop`, `metadata`, five exception shapes → `Done{Error}`).
   Text slots open on first delta (no Start frame — pi parity); reasoning
   text streams as thinking; signatures skipped on replay (Bedrock accepts
   unsigned — documented). Stop vocabulary verbatim (`end_turn`/
   `stop_sequence` → stop, `max_tokens`/`model_context_window_exceeded` →
   length, `tool_use` → toolUse, else error naming the reason).
3. **Binary event-stream framing** (`bedrock_event_payloads`): 12-byte
   prelude unwrapping (pi never sees this — the SDK hides it), with
   newline-JSON fallback for unwrapping proxies/tests. CRCs unchecked
   (documented: corruption surfaces via the stop-reason check).
4. **SigV4** (`auth::sign_bedrock_request` + `BedrockSignInput` struct):
   canonical request → derived key → Authorization, over the workspace
   `sha2` pin (HMAC-SHA256 is three lines — no new `hmac` crate). Plus the
   credential chain (`bedrock_chain_credentials`: explicit keys →
   `AWS_PROFILE` shared-config parse → None), civil-date clock
   (`bedrock_amz_datetime`, Hinnant's algorithm — no date crate), and the
   per-call region precedence (ARN > env > model-id default).
5. **Transport wiring** (`providers/mod.rs`): `DirectProvider::stream` +
   `try_stream_direct` sign when the Bedrock key is empty, send bearer when
   one resolved; "no key" means "sign", not "fail" (Bedrock-only
   exception); telling error when the whole chain is empty. `resolve_api_key`
   returns `Some` ONLY for real stored bearer tokens — the
   `USE_AMBIENT_AWS_CHAIN` marker (profile/chain login) never resolves as a
   token (a literal "AWS access keys" string as bearer would be a confusing
   403; the marker rule is tested implicitly by shape — see gotchas).
6. **Shared-accumulator change**: `tools_closed` is now
   `(index, id, name, buf)` and `finish()` merges Bedrock text slots +
   tool slots in emission order (pi's mixed `blocks` array — "narrate then
   call" interleavings intact). Other wires record `usize::MAX` (close
   order = final order, unchanged behavior — fixtures 10/10 confirm).
7. **Catalog + generator + login**: `WireApi::BedrockConverse` (unit —
   region is per-call, learned the hard way: `String` payloads kill `Copy`
   AND `const` rows), `BEDROCK_WIRE`, 3 `bedrock/*` rows (provider-qualified
   ids like Vertex), generator `amazon-bedrock` registry + wire branch + pi's
   three family exclusions, `auth login amazon-bedrock` three-way select
   (bearer / profile / chain, pi's login shape).
8. **Tests**: 7 wire tests (endpoint, Converse vocab, id sanitization,
   delta→Done flow with 2-block assert, stop vocab, usage split, framing) +
   SigV4 reference test (scope/signed-headers form, determinism, token
   paths) + catalog pins. Suite: 74 lib + 10 fixtures.

## Key decisions

- No AWS SDK (same rule as Vertex's no-`@google/genai`): the SDK does
   signing + framing + credential-chain-network; we own the ~dozen lines
   that matter (sign, frame-split, static-key files) and document the two
   network hops we skip (ECS/EC2 metadata — the error names them).
- No `hmac`/`chrono` crates: three-line HMAC over the pinned `sha2`,
   Hinnant civil-date over `SystemTime`. Dependency policy says no new dep
   without ADR need — neither clears that bar for ~10 lines.
- Region per-call, not per-row: the first draft carried `region: String`
   on the wire enum — killed `Copy` (broken struct-pattern test) AND `const`
   rows (`E0015`). The unit variant + `bedrock_wire_region` precedence is
   simpler AND matches pi (whose region lives in options/env, not the model).
- Prompt-cache `cachePoint` blocks NOT ported (needs catalog cacheability
   fields); thinking-signature replay NOT ported (unsigned accepted);
   proxy/HTTP2 knobs NOT ported (reqwest owns transport); `onPayload`
   hooks NOT ported (no caller). Same deferral posture as M47/M48.
- Prices on the 3 hand rows are list-rate snapshots flagged as such; the
   generator absorbs them on next refresh (same as M47/M48 hand rows).

## Gotchas

- `tools_closed` widening touched `finish()` for ALL wires: the first
   merge draft was written as confused scaffolding (correct approach
   buried in comments, `let _ =` placeholders). Rewrote cleanly as
   (order, seq, block) merge — lesson: when widening shared state, write
   the final shape first, not the reasoning trace.
- `unwrap_or_else(|_| …)` on `Option` is E0593 (takes 0 args) — same class
   as M48's; fixed by script + hand.
- `12 + 0 + len + 4` tripped `no_effect` — `16 + len` is the same number
   and reads as what it is (prelude + crcs).
- Test-helper arg-count lint (9/7): hoisted the identical suffix into
   consts, helper takes 6. The lint counts test fns too — plan for it.
- `BedrockSignInput` struct (not 10 args): the lint forced the better
   design — call sites read as named fields, not positional mystery.

## Verification

- `cargo test -p epsilon-llm --features real --lib`: 74 passed (8 new).
- `--test wire_fixtures`: 10/10 (shared `finish()` unchanged for old wires).
- `cargo test -p epsilon-cli --features real`: 10 + 3 green.
- fmt clean, workspace clippy zero (via `C:/tmp` target dirs — main-target
   linker lock persists; clean at milestone end).
