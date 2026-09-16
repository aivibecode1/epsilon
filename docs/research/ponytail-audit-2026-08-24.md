# ponytail-audit — Epsilon harness (2026-08-24)

> Historical research, not current product status. See the [archive index](README.md)
> and [verified audit](../../audit/README.md) before relying on completion or safety claims.

Run per `ponytail-audit` skill: whole-repo scan for over-engineering only
(correctness/security/performance explicitly out of scope — those went through the
/improve pass). Method: every `#[allow(dead_code)]`, every `pub` item without a
production caller, duplicated logic across crates, hand-rolled stdlib equivalents.
Findings verified against source before listing. One-shot report; nothing applied.

## Findings (ranked, biggest cut first)

1. `delete:` session `repair.rs` (115 lines) + `lease.rs` (113 lines) — zero
   production callers; repair is integration-test-only (honestly documented in
   plan 008), lease exports types nothing consumes. Replacement: wire both into
   Agent open paths OR delete until a milestone needs them.
   [crates/epsilon-session/src/{repair,lease}.rs]
2. `yagni:` extension command/tool bridge (`ExtHost::invoke`, ext tool wrapper,
   manifest `args`) — the M9a contract ships request/response plumbing no in-repo
   extension or frontend exercises; only event forwarding (plan 007) is live.
   Replacement: keep the seam if external extensions are near-term, else freeze
   behind a feature flag. [crates/epsilon-ext/src/{host,tool}.rs]
3. `shrink:` two glob implementations in-tree — `glob_to_regex` (tools/search.rs)
   and `glob_match` (safety/wildcards.rs) encode the same pattern language twice;
   rules and grep can disagree on edge cases. Replacement: one shared matcher on
   the contracts/util layer. [crates/epsilon-tools/src/search.rs,
   crates/epsilon-safety/src/wildcards.rs]
4. `delete:` SSE decoder branch for Anthropic-style `data.type` frames marked
   "kept for parity" — none of our three wires emits that shape; plan 001 made the
   adapters own frame semantics. [crates/epsilon-llm/src/http.rs]
5. `delete:` test-only dead hooks — `VecStream::single`,
   `JobObject::terminate` — reachable only from the defining crate's tests; move
   under `#[cfg(test)]`. [crates/epsilon-llm/src/retry_stream.rs,
   crates/epsilon-safety/src/sandbox/jobobject.rs]
6. `yagni:` hand-written OpenAPI doc (122 lines) duplicating the router table with
   a test to keep them aligned — fine while the API is six routes; regenerate or
   drop the route if it grows. [crates/epsilon-server/src/openapi.rs]

## Checked and ruled NOT findings

- 732-line static catalog: deliberate offline-first design (M17), real prices.
- `sha2` for the boot token: already-installed dep (ladder rung 5); no new crate.
- 11-crate layout, logic-free contracts, exact pins: ADR-governed architecture.
- `RunParams`/`SharedRun` structs (M19): replaced 11-param signatures; not bloat.

## Net

`net: -350 lines, -0 deps possible.` Biggest lever is #1 (session repair/lease
decision), then the glob unification (#3).

---

## Execution record (follow-up round, same day)

Every finding was re-verified against source before acting. Two verdicts CHANGED:

- **#5 RETRACTED — false positive.** `VecStream::single` has three production
  callers (`retry_stream.rs:122/224/257`) and `ffi::terminate` fires on the
  sandbox-timeout path (`jobobject.rs:269`). Only their stale
  `#[allow(dead_code)]` attributes were misleading — lesson: dead-code markers
  rot faster than the code.
- **#1 split**: repair was cheap to WIRE (better than delete); lease wiring adds
  no value today because each frontend mode uses its own session file, so
  cross-process write contention is not a real scenario.

Applied:

1. **#1 repair WIRED** into the CLI resume path: `--resume <path>` /
   `--continue` now runs `repair_on_open` before building the model view, so a
   crashed prior run's dangling tool calls get synthesized tombstones and the
   resumed context stays provider-valid. The "test-only" allow is gone.
2. **#4 event-field plumbing REMOVED** from the SSE decoder: no wire uses
   `event:` names (all three key off the data JSON). `SseEvent.event`,
   decoder field, parser branch deleted; test rewritten to prove data survives
   `event:` lines. Fixture helper updated.
3. **#6 OpenAPI doc KEPT as-is** after re-check: six routes, drift-guard test,
   regeneration machinery would cost more than the doc. Ruled lean for now.

Deferred (recorded, not forgotten):

- **#3 glob unification** — real duplication but the two dialects are NOT
  equivalent (search globs are segment-aware `**`; permission wildcards
  deliberately match across segments). Unifying needs a semantics decision
  first; filed under future work rather than risk a silent rule-matching change.
- **#2 ext bridge freeze** — architecture seam (ADR-governed); keep until the
  extension story matures.

Net applied now: ~40 lines removed, one feature genuinely wired instead of
deleted, two stale allows retired with the code they masked.
