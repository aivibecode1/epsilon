# M44 — pi-parity: OpenAI Responses wire (`openai-responses.ts` family)

Twelfth port batch against the 2026-09-03 parity audit (providers findings —
the Responses-family gap, the largest remaining wire gap). Epsilon routed
all 37 `openai`-provider rows over `/chat/completions`; pi routes that
provider over `/responses` (`openaiProvider` uses `openAIResponsesApi()`).
Chat Completions still serves those models (OpenAI keeps both endpoints),
but tool-id pairing, reasoning items, and usage shapes differ — staying on
chat meant our OpenAI traffic diverged from pi's in ways that surface as
mysterious tool-replay failures.

## What landed

1. **Wire** (`providers/openai_responses.rs`, ~380 lines + tests):
   `OpenAiResponsesWire` implementing `ProviderWire` — `POST {base}/responses`
   with `stream: true, store: false`; request body in the Responses item
   vocabulary (`input_text` / `output_text` / `function_call` /
   `function_call_output`, reasoning summaries); SSE dispatch on the
   `event:` frame field (`response.output_text.delta`,
   `response.function_call_arguments.delta`, `response.output_item.added/done`,
   `response.completed/incomplete/failed`); `call_id|item_id` pairing (pi
   `normalizeToolCallId` shape); final-arguments snapshot wins over delta
   fragments at done time; usage from the terminal `response.usage`;
   `map_responses_stop` (pi `mapStopReason` + ToolUse upgrade on tool
   activity); 16-token `max_output_tokens` floor (pi issue #6265); catalog
   output ceiling like the Anthropic wire.
2. **SSE decoder** (`http.rs`): `SseEvent` gains `name` (the `event:` field);
   chat frames omit it (decoder yields `None` — existing wires untouched),
   Responses frames carry it. Blank-line termination + multi-`data:` joins
   unchanged.
3. **Accumulator** (`providers/mod.rs`): `set_tool_json_at` (done-snapshot
   overwrite) alongside the append-style `feed_tool_json_at`.
4. **Catalog**: `WireApi::OpenAiResponses { base_url, env_key }` +
   `openai_responses()` shorthand; `wire_for` routes it; the **generator**
   (`regen_catalog.py::wire_expr`) assigns it to the `openai` provider so a
   refresh never reverts; all 37 `openai` rows swapped compat → responses
   (section-scoped replace, count verified).
5. **Tests**: 3 wire tests (request vocabulary, full
   text→tool→done→ToolUse stream with usage, incomplete/failed mapping) +
   fixed 3 existing `SseEvent` constructors for the new field.

## Key decisions

- Scope is the mainstream surface only: reasoning-encrypted content,
  custom/grammar tools, deferred tool-search, service-tier pricing, and the
  Azure/Codex variants stay out. Rationale: each needs either a new dep
  (OpenAI SDK types) or provider-specific auth (Azure AD, Codex OAuth) —
  both ADR-level, both tracked as follow-ups, neither needed to move the
  37 rows onto the right endpoint.
- `store: false` (no server-side response retention): pi defaults to
  non-persisted streams for coding sessions; opting into server storage is
  a privacy decision, not a default.
- Custom `<name>/...` ids still ride chat-completions (M28 unchanged):
  third-party compat servers speak chat, not Responses.

## Gotchas

- `LlmContext` has no `max_output_tokens` — first draft read
  `ctx.max_output_tokens` (invented field, E0609). The ceiling comes from
  the catalog row like the Anthropic wire.
- Test helper built `SseEvent { data }` without `name` (E0063 ×3 in mod.rs
  tests) — fixed with `name: None`.
- `message.error_message.unwrap()` behind `&` (E0507) — `as_deref()`.
- `models.dev/api.json` returns 403 from this network — the data-file
  refresh path is unavailable here, so the row swap is a section-scoped
  replace now, generator-owned going forward (both verified: 37 rows).
- The `real`-gated test binary relinked under a fresh hash again
  (`CARGO_INCREMENTAL=0`); the M35/M36/M40 linker note still applies.

## Verification

- `cargo test -p epsilon-llm --features real --lib`: 54 passed (43 old + 8
  new... 11 new: 3 responses + 8 from prior batches counted in the 43→54
  delta — full suite green, exact).
- No-feature lib: 43 passed. `cargo check -p epsilon-cli --features real`:
  clean. fmt clean, workspace clippy zero.
