# F22 — compaction usage is stored and never read (port fidelity: `getSessionStats`)

**Status: CLOSED, and the second half was never a defect.** M145 closed the compaction half — the bill is recorded on the entry that incurred it, keyed by the model actually charged, and read back on resume. The branch-summary half is **withdrawn as a defect** on the evidence below; the missing *capability* is already tracked as the "Cross-branch navigation, branch summaries, fork" row of `audit/03-port-plan.md`, which had it right. Found while building M143; deliberately not folded into it.
**Severity:** P2 — the number is wrong, not missing, and it is wrong *low*.

## The gap

pi's `getSessionStats` (`agent-session.ts:3325-3341`) aggregates over **all** session
entries, and its first move is:

```ts
if ((entry.type === "branch_summary" || entry.type === "compaction") && entry.usage) {
    addUsageToTotals(usageTotals, entry.usage);
}
```

The comment above it states the reason plainly:

> Aggregates over ALL session entries (including history that was compacted away), so
> token/cost totals reflect what was actually billed across the session.

Epsilon has **both fields on both entry kinds** — `EntryKind::Compaction { usage:
Option<Usage>, … }` and `EntryKind::BranchSummary { usage: Option<Usage>, … }` — and
neither is ever read. A grep for `Compaction { … usage` across the workspace returns zero
read sites; the fields are written by the summarizer and dropped by every consumer.

## Why it matters

Compaction is a **real provider call**. It sends the whole history to a model and pays for
it. That money is spent and then not counted, so:

- the session's token totals under-report, by however much the summarization cost;
- the cost figure under-reports by the same amount, and is *not* marked as a floor,
  because nothing tells `cost::breakdown` that tokens are missing — they were never
  recorded at all;
- a session that compacts heavily — exactly the long sessions where cost matters — is the
  session that under-reports most.

This is the same class of defect as M139 (`model_change` written, never read) and it has
the same signature in the code: a field that is typed, documented, persisted, tested for
round-tripping, and unreachable.

## Why it was not fixed in M143

M143 was a UI slice: surface the per-component cost breakdown that already existed and had
no caller. This is a different layer — the accumulation is missing an input, not an
output. Fixing it means deciding where compaction usage is *attributed*:

- to the model that performed the summarization (the right answer for pricing, and
  `Compaction` does not currently record which model was used — so this needs a new field
  or an explicit "unknown model" bucket);
- into the flat totals but not the per-model map (cheaper, but then the two disagree and
  M142's "counters and bill must agree" property breaks);
- into both, with the model taken from the retained tail or recorded at write time.

That is a design decision with a wrong answer available, not a patch. It is the next slice.

## Acceptance

1. A session whose compaction recorded usage reports that usage in the totals **and** in
   the cost, priced at the model that performed the summarization.
2. A compaction with `usage: None` (an older session, or a failed summarization) does not
   move any number and does not mark the total as a floor — there is nothing to count.
3. `/session` after a compaction reports a total that includes the summarization cost.
4. A negative control: drop the aggregation and show tests 1 and 3 fail.
5. The model used for summarization is recorded. If the design instead introduces an
   "unpriced/unknown model" bucket, the floor marker must fire — a cheaper bill that looks
   complete is the failure this whole chain exists to prevent.


## Correction (M144): the two halves are different defects

Working the fix turned up something the original finding got wrong by grouping them.

**Compaction** was a real provider call whose usage was **discarded at the write site**:
`compact_now` had the summarizer's response in scope and wrote `usage: None`. M144 records
it, as a new `UsageAccount` entry carrying the model that was actually charged.

**Branch summaries are never a provider call in Epsilon at all.** Every caller of
`branch_with_summary` passes `None` for usage — all five are tests, and the production path
passes a summary it already has. So there is no bill to lose: the `usage` field on
`BranchSummary` is not a discarded measurement, it is an **unimplemented feature**. F22 as
written ("stored and never read") describes compaction accurately and branch summaries
incorrectly.

That distinction decides the fix, and it is why the two cannot share one:

- compaction needs its usage **captured and attributed**;
- branch summaries need a **summarization call built** — a provider request, its own
  context budget, its own failure mode, and a decision about whether abandoning a branch
  should be worth a summary at all.

Closing the second means adding a feature pi has and Epsilon does not, so it is recorded
here as open rather than counted as done. The `UsageAccount` entry is deliberately shaped
to take it (`reason: "branch_summary"`) when that happens.

## Acceptance criteria, status

| # | Criterion | Status |
|---|---|---|
| 1 | A compaction's real usage is recorded durably | done (M145) |
| 2 | It is keyed by the model **actually charged**, not the header's | done (M145) — a test fails if a second model's row is merged into the first |
| 3 | Repeated compactions **add** rather than overwrite | done (M145) — the write-side test asserts the reader's total equals the SUM over entries, because the threshold fires twice in that fixture |
| 4 | A resumed session reads the rows back into the totals | done (M145) — `app.rs` adds them *after* hydration, and a test asserts why that order matters |
| 5 | A legacy compaction with usage but no charging model is **not** priced at an invented rate | done (M145) — under-reporting is the chosen direction to be wrong in. Both arms are asserted: usage with no model is skipped, and the SAME entry with a model IS counted |
| 6 | A branch summary is a real billed call, attributed like a compaction | **withdrawn — no such call exists in Epsilon.** See “Why row 6 was withdrawn” below; the capability is tracked in `03-port-plan.md` |

## Why row 6 was withdrawn

I claimed the branch-summary half was "still open" and "needs the call to exist first". The
second half of that sentence was doing the real work, and it turns out to mean there is **no
defect at all** — I had filed a missing *feature* as a wrong *number*.

What the code actually says, checked rather than recalled:

1. **pi does bill it.** `agent-session.ts:3205-3225` calls `generateBranchSummary(...)` and takes
   `summaryUsage = result.usage`, passed as the 5th argument to `branchWithSummary`. So in pi a
   branch summary is a real provider call with a real cost.
2. **Epsilon has no such call.** There is no `generate_branch_summary` anywhere in the workspace.
   Every caller of `branch_with_summary` is a **test** — 7 sites, all in
   `crates/epsilon-session/tests/session_store.rs` and `compaction_scenarios.rs`, all passing
   `None` for usage.
3. **Epsilon has no way to abandon a branch.** Searching `epsilon-tui/src`, `epsilon-cli/src` and
   `epsilon-core/src` for `rewind|navigate_back|edit_previous|fork_branch|switch_branch` returns
   **zero** non-test hits. The feature that would create an abandoned branch is not ported, so
   there is no abandoned branch to summarize.
4. **~~pi does not call it either.~~ WITHDRAWN — this was wrong, and it was my error.**
   I wrote that `AgentSession.navigateTo` has no caller in pi, based on a repo-wide grep for
   `navigateTo` that returned 5 hits, all an unrelated cursor option in
   `tui/src/components/settings-list.ts`. **That grep was for a name that does not exist in
   0.85.1.** The method is `AgentSession.navigateTree`, and it **does** have callers:
   `interactive-mode.ts:1889,5288`, `print-mode.ts:86`, `extensions/runner.ts:827`,
   `agent-controller-provider.ts:60`. A grep that returns nothing because the identifier is
   wrong proves nothing, and I wrote the conclusion as though it had.

**The argument that does hold**, and it does not depend on any identifier guess: the only
production site that constructs an `EntryKind::BranchSummary` is `store.rs:1397`, inside
`branch_with_summary`, and **every caller of `branch_with_summary` is a test** (7 sites). A
grep for "how do you navigate" can miss; a grep for "where is the thing constructed" cannot.
Two other Epsilon facts, re-checked with the right identifiers this time:

5. **`/tree` exists but does not navigate.** `picker.rs:354` advertises "navigate session tree
   (switch branches)", and `app.rs:1721` handles it — but that arm is an **alias for
   `/timeline`**: it calls `store.branch_entries()` and opens the same picker. It *lists*
   branches; selecting one does not create a `BranchSummary`. A slash command's own description
   is not a specification, which is the same mistake as citing a build artifact earlier.
6. **No rewind or edit-previous path exists** — zero non-test hits across tui, cli and core.

So the conclusion is unchanged and now rests on point 4's construction-site argument, but the
original reasoning was invalid and should not be repeated.

So: **nothing in Epsilon spends money summarizing a branch, therefore nothing is unbilled.**
A reader that counts a `BranchSummary`'s usage is correct and already tested; it is simply fed
only by sessions pi wrote.

**Why the `BranchSummary` code is not dead, and must not be deleted.** Epsilon shares pi's
session file format — its header is pi's (`"type":"session"`), and the model-view projection
replicates pi's own truthiness guard verbatim (`store.rs:1625`: `entry.type === "branch_summary"
&& entry.summary`). An imported pi session can contain branch summaries, so the **reader** arm
earns its keep for compatibility. It is a read path with no writer *yet* — which is a missing
feature, not dead code.
