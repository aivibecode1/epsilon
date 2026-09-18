# Compaction correctness slice (CORE-03/04/05)

## Scope and source of truth
- Revalidation review `audit/evidence/revalidation-core-review.md` findings
  CORE-03 (durable projection), CORE-04 (failed/empty/cancelled summarization),
  CORE-05 (aborted terminal message lifecycle). Plan row 3 of `audit/09-revalidation.md`.
- Owner instruction (current session): finish the pi port faithfully before new features.

## Changes made (working tree, uncommitted)
- `crates/epsilon-session/src/store.rs` `model_view()`: a later Compaction entry now
  `out.clear()`s the projection an earlier one built — latest compaction + its retained
  tail + entries after it (pi `session-manager.ts:418-453`). Previously summaries
  accumulated and superseded material re-entered every provider request.
- `crates/epsilon-core/src/compact.rs`: `summary_text` returns `Option<String>`; the
  template placeholder fallback is deleted (empty answer must not replace context).
- `crates/epsilon-core/src/lib.rs`: new `CompactionOutcome {Committed, Failed(String),
  Cancelled}`; `compact_now` is cancellation-aware (biased select on the run token for
  stream creation and polling, final check before commit), rejects
  `stop_reason.is_failure()` and empty text, and appends the durable entry *before*
  replacing runtime history, reporting append failure instead of ignoring it
  (no partial commit). New run-scoped `compact_suppressed` flag: one pressure-triggered
  attempt per run after a failure, plus a Notice naming the reason; overflow-recovery
  failure now ends the run with `Error(reason)` instead of a second identical request;
  cancellation during compaction ends the run `Aborted`.
- Abort arms in `stream_one_turn` now push the synthetic terminal message into history +
  durable mirror and emit `MessageEnd` (same lifecycle as Done and EOF-salvage paths).
  This is the CORE-05 fix; previously the message existed only in `RunOutcome.produced`.

## Tests added
- `epsilon-session/tests/session_store.rs`: `a_second_compaction_replaces_the_superseded_projection`.
- `epsilon-core/tests/compaction_scenarios.rs`: `failed_summarization_does_not_commit_and_is_reported_once`,
  `empty_summary_text_is_refused_rather_than_padded`,
  `abort_during_summarization_ends_the_run_aborted_and_commits_nothing`; helper split
  `setup_with_summarizer` + `drain_all`.
- `epsilon-core/tests/loop_scenarios.rs`: `abort_mid_stream_commits_the_terminal_message_to_history_and_store`.

## Status and next actions
- NOT yet built/verified. Round A (find slice, `proc_641c4474afaa`) is running in an
  isolated candidate; one build owner. Round B = isolated candidate of new HEAD (after
  the find commit) + exactly: store.rs, compact.rs, lib.rs + the three test files.
- Run gates at `CARGO_BUILD_JOBS=1` with external `CARGO_TARGET_DIR` (0xc0000142 flake).
- After a green battery: milestone log (`docs/milestones/M87-…`), ADR-007 amendment,
  `docs/history.md` row, ledger rows for CORE-03/04/05 + verification-log entry,
  then commit (code + docs), retire nothing (F07 append-failure breadth still open).
- Expected RED evidence for the report: the same tests against HEAD must fail
  (store test: 5 entries in view; compaction tests: commit happens / no notice /
  abort message absent from history).
