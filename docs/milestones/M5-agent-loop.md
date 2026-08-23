# M5 — epsilon-core agent loop + epsilon-cli demo

Date: 2026-08-23 · Status: complete

## What was built

| Module | Contents |
|---|---|
| `lib.rs` | `Agent`: pi's two-level loop verbatim — steering inner / follow-up outer, drain points, per-queue `QueueMode`, turn budget; sticky `TurnEndReason`; broadcast event bus; usage accumulation |
| `exec.rs` | Batch executor: sequential preflight → bounded parallel pool (`Semaphore`, default 8) or barrier mode; model-order commit; doom-loop brake (run-scoped streaks, ask at threshold); fail-all-on-truncation; unknown-tool/invalid-arg/error paths |
| `gate.rs` | `PermissionGate` trait + `AutoAllow`/`AutoDeny` policies (real rulesets land in M6) |
| `registry.rs` | Name-keyed tool registry, last-registration-wins (pi parity) |
| `testing.rs` | `EchoTool`/`FailTool` fixtures + `SlowProvider` timing wrapper |
| `epsilon-cli/src/main.rs` | Runnable demo: scripted 3-turn session with parallel calls + a failing tool, printing the live event stream |

Workspace totals: **29 tests** (15 contracts, 12 llm, 7 session, 7 loop scenarios incl.
concurrency-rejection). Demo verified by running.

## Test-caught design bugs (the tests earning their keep)

1. **Doom-loop streaks must be run-scoped, not batch-scoped.** First implementation counted
   consecutive identical calls within one assistant message's batch — but each faux step is
   its own batch, so the brake never fired at threshold 3 across three separate turns.
   opencode counts across the whole run; state moved to `Agent.doom_streaks` (cleared per run).
2. **Concurrency test raced**: a faux provider is so fast the first run finished inside the
   test's 5 ms sleep, so "second run rejected" never triggered. Fixed with `SlowProvider`
   (deterministic delay wrapper) instead of flaky sleeps — same principle as the faux
   provider itself: control time, don't race it.
3. **Demo script bug exposed correct behavior**: a pure-text first step ends the run after one
   turn because no tool calls remain — pi semantics working as specified. The script was
   rewritten to request tools from turn 0.

## Key decisions made while coding

- `Agent::run` returns `RunError::AlreadyRunning` instead of queueing: concurrent runs on one
  agent are caller bugs, surfaced loudly (test enforces it).
- Terminate hints ride in tool-result `details` ("terminate": true) — the message type stays
  pi-compatible without widening the wire format.
- Message updates during streaming are emitted from folded partials; `Done` remains the sole
  authoritative final message (M2/M3 invariant held end-to-end).

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 29 passed
cargo run -p epsilon-cli                # 3-turn scripted session completes
```

## Where this leaves us (per docs/qa/03-partial-vs-full-port.md)

The harness kernel is done: real conversations with stub tools through the complete pipeline,
steering/follow-up queues, forkable sessions underneath, deterministic offline tests, and a
live-event demo. Next milestone (M6) attaches REAL tools (read/write/edit/bash/grep) plus the
permission ruleset engine — that is when Epsilon becomes genuinely usable on actual codebases.
