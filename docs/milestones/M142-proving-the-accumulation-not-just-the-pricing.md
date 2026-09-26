# M142 — proving the accumulation, not just the pricing

**Status:** tests only; no behaviour change. Battery `m142b`: **`GATE PASS`, 62 test binaries, 1,222
passed, 0 failed, 1 ignored** — up from 1,216 at M141, and the delta is exactly these 6 tests. Negative
control at
[`../../audit/evidence/m142-negative-control.md`](../../audit/evidence/m142-negative-control.md).

M141 fixed two things and could only prove one of them. It fixed the *pricing* — each
model's tokens at that model's rates — and it changed the *accounting* so that the
per-model split exists to price. Its six tests all lived in `ui/cost.rs` and all called
`breakdown_per_model` with a hand-built `BTreeMap`.

Which means a bug in `UiState::record_usage` — the code that *builds* that map — would have
passed every one of M141's tests. A perfect pricing function fed a collapsed map produces
exactly the defect M141 set out to remove, and M141's evidence could not see it.

This slice closes that gap. It adds no product code.

## What is now covered

Six tests in `state.rs`, driving a real `UiState` through the two paths that populate the
map — the live `MessageEnd` stream and `hydrate_from_history`'s `/resume` replay — and
reading the map back.

1. `each_turn_lands_under_the_model_that_produced_it` — two turns, two models, two rows.
   The merged row is the state M141 had to eliminate, so this is the primary property.
2. `a_second_turn_on_one_model_adds_to_that_models_row` — three turns on one model sum into
   one row. Without it, a `record_usage` that *replaced* rather than added would satisfy
   test 1 and lose per-turn detail the pricing depends on.
3. `the_flat_totals_still_equal_the_sum_of_the_rows` — the token counters and the bill must
   agree, or the header shows two different sessions at once.
4. `a_resumed_session_keeps_each_messages_own_model` — replayed history is attributed to
   each message's own `model_id`. This is the case M141 most improves and the one that had
   **no coverage at all**; it is named in M141's "What nothing exercises".
5. `resuming_replaces_the_previous_sessions_rows` — a `/resume` must not merge into the
   session already on screen. If the map were not cleared alongside the flat totals, the
   map would be the *only* wrong thing, which is exactly the kind of drift a test that only
   checks the totals would miss.
6. `a_reply_lands_on_its_own_model_even_after_a_switch` — a turn that was in flight when
   the user switched stays on the model that was actually billed for it.

## The control, and the one test that survived it

`record_usage` was changed to key every turn under the constant `"collapsed"`. **5 failed,
1 passed.**

The failure messages print the map, so the control needs no commentary:

```
two models, two rows: {"collapsed": TokenCounts { input: 3000, output: 300, ... }}
  left: 1
 right: 2
```

Two turns on two models, one row, all 3,000 tokens in it. For the resumed session:
`{"collapsed": input: 5000}`. The information the pricing depends on, visibly gone.

The survivor is worth reading. `the_flat_totals_still_equal_the_sum_of_the_rows` compares
the flat total against the sum of the rows, so collapsing the rows into one leaves both
sides equal and it passes. That is the test behaving correctly: its subject is *agreement*
between the counters and the bill, and under the control they do agree — they are simply
both wrong about which model was involved. A more specific assertion about row layout would
have failed here too, and would have been testing the layout rather than the property.

## Why this was worth doing as its own slice

It is a small slice with no user-visible change, which is exactly the kind of work that gets
skipped. But the standing goal is a harness whose numbers are *accurate*, and accuracy that
rests on an untested half is accuracy that can be undone by the next person who edits an
accumulator. M141's own milestone said the accumulation was untested; this is that sentence
paid off.

## The first battery round failed, and it was not this slice

Battery `m142` failed at 46 of 62 binaries on `per_kind_counts_tie_out_on_both_sides` in
`epsilon-session` — a crate this slice did not touch — with `os error 2` on a `fs::rename`
whose `fs::write` had just succeeded. The file was written, then absent. Same AV-quarantine
signature already recorded for this machine, and this time in a test's own `%TEMP%` scratch
directory rather than under the build target, so the external-`CARGO_TARGET_DIR` mitigation does
not cover it. Re-run on identical bytes: 5/5 pass.

The full record is in
[`../../audit/evidence/m142-failed-round.md`](../../audit/evidence/m142-failed-round.md). It is kept
in the repository rather than discarded, because an audit that quietly drops failed rounds is the
"false bill of health" the M134 ledger work removed. **Only `m142b` is cited as passing.**

## Limits — what this slice does not do

- **No test drives a real `/model` switch in a live session.** Test 6 simulates the
  situation (a reply arriving for a model other than the current one) by attributing the
  message correctly, but nothing switches `App::model_id` and then asserts the header's
  rendered string before and after. The composition of accumulation and rendering is still
  verified by the compiler and by reading.
- **`record_usage` remains private.** The tests reach it through `apply` and
  `hydrate_from_history`, which is the right way round — they exercise the public path — but
  it means a future refactor could add a third accumulation site and only a test that
  happens to drive it would notice.
- **A tool-result turn carries no usage and is not covered.** A tool result is not billed,
  so nothing should be recorded for it; no test asserts that explicitly.
- **Cache *write* attribution across models is untested.** The two-model tests vary input
  and output; a session where only cache traffic differs between models is the case that
  would slip, and it is the case where mispricing is largest.

## What nothing exercises

- **The header's rendered string for a two-model session.** `ui/metrics.rs` tests the
  fragments against hand-built `Metrics`; nothing renders a real accumulated two-model
  session and reads the `$…` fragment back.
- **The store round-trip.** `hydrate_from_history` is fed `Message` values directly. That
  those messages survive a write/read cycle of the session file with their `model_id`
  intact is a property of `epsilon-session`, and M139 touched adjacent code without pinning
  it. A resumed session whose messages lost their `model_id` would collapse to one row —
  the exact control failure above — with no test anywhere catching it.
