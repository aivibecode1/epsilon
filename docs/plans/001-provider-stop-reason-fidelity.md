# Plan 001: Provider stop-reason fidelity (OpenAI finish_reason, Anthropic message_delta, Gemini early-return)

> **Executor instructions**: Follow steps in order; run every verification command.
> On STOP conditions, report back instead of improvising. Update your row in
> `docs/plans/README.md` when done.
>
> **Drift check**: `git diff --stat 64e927f..HEAD -- crates/epsilon-llm/`
> If provider files changed since `64e927f`, compare excerpts below against live code;
> mismatch = STOP.

## Status

- **Priority**: P1 · **Effort**: M · **Risk**: LOW
- **Depends on**: none · **Category**: bug
- **Planned at**: commit `64e927f`, 2026-08-24

## Why this matters

Three HIGH-confidence wire-adapter bugs make real-provider runs misreport stop
reasons or drop tool calls. Worst case: an OpenAI-family model truncated by the
token limit reports `EndTurn` instead of `MaxTokens`, so the loop's
fail-all-tool-calls-on-truncation rule (`crates/epsilon-core/src/exec.rs`) never
fires and corrupted JSON arguments get executed. These are the harness's most
safety-critical invariants, broken silently on the three shipped wires.

## Current state

- `crates/epsilon-llm/src/providers/openai_compat.rs:168` — `[DONE]` arm:
  `let (stop, err) = self.map_stop(None, has_calls);` passes `None` even though
  line ~228 records `finish_reason` via `HttpEventStream::note_finish_reason`.
- `crates/epsilon-llm/src/providers/mod.rs:237` — `note_finish_reason` stores the
  string; **no code ever reads it back**.
- `crates/epsilon-llm/src/providers/anthropic.rs:181-190` — `message_delta` handled
  for usage only; its `delta.stop_reason` is ignored. `message_stop` hardcodes
  `st.finish(StopReason::EndTurn, None)`.
- `crates/epsilon-llm/src/providers/google.rs:140-145` — inside the parts loop, the
  first `text` part does `return Some(vec![...])`, skipping any later parts
  (functionCall / usageMetadata / finishReason) in the same chunk.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Feature tests | `cargo test -p epsilon-llm --features real` | all pass |
| Full gates | `cargo fmt --all -- --check && cargo clippy --workspace --all-targets && cargo test --workspace` | clean / 0 warnings / all pass |

## Scope

**In scope**: the three provider files, `providers/mod.rs` (only to add a getter for
`finish_reason` if needed), new fixture tests in the same files' `#[cfg(test)]`
modules (feature-gated `#[cfg(all(test, feature = "real"))]`).
**Out of scope**: epsilon-core, retry logic, catalog, any behavior not listed here.

## Steps

1. In `mod.rs`, add `pub(crate) fn finish_reason(&self) -> Option<&str>` returning
   `self.finish_reason.as_deref()`. Verify: `cargo build -p epsilon-llm --features real`.
2. openai_compat `[DONE]`: call `self.map_stop(st.finish_reason(), has_calls)` where
   `finish_reason()` reads the stored value; extend `map_stop` so `"length"` maps to
   `StopReason::MaxTokens`, `"content_filter"` to `StopReason::Error` (error_message
   "stopped by content filter"), other/None keep current behavior.
3. anthropic: capture `delta.stop_reason` from `message_delta` via
   `note_finish_reason`; at `message_stop`, map through the same vocabulary
   (`max_tokens` → MaxTokens, `refusal` → Error with message).
4. google: replace the early `return` inside the text-part branch with pushing the
   TextDelta into a local `Vec<StreamEvent>` accumulated across ALL parts; after the
   loop, append functionCall/usage/finish events and return the batch once.
5. Add table-driven unit tests per adapter: raw SSE string → expected final
   StopReason/events. Must include: OpenAI `finish_reason:"length"` with tool_calls →
   MaxTokens; Anthropic `message_delta {stop_reason:"max_tokens"}` then `message_stop`
   → MaxTokens; Gemini chunk containing BOTH text part and functionCall part → both
   TextDelta AND ToolCall events emitted.

## Test plan

New `#[cfg(all(test, feature = "real"))]` modules in each provider file; pattern:
existing decoder tests in `http.rs`. Verification: `cargo test -p epsilon-llm --features real`.

## Done criteria

- [ ] All three wires map truncation to `StopReason::MaxTokens` (tests prove it)
- [ ] Gemini combined text+functionCall chunks emit both events
- [ ] `cargo test --workspace` still green (faux path unchanged)

## STOP conditions

- `map_stop` signature differs from excerpt (drift).
- Making Gemini accumulate changes event ordering that existing tests depend on in a
  way you cannot resolve mechanically.

## Maintenance notes

Reviewers should scrutinize the mapping tables — they are the product. If pi's
adapters (`references/pi-0.84.2/packages/ai/src/api/*`) are updated upstream, re-check
these mappings against it.
