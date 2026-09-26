# M143 — the session says where the money went, not just how much

**Status:** fixed. Battery `m143b`: **`GATE PASS`, 62 test binaries, 1,223 passed, 0 failed, 1 ignored** — up from
1,222 at M142, and the delta is exactly this slice's one new test. Negative control at
[`../../audit/evidence/m143-negative-control.md`](../../audit/evidence/m143-negative-control.md).

## What was dead code

`CostBreakdown` has four per-component accessors — `input()`, `output()`, `cache_read()`,
`cache_write()` — each documented, each returning a real value, each covered by a test in
`ui/cost.rs`. A grep for callers outside that file returned **nothing**.

That is the worst kind of dead code: it reads like a feature. A reader of `cost.rs` would
reasonably conclude the header shows a breakdown, because the module is full of careful
machinery for producing one. It did not. The header showed a single total, and the
machinery sat underneath it doing nothing.

## What pi does, and what Epsilon does

pi's `SessionStats` carries a `cost: number` alongside per-bucket `tokens`
(`agent-session.ts:270-287`), and `getSessionStats` aggregates usage across **all** session
entries. pi has no `/cost` command — so this is not a port of a command. It is the same
*idea* applied where Epsilon already had the surface: `/session`, which prints token
totals, cache attribution, and session repairs, and which had no cost line at all.

That is the honest framing: **this is not a pi feature ported, it is Epsilon's own dead code
connected to the surface it was clearly written for.**

## The line

```
cost: $0.31 · in $0.04 · out $0.26 · cache read $0.01 · cache write $0.00
```

One line, in the place the user already looks for session numbers. A total is not
actionable: `$0.31` reads the same whether the prompt cache saved 90% of the spend or none
of it, and the cache is the main lever a user has on cost.

## It says what it does not know

Three cases, each with its own wording, because collapsing them is the failure this whole
chain keeps fixing:

- **no usage yet** → `cost: nothing billed yet`, never `$0.00`, which is a
  precise-looking number about nothing;
- **a model the registry lacks** → the total is a **lower bound**, marked `+` with the same
  marker the header uses, so `/session` and the header can never disagree;
- **a genuinely free model** (ollama, a `:free` row) → `$0.00` is the true answer, and
  `is_floor` is precisely what distinguishes it from the unpriced case. This line trusts
  that flag rather than second-guessing the registry.

The floor marker is reused rather than reinvented because consistency between the two
surfaces is the property that matters: if `/session` said `$0.31` while the header said
`$0.31+`, the user would have no way to know which to believe.

## The control, and what it taught

Removing the line: **1 failed, 320 passed.** The sibling `/session` test still passes,
which is what makes the control informative — it removed a *line*, not the command.

**The first control attempt passed all 321 tests.** That looked like a weak test and wasn't:
`/session` has two branches, and the test's fixture builds a store-less agent, so it takes
the *other* branch from the one I edited. The control had removed code the test never ran.

That is a third way a control can be wrong, and it is worth recording beside the other two:
it can be *different* code rather than the old code (M141), it can be uninformative about
a property by design (M142), or it can be applied **where the test does not execute**
(M143). The rule that covers all three: *a control must be applied where the test's
execution actually goes.* Finding that out meant reading the fixture — re-running would
have kept producing green with the control still in place.

## F22 — found here, fixed elsewhere

Reading pi's `getSessionStats` to check parity turned up a separate defect: pi explicitly
adds usage from `compaction` and `branch_summary` entries so totals "reflect what was
actually billed". Epsilon has `usage: Option<Usage>` on **both** entry kinds and reads
neither. Compaction is a real provider call whose cost is currently uncounted, so the
session total under-reports — worst in exactly the long sessions where cost matters.

It is recorded, with acceptance criteria, at
[`../../audit/evidence/F22-compaction-usage-never-read.md`](../../audit/evidence/F22-compaction-usage-never-read.md)
and deliberately **not** folded in here. Fixing it means deciding which model a
summarization is billed to, and `Compaction` does not record one — a design decision with a
wrong answer available, not a patch.

## The first battery round failed, and the diagnosis changed

Battery `m143` failed at 46 of 62 on `pi_import_carries_every_source_entry` — a *different* test in the *same*
test binary that failed in `m142`, with the same `os error 2` on a `fs::rename` whose `fs::write` had just
succeeded. Three occurrences is a pattern, not a flake, so the earlier AV-quarantine hypothesis was wrong and
was replaced.

**The real cause: this host exports `TMPDIR=/tmp TEMP=/tmp TMP=/tmp`, and native Windows resolves those to
`C:\tmp`, which does not exist.** `std::env::temp_dir()` follows the process environment, so any test using
`tempfile` inherits a root the environment and the machine disagree about; the AV filter then has a chance to
win the race in the write→rename window, nondeterministically, which is why a different test loses it each
round and why isolated re-runs pass.

`m143b` re-ran with `TMP`/`TEMP`/`TMPDIR` exported to `C:/Users/Hp/AppData/Local/Temp` and passed on the **first**
attempt — both previously-failing tests green in the same run. That is what distinguishes a diagnosed cause from
a hopeful one. The full analysis, including why a passing re-run is evidence about the *environment* and not
the code, is at
[`../../audit/evidence/temp-var-os-error-2-diagnosis.md`](../../audit/evidence/temp-var-os-error-2-diagnosis.md).

## Limits — what this slice does not do

- **Only the "nothing billed yet" path is tested.** The test drives a store-less agent with
  no usage. The priced path — four components, a total, and a possible `+` — is rendered by
  the same `format!`, but no test asserts a real line's text. `cost.rs` tests the
  arithmetic; this tests that the line is emitted.
- **Not a separate `/cost` command.** The breakdown is a line inside `/session`, which also
  prints file paths, leaf ids, and repair notices. A user who wants only the money has to
  read past them. A dedicated command is a reasonable follow-up, and the rendering is
  already a separate function.
- **The no-store branch and the store branch print the same line but were reached
  differently.** The store-backed branch's call is covered only by the compiler.

## What nothing exercises

- **The formatted line's exact text with non-zero components.** `format_cost` is tested at
  every scale in `ui/format.rs`, so each component's formatting is proven; their *assembly*
  into one line is not asserted anywhere.
- **`/session` after a real provider call.** Every test uses a fixture with no usage, so the
  priced path is never rendered end to end.
- **A floor-marked line.** No test produces a model absent from the registry and asserts the
  `+` reaches the notice. That is the case most worth adding next, because it is the one
  where being wrong understates the bill.
