# M47 — pi-parity: Mistral Conversations wire (`mistral-conversations.ts`)

Fifteenth port batch against the 2026-09-03 parity audit (providers
findings — the last named wire family: pi's `mistral` provider). Every other
named family now has its wire (Anthropic Messages, Google, OpenAI
completions, OpenAI Responses); Bedrock/Vertex remain, and they are auth
transports (SigV4/GCP credentials), not new event vocabularies — a different,
larger milestone.

## What landed

1. **New wire** (`epsilon-llm/src/providers/mistral.rs`,
   `MistralConversationsWire`): `POST {base}/v1/chat/completions`, Bearer
   auth, request framing reused from the compat builder with Mistral deltas —
   `prompt_mode: "reasoning"` for magistral rows, `reasoning_effort` for the
   small/medium/devstral reasoning rows (pi's `streamSimple` routing),
   `strict: false` on function tools (pi's fallback for tools without
   sampling metadata).
2. **Five genuine deltas** (everything else IS the compat wire — one
   implementation serves both, pi's shape is OpenAI-shaped by design):
   snake_case payload (emitted directly, no remap step); 9-char tool-call id
   normalization with pi's `shortHash` reimplemented (`wrapping_mul` =
   `Math.imul`, base-36 render) and collision-salted retries; family-routed
   reasoning; cached-token extraction across all eight key shapes pi has
   seen (`prompt_tokens_details.cached_tokens`, camelCase twins,
   `num_{cached_}tokens`), clamped to `[0, prompt]`; and pi's stop
   vocabulary verbatim (`null` → clean stop, unknown → error naming the
   reason — the compat mapper's "unknown means clean end" is wrong here).
3. **Shared accumulator support** (`providers/mod.rs`): `mistral_ids`
   side-table (pi's closure maps, re-expressed) + `seed_usage_mistral`.
4. **Catalog**: `WireApi::MistralConversations` + `mistral_conversations`
   shorthand + `wire_for` routing; 7 hand-curated native rows
   (`mistral-large-latest`, `medium-3.5`, `small-2603`, `devstral-medium`,
   `magistral-medium`, `codestral`, `pixtral-12b`). Prices are La Plateforme
   list rates at snapshot time, flagged as such in the data comment — the
   next `regen_catalog.py` run (which now knows the `mistral` registry +
   wire) absorbs them into generated rows.
5. **Tests**: 6 wire tests (endpoint, reasoning routing per family, id
   normalizer stability, stop vocabulary, cached-token split, array content
   chunks) + catalog pins (`mistral-large-latest` current-gen, `mistral` in
   the priced-provider guard). Suite: 62/62 `--features real`.

## Key decisions

- Reuse, don't fork: the request builder delegates to the compat builder
   and applies deltas after. A separate full implementation would drift;
   pi's own file is 90% OpenAI-shaped for the same reason.
- `x-affinity` session pinning NOT ported: Epsilon's `LlmContext` carries
   no session id (no wire-layer session concept). If sessions ever reach
   the wire, the header slots in beside `Authorization`.
- `onPayload`/`onResponse` hooks NOT ported: no caller uses interception
   here. Same rule as elsewhere — port call sites, not extension points.
- Per-model `thinkingLevelMap` tables NOT ported: the wire maps our ladder
   directly (Low→low, Medium→medium, High+→high; pi's test pins medium→high
   for small-2603, which our mapping satisfies). A table would need catalog
   fields that don't exist yet.
- Image message transforms NOT ported: our context is text-only at the
   wire (M43 images ride tool results, and pi's `toChatMessages` image path
   only fires for rows whose `input` includes image — `pixtral-12b` is the
   only such row here, and it works text-only today).

## Gotchas

- The `epsilon_llm-59b4dd66c3cc2372.exe` linker lock returned (LNK1104 on a
   phantom path — file absent in `ls` yet "permission denied" on create):
   same Windows-incremental staleness as M35. Workaround stands: build +
   run tests under `--target-dir C:/tmp/eps_target` (fresh dir links
   clean, 41s cold). The main `target/` needs a clean at milestone end.
- Test-first assertion caught a real pi-parity bug mid-write: `"null"` and
   `""` diverged (my first draft seeded `"null"` from itself). pi's normalizer
   strips THEN length-checks, so `"null"` (4 chars) falls to the index-seed
   fallback — the test's `c == d` assertion forced the fix before it shipped.
- `mistral_conversations` dead-code warning until rows land: shorthand first,
   rows second, one build — warning resolves itself in the same milestone.

## Verification

- `cargo test -p epsilon-llm --features real --lib` (via `C:/tmp/eps_target`):
   62 passed (6 new), 0 failed.
- `cargo fmt --check` clean; workspace clippy zero (via `C:/tmp/eps_clippy`).
