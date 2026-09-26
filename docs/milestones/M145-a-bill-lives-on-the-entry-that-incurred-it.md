# M145 — a bill lives on the entry that incurred it

**Status:** shipped, read and write. Battery `m145`. **F23 is closed and F22's compaction
half is fixed.** Negative control:
[`../../audit/evidence/m145-negative-control.md`](../../audit/evidence/m145-negative-control.md).

## The problem F23 named

M144's write side appended a separate `UsageAccount` entry after the `Compaction` entry and
was reverted: `Agent::pop_last_message` pops the **last entry** to discard a failed turn,
while runtime `history.pop()` pops the last **message**, and the two agree only while nothing
non-message is last. A bill in the tail is a bill a rollback can silently eat.

M145 removes the collision by **not having a second entry**. This is pi's shape, and reading
`references/pi-0.85.1/packages/coding-agent/src/core/session-manager.ts:68-92` is what
settled it:

```ts
export interface CompactionEntry<T = unknown> extends SessionEntryBase {
    type: "compaction";
    summary: string;
    firstKeptEntryId: string;
    tokensBefore: number;
    /** Usage from the LLM call(s) that generated this summary, if available */
    usage?: Usage;
}
```

**pi has no accounting entry at all.** The bill is a field on the very entry it bills, so a
positional pop removes the summary and its cost together — F23 cannot exist. `getSessionStats`
reads it from the same loop that walks the transcript (`agent-session.ts:3341-3344`):

```ts
if ((entry.type === "branch_summary" || entry.type === "compaction") && entry.usage) {
    addUsageToTotals(usageTotals, entry.usage);
}
```

M144's `UsageAccount` was a reasonable invention and the wrong one. The evidence for that is
not that it was awkward — it is that the reference implementation solves the same problem
without it, and the reason it can is exactly the reason mine could not.

## Why Epsilon still needs the model, and pi does not

pi's `Usage` carries a pre-computed `cost` (`usage-totals.ts:27`: `totals.cost +=
usage.cost.total`), so aggregating needs no attribution — every call already knows what it
cost. Epsilon's `Usage` is four token counts and nothing else; price comes from the catalog
at read time (`M141`'s `breakdown_per_model`), so **a bill without a model cannot be priced**.

That difference is the whole design problem, and it is why the fix is not "copy pi" but
"adopt pi's shape and keep Epsilon's one necessary extra".

pi's own fallback is instructive and must **not** be copied: `getUsageCostBreakdown` buckets
compaction under the literal key `"Tools/summaries"` (`usage-totals.ts:51-53`), lumping it in
with tool results. That is a display grouping for a cost that was already known, not an
attribution. Copying it here would price a summarizer's tokens at some arbitrary rate — the
M141 defect in a new place.

## The decision

`Compaction` and `BranchSummary` gain a `model_id: Option<String>`, beside the `usage` field
that already exists on both:

- **the bill and its price live together on one entry**, so any pop, rollback or truncation
  removes them as a unit — F23 is structurally impossible;
- **`model_id` is `Option`** because sessions written before M145 have usage and no model.
  The reader skips those rather than inventing a price, which is the same deliberate
  direction M144 chose and the same one a store test asserts;
- **`UsageAccount` is deleted.** It has no writer and no consumer that cannot be served by
  the two existing entry kinds. Shipping a contract nobody writes is the dead code M143 spent
  a milestone removing elsewhere.

## Verification

**All five gates green on the final bytes** — `audit/evidence/gate-m145-test.log` (62 binaries)
and `audit/evidence/gate-m145-demo.log`.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --features real -- -D warnings` | exit 0 |
| `cargo test --workspace --features real` | **62 binaries, 1,232 passed, 0 failed**, exit 0 |
| `cargo run -q -p epsilon-cli --features real -- --demo` | exit 0, 6 messages, 300/150 tokens |
| `cargo check -p epsilon-cli --features real` | exit 0 |

**1,232 against M144j's 1,230: +2.** One is the new
`a_compaction_records_its_bill_on_its_own_entry`; the other is the `BranchSummary` arm of
`a_pop_that_discards_a_turn_takes_its_bill_with_it`. The round-trip test in
`epsilon-contracts` grew assertions rather than cases, so it contributes no count.

### How these five were run, and why that matters

`scripts/gate.ps1 m145b`, `m145c` and `m145d` **did not complete.** `m145b` and `m145c` were
reaped mid-build with no verdict; `m145d` did report — and it caught four real compile
failures my earlier focused runs had missed, because a focused `-p epsilon-session` run does
not compile `epsilon-contracts`' own tests. `m145e` caught a fifth: I wrote `Usage` literals
from memory and omitted `total_tokens`.

So the passing set above is **not** a `gate.ps1` log. Each gate was run directly, in the
order above, against the same frozen source, and the logs are kept separately. That is a real
departure from the AGENTS.md rule that the battery is the record of record, and it is
disclosed rather than papered over: the next round on a stable machine should be a single
`gate.ps1` run, and this milestone's numbers should be read as "the five commands passed",
not "a battery passed". The reason is recorded in
[`../../audit/evidence/m145-battery-does-not-survive-this-session.md`](../../audit/evidence/m145-battery-does-not-survive-this-session.md).

Two of the four `m145d` failures were **my** errors, not the design's, and both are recorded
because they are the kind that a focused green run hides:

- `epsilon-contracts`' all-kinds round-trip test built `Compaction` and `BranchSummary` with
  `usage: None`, which compiles only because the new field was defaulted. A round-trip test
  that leaves a field at `None` **cannot tell whether it survives serialization**, so both
  now carry real usage and a real `model_id`.
- `Usage` has five fields, not four. I asserted the sum for `total_tokens` in one place and
  omitted it in another; the sum would also have been a worse test, since `total_tokens` is
  provider-reported and deliberately not derivable from the other four. It is now `25` where
  the fields sum to `21`.

## The negative control

M144's shape — the bill on a **separate second entry** — gives **1 failed, 10 passed**:

```
a billed compaction recorded nothing, so its cost is invisible
test result: FAILED. 10 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

Captured in [`../../audit/evidence/m145-negative-control.md`](../../audit/evidence/m145-negative-control.md).
Restoring was verified by source, not assumed: zero `CONTROL` markers remain in
`crates/epsilon-core/src/lib.rs`, and the `Some(answer.usage)` / `Some(model.id.clone())`
pair is present.

## What this does not solve, and says so

- **Existing `Compaction` usage stays uncounted.** No model is recorded for it, and inventing
  one is the failure this whole chain keeps fixing. Under-reporting an old session is the
  honest direction.
- **The summarizer's model is the agent's current model.** `compact_now` uses
  `self.model_id`, so a future "summarize with a cheaper model" feature would need the
  recorded id to diverge from the header — which is exactly why the field is on the entry
  rather than read from the header at tally time.
- **`BranchSummary` still bills nothing**, because every caller passes `None` and no
  summarization call exists (F22's second half, still open). The new field makes it
  *possible* to record one; it does not create the call.

## What nothing exercises

- **A `BranchSummary` that carries a bill.** The reader matches the kind and the store test
  covers `Compaction`; nothing appends a `BranchSummary` with `usage` + `model_id` set, so
  that match arm is compiled and reasoned about but never driven.
- **A summarizer billed to a model different from the conversation's.** The per-model
  keying is proven with hand-built entries; no test drives a real agent whose summarizer
  model differs from the main loop's, which is the case the `model_id` field exists for.
- **A real provider's usage decoding into this bill.** The end-to-end test runs on
  `FauxProvider`, so nothing proves a live provider's `Usage` reaches the entry and then
  `/session`. That requires a real call.
- **A compaction that fails after the summary is billed.** If a later step of `compact_now`
  fails, the entry is already durable with its bill — correct, but no test asserts the
  resulting state.
- **The pi-import path with a `Compaction` entry.** `usage: None, model_id: None` is what
  it writes, and no test asserts the imported session reports no summarization cost rather
  than a wrong one.
