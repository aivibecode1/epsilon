# F23 — a durable accounting entry collides with `pop_last_message`

**Status:** CLOSED by M145 — the collision is now structurally impossible, and a store test
(`a_pop_that_discards_a_turn_takes_its_bill_with_it`) reproduces the pop and asserts the bill
goes with the summary. **The fix was to delete M144's second entry**, adopting pi's shape: the
bill is a field on the `Compaction` that incurred it, so there is nothing for a positional
rollback to separate. That is option (3) from the analysis below — reached by reading the
reference implementation rather than by choosing among the options.
**Severity:** P2, but it is a **precondition** for F22, not a parallel defect.

## What was attempted

F22's compaction fix appends a `UsageAccount` entry to the session file immediately after
the `Compaction` entry, inside `Agent::compact_now`. The full implementation — recording
`answer.usage`, attributing it to `model.id` (the `ModelRef` the summarizer actually ran
with), and reading it back through `SessionStore::usage_accounts` — worked. The tests passed
in isolation; the four store-level reader tests and the three TUI tests were green.

Then the battery failed, and a diagnostic added to the write-side test showed exactly why:

```
a billed compaction left no UsageAccount, so its cost is invisible;
kinds=["message", "message", "message", "compaction", "message", "message", "compaction", "message"]
```

**Two `compaction` entries, and no `usage_account` between them.** The row was written and
then removed.

## The collision

`Agent::pop_last_message` (`crates/epsilon-core/src/lib.rs:1086-1098`) discards a turn that
must not be kept — the overflow-retry path at line 1550, and the F07 rollback at line 2212:

```rust
fn pop_last_message(&self) -> Result<(), RunError> {
    if let Ok(mut h) = self.history.lock() { h.pop(); }
    if let Ok(store) = self.session_store.lock() && let Some(st) = store.as_ref() {
        let mut s = st.lock().map_err(|_| store_lock_failure())?;
        s.pop_last()...
    }
    Ok(())
}
```

`SessionStore::pop_last` removes **the last entry in the file**, whatever kind it is. The
runtime `history.pop()` removes the last *message*. Those agree only while every non-message
entry is either absent or never the last thing written.

Appending `UsageAccount` immediately after `Compaction` breaks the agreement: the two
appends are one logical operation, and the store has no way to undo them as a unit. A pop
meant for a message can consume the accounting row, and the same pop then leaves a
`Compaction` behind with no bill — which is exactly the state the diagnostic printed.

This is the **same shape as F07** (`audit/02-defect-ledger.md`): two durable writes that must
agree, where committing only one of them leaves the views disagreeing. F07's answer was to
treat the pair as one write; this pair cannot be, because the store's rollback primitive is
positional and the two entries are not adjacent in the sense that matters.

## Why the write side was reverted rather than fixed

Three options, and none of them is a patch:

1. **Make the accounting row the last thing written, always.** Not possible: the rollbacks
   are reached from paths that do not know an accounting row exists.
2. **Teach `pop_last_message` to skip accounting rows.** It would have to pop repeatedly
   until it removed a non-accounting entry — silently discarding anything in between,
   including a `Compaction` summary. That converts a bounded bug into an unbounded one.
3. **Record the bill somewhere that is not the append-only transcript.** A sidecar, or a
   header field, or a footer recomputed on close. This costs the property that made the
   entry attractive (it is durable and ordered with the thing it bills), but it removes the
   collision entirely, because a file's tail is then only ever a message.

The honest position is that **F22's write side needs a design decision, not an edit**, and
that shipping the reader alone is a half-measure. The reader is shipped (it is correct, it
is tested, and it has no consumer that can misfire on a missing row), and it will pick up
bills the moment a writer exists. The writer is not shipped, so **no session file has ever
been written with a `UsageAccount` row by this build.**

## Acceptance criteria

| # | Criterion | Status |
|---|---|---|
| 1 | Popping a failed turn can never remove a bill | **open** — needs a decision from (3) above |
| 2 | A `Compaction` row and its bill are undone as a unit, or not at all | **open** — same |
| 3 | Two compactions in one run each keep their own bill | reader-proven (a store test), **writer-unproven** |
| 4 | `usage_accounts()` never prices a row with no charging model | **done** (M144) — a legacy compaction with usage is deliberately not counted |
| 5 | `add_usage_accounts` reaches the flat totals as well as the per-model rows | **done** (M144) — the slice's own test caught the omission |
| 6 | A resumed session folds bills into its totals | **done** (M144), inert until (1) lands |

## The lesson

The battery caught this; a focused test run did not. The write-side test passed in
isolation and failed in the full suite, because `pop_last_message` is reached by a *different*
test's turn — the interaction only exists when several compactions and a rollback share one
session file.

That is an argument for the full battery, but it is also an argument against the shape of the
change: a durable record that a positional rollback can silently eat is fragile no matter how
many tests pass. The `Limits` section of the M144 milestone says the write side is absent; the
reason is here.
