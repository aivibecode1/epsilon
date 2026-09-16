# Plan 005: Wire-fixture test harness for the `real` provider layer

> **Drift check**: `git diff --stat 64e927f..HEAD -- crates/epsilon-llm/ .github/`

## Status

- **Priority**: P1 · **Effort**: L · **Risk**: LOW
- **Depends on**: plan 001 (fixes first, then pin them) · **Category**: tests
- **Planned at**: commit `64e927f`, 2026-08-24

## Why this matters

TEST-01/02: ~1,000 LOC of hand-written wire mapping (the M17 headline) has zero
executing coverage — the SSE decoder's own tests don't even compile in the default
suite because `http.rs` is fully feature-gated, and CI only ever `cargo check`s the
real path. Every chunk-boundary, CRLF, error-frame, and usage-extraction behavior can
regress silently. This harness converts the entire class from "hope" to "table rows".

## Current state

- `crates/epsilon-llm/src/http.rs:9` — file-level `cfg(feature = "real")`.
- `.github/workflows/ci.yml:44` — only `cargo check -p epsilon-cli --features real`.
- Provider adapters have no `#[cfg(test)]` modules at all.

## Scope

**In scope**: moving decoder tests out of the gate (decoder needs no reqwest),
new `crates/epsilon-llm/tests/wire_fixtures.rs` (feature-gated), CI job addition.
**Out of scope**: changing production code semantics (that was plan 001).

## Steps

1. Split `http.rs` so `SseDecoder` + its tests compile unconditionally; keep the
   reqwest-dependent client behind the feature. Verify default suite still green.
2. Build `tests/wire_fixtures.rs`: helper that feeds raw bytes (split at arbitrary
   offsets, incl. mid-line and mid-CRLF) into the ingestion path used by providers,
   collecting `Vec<StreamEvent>`; table of fixtures × expected events:
   - Anthropic: message_start usage seed → text deltas → tool_use input_json_delta
     assembly → message_delta stop_reason+usage → message_stop.
   - OpenAI-compat: indexed tool_calls deltas across chunks; `[DONE]`;
     `stream_options.include_usage` trailing chunk; finish_reason length.
   - Google: combined text+functionCall+usageMetadata frame (plan 001 fix);
     `x-goog-api-key` request shape asserted via captured request builder if
     feasible, else skip.
   - Cross-wire: multi-byte UTF-8 char split across two chunks (documents current
     from_utf8_lossy behavior; see docs/plans/006 for the fix).
3. Add CI lane: `cargo test -p epsilon-llm --features real` on the windows-full job.

## Done criteria

- [ ] Default `cargo test --workspace` compiles+runs decoder tests
- [ ] Feature suite ≥ 15 new fixture cases passing
- [ ] CI runs the lane (yaml diff)

## STOP conditions

- Feeding raw bytes requires a production-type refactor beyond accessor additions.

## Maintenance notes

New wire behaviors should land as fixture rows FIRST (test-first fits this harness).
