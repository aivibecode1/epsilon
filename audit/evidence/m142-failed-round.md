# M142 — battery `m142` failed on an unrelated test; `m142b` is the cited evidence

## What happened

Battery `m142` reached the workspace test phase and failed at 46 of 62 binaries:

```
running 5 tests
test reconcile_an_external_pair ... ok
test abandoned_branches_are_not_imported ... ok
test per_kind_counts_tie_out_on_both_sides ... FAILED

thread 'per_kind_counts_tie_out_on_both_sides' panicked at
  crates\epsilon-session\tests\pi_import_reconciliation.rs:144:29:
rename into C:\Users\Hp\AppData\Local\Temp\epsilon-pi-reconciliation-20532\before-compaction.v2.jsonl:
  The system cannot find the file specified. (os error 2)

test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
===== GATE FAIL =====
```

## Why this is not a defect in this slice

**M142 changed no product code.** It added six tests to `crates/epsilon-tui/src/state.rs`.
The failing test is in `crates/epsilon-session/tests/pi_import_reconciliation.rs`, a crate
this slice did not touch, testing the pi session importer, which has no relationship to
token accounting.

**The failure is environmental, and the message says so.** `os error 2` is
`ERROR_FILE_NOT_FOUND`, raised by `std::fs::rename`. The sequence in the test is
`std::fs::write(&tmp_path, ..)` then `std::fs::rename(&tmp_path, out)` — the write is
checked and succeeded, and the rename then reported that the very file just written was
absent. Code does not do that. This is the Avira-quarantine signature already recorded in
`agents/memory/next-hardening.md`: a security product intercepting a freshly written file
under a watched path, in a workspace where a 35–40 GB `target/` has repeatedly pushed C: to
89% full and Avira has previously been observed deleting artifacts mid-run.

**Re-run on the same bytes, 5/5 pass.** No source change between the failing run and this
one:

```
test reconcile_an_external_pair ... ok
test abandoned_branches_are_not_imported ... ok
test the_m119_gap_is_resume_repair_not_import_loss ... ok
test per_kind_counts_tie_out_on_both_sides ... ok
test pi_import_carries_every_source_entry ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Why `m142b` was run anyway

A passing re-run of the single test is not a full battery, and a battery that stops at
46 of 62 binaries has not exercised the demo gate, the real-feature check, or the remaining
16 test binaries. So the battery was re-run end to end on the same bytes, under a new label
— the repo's rule that every attempt gets its own label, because `gate.ps1` truncates the
log on start and re-using a label would have destroyed the failed round's evidence.

**`m142` is kept in this repository as the record of what happened. Only `m142b` is cited as
passing evidence.** The distinction matters more than the convenience: an audit that quietly
discards failed rounds is exactly the "false bill of health" that the M134 ledger work
removed, and the failure here is genuinely informative — it is the second time this machine
has produced a mid-run artifact deletion, and it is why `CARGO_TARGET_DIR` lives outside the
repository.

## What a reader should take from this

A flaky failure is still information. This one says the machine's disk pressure and AV
filtering are now a recurring source of false test failures, and that the mitigation in use
(an external `CARGO_TARGET_DIR` under `%LOCALAPPDATA%\Temp`) reduces but does not eliminate
it. If this recurs on paths outside the target directory — as it did here, in a test's own
`%TEMP%` scratch directory — the honest conclusion is that the remaining exposure is the
AV filter, not the build layout, and that is an owner decision (an exclusion for
`%LOCALAPPDATA%\Temp`) rather than something an agent session should change.
