# M86 — Compaction correctness: acceptance, cancellation, projection

## What

Three defects from the port revalidation's core review land together, because they
share one code path and one commit boundary.

1. **CORE-03 / F04 — a later compaction replaces the superseded projection.**
   `SessionStore::model_view` used to push every compaction's summary + tail onto
   the view. After a second compaction a store-backed run sent S1/T1/S2/T2 while
   runtime history held only S2/T2 — superseded material rode along in every
   request and after reopen. It now clears the projection before installing the
   latest summary + tail (pi `session-manager.ts:418-453` selects the latest
   compaction on the active path). The human log is untouched; retention is not
   the mechanism.

2. **CORE-04 / F06 — a pass commits only on a usable summary.**
   `compact_now` returns `CompactionOutcome::{Committed, Failed(String), Cancelled}`:
   - the summarization turn must have a non-failure stop reason (pi rejects
     `stopReason === "error"`, `compaction.ts:670-685`);
   - the answer must carry nonempty text — the old template fallback could
     replace useful context with a placeholder (Epsilon's own rule; pi's helper
     has no nonempty check, so this is not claimed as parity);
   - the durable entry must append successfully **before** runtime history is
     replaced, because store-backed requests read the store's projection — a
     one-view commit would leave the two views disagreeing;
   - the call is cancellation-aware at connect and while polling, with a final
     check immediately before the commit (pi checks its signal before appending,
     `agent-session.ts:1854-1881`).
   A pressure-triggered failure reports a `Notice` naming the reason and
   suppresses further pressure attempts for that run (each retry is a paid call);
   an overflow-recovery failure reports the reason and ends the run with
   `TurnEndReason::Error` instead of repeating the oversized request; a cancelled
   pass ends the run `Aborted` and commits nothing.

3. **CORE-05 / F17 — the aborted turn has the same message lifecycle as a
   completed one.** The two local cancellation arms in `stream_one_turn` now
   `push_message` the synthetic terminal message (runtime history + durable
   mirror) and emit `MessageEnd`, exactly like the provider-`Done` and
   EOF-salvage paths. Previously the partial answer existed only in
   `RunOutcome.produced`, so a resumed conversation lost it.

## Key decisions

- **Failure is an outcome, not a `RunError`.** A summarizer that errors is a
  compaction problem, not an agent-invariant violation; `RunError` stays for
  real invariant breaks (`store poisoned`, history poison).
- **One pressure attempt per run after a failure** (`compact_suppressed`). A
  summarizer that just failed will not succeed on the next turn, and every retry
  is a billable call. Overflow recovery keeps its own existing one-shot latch.
- **Order: summarize → check cancellation → append → replace runtime history.**
  The append is inside the same window as the runtime replacement so the two
  views cannot diverge (F07's remaining breadth is still open: prompt/result
  append failures carry the same class of problem for ordinary turns).
- **Cancellation wins ties** (`biased` select): the same discipline the main turn
  uses, so an aborted pass cannot later commit.

## Regressions (executed)

| Test | File | Asserts |
|---|---|---|
| `a_second_compaction_replaces_the_superseded_projection` | `epsilon-session/tests/session_store.rs` | latest summary + its tail + later messages only; first summary absent; human log keeps every entry |
| `failed_summarization_does_not_commit_and_is_reported_once` | `epsilon-core/tests/compaction_scenarios.rs` | run completes; one attempt, zero commits; Notice names the reason; no durable Compaction entry |
| `empty_summary_text_is_refused_rather_than_padded` | same | whitespace answer is named and refused; no entry |
| `abort_during_summarization_ends_the_run_aborted_and_commits_nothing` | same | run ends Aborted; one attempt, zero commits |
| `abort_mid_stream_commits_the_terminal_message_to_history_and_store` | `epsilon-core/tests/loop_scenarios.rs` | one terminal assistant message in history, store and reopened model view; exactly one `MessageEnd` |

## Verification

Round-B battery (`gate-m86-88f`, candidate `epsilon-m86-green` = HEAD `d0ac16b` + the ten
changed files, byte-verified by hash; external target dir, `CARGO_BUILD_JOBS=1`): fmt,
clippy, test, demo and real-check all exit 0 — **751 passed, 0 failed, 0 ignored across
55 summaries** (`audit/evidence/gate-m86-88f.log`, sha256 `1ccc939e…`). The test profile
was built with `CARGO_PROFILE_TEST_DEBUG=1` after the machine's AV pinned the previous
`wire_fixtures-…exe` artifact path (`LNK1104` on a file that does not exist; attempt
history in the verification log) — assertions and counts are unaffected.

RED (executed against the pre-fix code, `audit/evidence/red-round.log`, sha256
`103b5307…`): `a_second_compaction_replaces_the_superseded_projection` failed with the
model view holding *both* summaries (`first summary`, `tail-1`, `second summary`,
`tail-2`, `after` — the superseded projection was still there); the three compaction
tests failed on the pre-fix behaviour each names — committing on `StopReason::Error`,
committing an empty summary, and a cancelled pass committing nothing; the abort-salvage
test failed with "one terminal message survives the abort".

## Limits

- No live provider was called; every regression uses the faux provider.
- Early-EOF summarizer and durable-append-failure paths are implemented but not
  separately regression-tested (ledger notes say so). The append-failure branch — the exact
  case the append-before-replace order exists for — and the poisoned-history rollback both
  need a store-level fault-injection seam that does not exist yet (F07 work).
- Branch-local variants of the projection replacement (alternate branches,
  reopen) are unexecuted at the loop level; the store-level test covers the
  projection rule itself.
- Independent review (read-only, no builds) returned no blockers; its four minor
  findings and three notes, with dispositions, are recorded in
  [the review record](../../audit/evidence/m86-88-review.md) — the doc-order and
  poisoned-history findings were fixed here, the coverage gaps and event-shape
  asymmetry are documented rather than papered over.
