# M144 — a summarization is a real provider call, so it is a real bill

**Status: READ SIDE ONLY. The write side is reverted and is now F23.**
Battery `m144j`: **`GATE PASS`, 62 test binaries, 1,230 passed, 0 failed, 1 ignored** — up from 1,223 at
M143b, and the delta is exactly this slice's seven reader tests;
negative control at
[`../../audit/evidence/m144-negative-control.md`](../../audit/evidence/m144-negative-control.md);
F23 at
[`../../audit/evidence/F23-accounting-row-collides-with-pop-last.md`](../../audit/evidence/F23-accounting-row-collides-with-pop-last.md).

## What shipped, and what did not

**Shipped — the read side.** The new `UsageAccount` entry kind,
`SessionStore::usage_accounts()` grouping rows by charging model and reason,
`UiState::add_usage_accounts` folding them into the per-model rows *and* the flat totals, and
the resume path in `app.rs` reading them after hydration. Seven tests, all green.

**Not shipped — the write side.** `compact_now` recorded the usage and appended the row. It
worked in isolation and **was reverted**, because it collided with `pop_last_message` — see
F23. No session file produced by this build contains a `UsageAccount` row.

That is an unsatisfying place to land, so the reasoning belongs here rather than in a commit
message: the reader is correct and tested, and it is inert until a writer exists. Saying "the
bill is now counted" would be false, which is why the status line says READ SIDE ONLY.

## Why a new entry kind rather than a new field

`Compaction` and `BranchSummary` already had a `usage: Option<Usage>`. Filling it in would
have been the smaller change, and it would have been the wrong one: a `Compaction` with
`usage: None` means two different things — *never billed* and *billed by a session written
before this entry kind existed* — and accounting cannot tell them apart. A `Compaction` with
usage but no charging model has to be priced at *some* model's rate, and guessing would be
the M141 defect reintroduced one layer up.

`UsageAccount { model_id, provider, reason, usage }` records only what can be attributed, so
the reader never has to infer. `reason` is a free-form tag rather than an enum, so a future
caller does not widen a contract that is only ever read for grouping.

The legacy `Compaction.usage` is still populated, so a reader predating this entry kind finds
the measurement beside it.

## The read path cannot go through messages

`hydrate_from_history` takes the store's **message** view. A `UsageAccount` is not a message
— it never enters LLM context. Routing billing through that method would mean either
pretending the entry is a message or widening the method's contract, and the first is the
mistake that let the charge go uncounted.

So there is a separate `UiState::add_usage_accounts`, and the ordering is a tested property
rather than a comment: `hydrate_from_history` clears `usage_by_model`, so bills added first
are silently discarded. `app.rs`'s resume path therefore adds them **after** hydrating, and
`hydration_and_accounting_compose_in_either_order` asserts both halves — the surviving order
and the silently-losing one — so reordering those two lines produces a red test naming the
charge that vanished.

## One thing the test caught in my own fix

`record_usage` fills only the per-model map (M141's split). My first `add_usage_accounts`
called it and nothing else, so the **cost breakdown would have been right while the flat
totals — `/session`'s token line and the header's counters — still ignored the spend.** That
is F22's exact class of defect, one function over, reintroduced by the fix for it.
`a_recorded_compaction_bill_reaches_the_sessions_token_rows` asserts the flat totals and
failed with `left: (0, 0)`.

## The control, and what it exposed

Reverting the write left **all 249 `epsilon-core` tests green**. Every test in the slice
exercised the *reader* with hand-built rows; nothing tested the *writer*. A fix that recorded
nothing would have passed the whole slice.

That is a fourth way a control can mislead, after M141's *different code*, M142's
*uninformative* subject, and M143's *wrong branch*:

> **A control must exercise the code the fix actually changed.** A slice that adds a read
> path must also be controlled at the write path, or its two halves can be independently
> broken.

The test that drove a real compaction through a real agent is what surfaced F23, and it was
removed along with the write side it covered. Its captured failure is preserved in the F23 file:
`kinds=["message", "message", "message", "compaction", "message", "message", "compaction",
"message"]` — two compaction entries and no `usage_account` between them, which is how the
collision became visible.

A first attempt at that control also failed, wrongly: with 2,000-char turns the pressure
trigger never fired, so the run failed before reaching the assertion about billing. A test
that fails before its subject is not evidence. The fixture now copies
`pressure_triggers_compaction_and_persists_entry`'s shape exactly and asserts the trigger
fired, so that false RED cannot recur silently.

## F22 is half closed, and the other half is a different defect

Working the fix corrected my own finding. F22 grouped compaction and branch summaries as
"stored and never read"; that is accurate for compaction and **wrong** for branch summaries.

Every caller of `branch_with_summary` passes `None` — all five are tests. There is no bill to
lose: the `usage` field on `BranchSummary` is not a discarded measurement, it is an
**unimplemented feature**. Closing it means building a provider call Epsilon does not make,
with its own context budget, failure mode, and a decision about whether abandoning a branch
should cost a summary at all. It stays open, with acceptance criteria, in the F22 file. The
`UsageAccount` shape is deliberately ready to take it (`reason: "branch_summary"`).

## Limits — what this slice does not do

- **Nothing writes a `UsageAccount` row.** The whole chain this milestone completes is inert
  until F23 is resolved. The reader, the grouping, the TUI fold-in and the resume wiring are
  real and tested; they have no producer.
- **The write-side test is gone with the write side.** Its captured failure lives in the F23
  file, but no test in the tree now drives a compaction expecting a bill.
- **A resumed session reads zero bills** until a writer exists, so the resume path is proven
  only against store rows inserted by hand.

## What nothing exercises

- **A compaction billed to a model *different* from the conversation's model, end to end.**
  Two store tests and one TUI test cover the per-model keying with hand-built rows; nothing
  drives a real agent whose summarizer model differs from the main loop's.
- **The floor marker with a summarizer's model absent from the catalog.** A cheap local
  summarizer in a session driven by a priced model would mark the total as a lower bound —
  correct, and entirely unexercised.
- **`add_usage_accounts` with cache buckets set.** Both call sites currently pass zeros;
  whether a provider that reports cache traffic on a summarization would be counted
  correctly is untested.
- **Two compactions in one resumed session.** The store test adds two rows and checks they
  sum; nothing resumes a session and confirms both survive the round trip.
