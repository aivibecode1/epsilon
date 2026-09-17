# M85 — One-at-a-time follow-up default

Status: code committed as `3afafd3`; closeout complete. The code commit preceded
completion of the full battery, contrary to the intended verification-before-commit
process. This log records the actual closeout; no history was rewritten.

## Change

`LoopLimits::default().followup_mode` now uses `OneAtATime`, matching the
locally inspected pi 0.84.2 constructor (`packages/agent/src/agent.ts:231–232`).
The prior `All` default delivered both pending tasks in one request. Each queued
follow-up now waits until work from the previous one would otherwise stop.
Explicit `QueueMode::All` still batches pending follow-ups in FIFO order.

No queue algorithm, tool scheduler, provider adapter or frontend was changed.
Callers requiring the previous behavior can explicitly select `All`.

## Focused evidence

The integration tests use the actual Agent loop and FauxProvider's recorded
request contexts, not a simulation of queue delivery. The default regression
failed before the fix — expected three requests, received two (exit 101) — and
passed after the single default change (exit 0). Explicit All-mode preservation
passed alongside it (2/2).

- `audit/evidence/queue-conformance-red.log`: failing default regression
  (working-tree provenance).
- `audit/evidence/queue-conformance-green.log`: first green regression.
- `audit/evidence/queue-conformance-modes.log`: both focused scenarios green.

## Isolated candidate gates

The candidate contained `bcc1233` plus only `core/src/lib.rs` and
`tests/queue_conformance.rs`; broad inherited changes were excluded. Committed
files matched it byte-for-byte. All five gates passed: **742 passed, 0 failed,
0 ignored**, demo `Completed`, GATE PASS recorded in
`audit/evidence/gate-queue-candidate.log`. Cleanup removed 16,325 files
(9.4 GiB), C: 60G free, no target directories remain. No CLI/TUI source
changed, so the global binary was not reinstalled.

Independent review passed with no blocking findings and byte-identity
confirmation: `audit/evidence/queue-final-review.json` (and the earlier design
review, `audit/evidence/queue-doc-review.md`).

## Boundaries

This fixes the default, not complete queue parity. Explicit OneAtATime coverage,
steering priority, tool-cycle boundaries, event order, abort behavior and
equivalent executed pi scenarios remain outside these two fixtures. Existing
context-callback and session-correctness gaps remain open (CORE-02/03/04/05,
SESSION-01). The first review approved the test's oracle but predated the
production fix and All test; the final diff review approved the actual change.
