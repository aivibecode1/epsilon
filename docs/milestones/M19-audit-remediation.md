# M19 — Audit remediation: executing the 8 improvement plans

Date: 2026-08-24 · Status: shipped in this session. Trigger: the user's instruction
to implement every plan produced by the deep audit (Phase 3a of the alignment
program). Input: `plans/001`–`plans/008` + `plans/README.md`.

## What happened

The 7-auditor deep audit (`reports/re-audit-2026-08-24.md`) found 30+ verified
issues. This milestone executes ALL eight plans, in dependency order, with gates
run after each plan and a full battery at the end.

### Plan 001 — provider stop-reason fidelity (CORRECTNESS-01/02/03)

All three wires discarded or misread the provider's own stop signal:

- **OpenAI-compat**: `[DONE]` called `map_stop(None, has_calls)` while the recorded
  `finish_reason` sat unread. A `length` finish became `EndTurn`, so truncated tool
  arguments could be EXECUTED instead of failing all open calls.
  Fix: getter on `HttpEventStream`, `[DONE]` maps the stored reason.
- **Anthropic**: `message_stop` used a helper that ignored the stop_reason captured
  from `message_delta`. Fix: reuse `map_stop(wire_reason, has_tools)` there.
- **Gemini**: an early-return when `finishReason` was present dropped any
  `functionCall` parts in the same frame. Fix: process parts first; finish flag only
  terminates the loop.

Tests: `crates/epsilon-llm/tests/wire_fixtures.rs` (see plan 005).

### Plan 002 — loop integrity

1. **Overflow latch reset per run** (`Agent::run`): ADR-007's trigger-2 recovery is
   once per RUN; the latch previously stayed set for the whole process, so the
   second context overflow in a long TUI session had no recovery.
2. **Requeue on abnormal exit**: steering messages drained at a checkpoint but not
   yet injected were silently eaten by abort/budget/error exits. New
   `MessageQueue::restore` + `requeue_pending` put them back in order.
   Design note: follow-ups only become `pending` at the outer boundary where an
   empty batch breaks the loop, so requeueing to the steering queue loses nothing.
3. **UTF-8 safety**: byte-offset clipping in `compact.rs` could split a multi-byte
   char (panic); `/search` snippets sliced the ORIGINAL string with offsets from the
   LOWERCASED copy — unsound because lowercasing can change byte length
   (İ → 3 bytes). Both now slice one string on char boundaries.

### Plan 003 — serve-mode trust boundary (SECURITY-01)

Serve mode was an unauthenticated control plane that could start runs AND resolve
its own permission asks. Now:

- `ServeState::with_boot_token` generates a per-boot bearer token (sha256 over
  time+pid+counter); every route except `/healthz` + `/openapi.json` requires
  `authorization: Bearer <token>` compared in constant time. The CLI prints it once.
- Host allow-list (loopback only) blocks DNS-rebinding; non-local `Origin` headers
  are refused (cross-origin POSTs from web pages).
- Unattended print/rpc runs default to `AutoDeny` (deny-with-guidance) instead of
  silently self-approving; `--yes` opts into auto-approve with a loud warning.

Tests: `bearer_token_required_when_configured`, `non_loopback_host_is_rejected`;
existing server tests updated to send loopback Host headers.

### Plan 004 — secret-leak closure (SECURITY-03/07)

1. **Deny beats memory**: `RulesetGate::authorize` evaluates the engine FIRST for
   Deny; a session "always allow" promotion can no longer survive mid-session
   policy tightening.
2. **grep skips secret-class files** (`is_secret_class_file`): `.env*` (except
   `.env.example`), credentials, `.npmrc`, `.netrc`, secrets.yaml/yml, *secret*.
   The read-gate protected `read .env`; grep printed the same lines ungated.
3. **Normalized gate subjects**: `gate_subject` re-relativizes and trims, so
   `.env`, `./.env`, and `src/../.env` produce ONE subject — rule matching and
   session memory keys are consistent.

### Plan 005 — wire fixture harness (TEST-01)

`tests/wire_fixtures.rs`: raw JSON frames through each adapter's `on_event` —
OpenAI finish_reason mapping, Anthropic message_delta→stop, Gemini part
accumulation, plus a split-chunk test. `providers` module made `pub` behind the
existing `real` feature (it already gated everything); adapters/structs widened to
`pub` with docs. CI gained `cargo test -p epsilon-llm --features real` — the real
layer is now TESTED, not just compiled.

### Plan 006 — byte-safe SSE buffer (PERF/CORRECTNESS tail)

`WireStream.line_buf` is `Vec<u8>`; lines decode only when complete, so a 4-byte
emoji split across TCP chunks survives intact. `providers::test_support::
LineReassembler` mirrors the logic for fixture tests.

### Plan 007 — extension event forwarding (DEBT-06)

`ExtHost::send_event(filtered, ev)` returned immediately when `filtered == true`,
and the pump always passed `true` — nothing was ever forwarded. The host now
stores its manifest's `events` list and filters per event type itself;
`send_event(&ev)` has no filter parameter left to misuse.

### Plan 008 — CLI composition cleanup (DEBT-01/04 + DEP/DX tails)

- `RunParams` struct replaces `build_runtime`'s 11 params; the twin
  `workspace_for_roots` argument (always identical to `workspace`) is gone.
- `SharedRun` carries the common mode inputs; the four mode fns take it plus their
  own specifics — all four `too_many_arguments` allows deleted.
- One `Diagnostics` sink replaces the ad-hoc banner vecs in dispatch.
- Unreachable bench arm removed from dispatch (the real arm lives in the mode
  match).
- epsilon-session: crate-wide `#![allow(dead_code)]` replaced by a targeted,
  reasoned allow on `repair` (honestly documented as test-only until wired).
- Manifests: contracts' unused plain `tokio` dep removed (tokio-util covers it).
- AGENTS.md stale "29+ tests" replaced with the no-count convention.

## Verification

```
cargo fmt --all -- --check        → clean
cargo clippy --workspace --all-targets → 0 warnings
cargo test --workspace            → SUITE passed: 220 failed: 0
cargo run -q -p epsilon-cli -- --demo  → demo completes
cargo check -p epsilon-cli --features real → ok
```

## Gotchas for future sessions

- The inline edit-tool linter still reports fake "edition 2015" errors on this
  workspace; trust cargo only (skill lesson #1, confirmed again).
- Making `providers` public pulled a chain of visibility changes (adapters,
  `HttpEventStream::empty_for_tests`, SseEvent re-export) — feature-gated, zero
  cost without `real`.
- Server tests must send `host: 127.0.0.1` now that the guard exists; the
  raw-socket SSE test needed its literal request line updated too.
