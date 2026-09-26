# M141 — a session's bill is priced per model, not at the current one

**Status:** fixed. Battery `m141b`: **`GATE PASS`, 62 test binaries, 1,216 passed, 0 failed, 1
ignored** — up from 1,210 at M140, and the delta is exactly these 6 tests. Negative control at
[`../../audit/evidence/m141-negative-control.md`](../../audit/evidence/m141-negative-control.md).

M140 made each turn's *token counts* correct. This makes them *priced* correctly, which is
the second half of the same promise: a token's price is a property of the model that
consumed it, and Epsilon was charging every token in the session at whatever model happened
to be selected when the header drew.

## The defect

`App::metrics` did this:

```rust
let info = lookup_model(&self.model_id);        // the model selected NOW
let counts = TokenCounts { input: tokens_in, ... };   // the sum over ALL models
breakdown(counts, prices_from(info))
```

Sum first, price once. So `/model` silently repriced the entire session's history the
instant you switched — and `/resume` did the same, re-accumulating replayed history that
had been billed at other prices.

The accounting could not do better, and that is the real diagnosis: `UiState` kept four bare
`u64` totals (`tokens_in`, `tokens_out`, `cache_read`, `cache_write`). Once summed, the
information about *which model consumed which tokens* is gone. Nothing downstream could
recover it.

**The direction of the error follows the direction of the switch.** Moving to a pricier
model inflates the apparent cost of work already done; moving to a cheaper one deflates it.
Neither is visible on screen, and neither is signalled.

## The fix

Three parts, each where the information still exists:

1. **`UiState` keeps the split.** A `BTreeMap<String, TokenCounts>` alongside the flat
   totals, filled from `a.model_id` — the id on the message that produced the response, not
   the currently-selected one. Both accumulation sites (live `MessageEnd` and the
   `hydrate_from_history` replay) were updated, because `/resume` is where the multi-model
   case actually arises.
2. **`cost::breakdown_per_model`** prices each model's counts at that model's own rates and
   sums the components, dividing once at the end.
3. **`App::metrics`** looks up prices per model id instead of once per session.

The flat totals stay, and still feed the header's token counters (`↑12k ↓3.4k`) — those
genuinely only need the sum. Only the *bill* needs the split.

One lookup stayed keyed on `self.model_id` deliberately: `context_window`. A window is a
capacity, not a bill, so "the context window of the model you are talking to" is the right
answer even for a session that used several.

## Two judgement calls

**An unpriced model makes the total a floor.** If the registry has no row for a model that
produced tokens, those tokens are real money of unknown amount. The alternative — pricing
them at zero — makes the bill *smaller* and looks complete, which is the exact failure this
module exists to prevent. So it sets `floor`, and the header prints `$…+`.

**Summing per-model components, dividing once.** `breakdown` already works in a scale where
per-model rounding cannot lose a micro-dollar, and one final division means the total cannot
depend on iteration order. There is a test for that specifically, because a future
`HashMap` would otherwise be free to change a number the user sees.

## Tests — six, in `ui/cost.rs`

1. `a_single_model_session_is_priced_exactly_as_before` — the overwhelmingly common case
   must be untouched, or the fix is not worth its complexity.
2. `a_switch_does_not_reprice_the_turns_that_came_before_it` — the defect.
3. `the_old_pricing_would_have_overstated_this_session` — states the old number next to the
   right one.
4. `an_unpriced_model_makes_the_total_a_floor_not_a_smaller_bill`
5. `the_total_does_not_depend_on_which_model_is_visited_first`
6. `a_session_with_no_usage_yet_is_zero_and_not_a_floor`

**Negative control:** the body replaced with the old sum-then-price-once path — **4 failed,
8 passed**. The four failures are exactly the multi-model tests; every single-model test
survives, which is what makes the control informative rather than merely destructive. The
control reads `left: 9000000, right: 49500000`: **$9.00 reported for $49.50 of work.**

That control took two attempts, and the first is recorded in the evidence file. It priced
every model's tokens at the first model's rates *inside* the loop, which double-counted
rather than repricing, and only one test noticed. A control has to be the old code, not
merely different code.

## A test I wrote wrong

`the_old_pricing_would_have_overstated_this_session` first asserted the old total was
$150.00. It is $90.00: `breakdown` works in micro-USD-per-Mtok × tokens, so 2M input at 30
is $60.00 and 200k output at 150 is $30.00. The failure was the test's arithmetic, not the
implementation — worth recording because the corrected version is *more* useful: $90.00
against a true $49.50 is an 82% over-report, which says something specific, where a tidy
3× ratio would have said only "bigger".

## The first battery round failed

**`m141` did not reach the gates.** Restoring the implementation after the negative control
left a duplicate closing brace at `cost.rs:197`, and the battery caught it at the *first*
gate (`fmt --check`) as `error: unexpected closing delimiter`. Only `m141b`, run against the
corrected file, is cited as evidence.

It is worth noting what this failure is and is not. The control itself had already passed
against that same file, because the control runs through `cargo test`, which parses the
crate — so a stray brace could not have been sitting in the source during the control run
either; it was introduced by the restore step that came after. The lesson is the one this
repo has already learned twice: **a control run and a battery run are separate facts about
separate byte states, and evidence only covers the bytes that were actually built.**

## Limits — what this slice does not do

- **`UiState::record_usage` is private and untested directly.** The per-model map is
  populated correctly by inspection and by the fact that `App::metrics` compiles against
  it, but no test feeds two models' worth of `MessageEnd` events through a real `UiState`
  and reads the map back. The pricing logic is tested; the *accumulation* is not.
- **Cache buckets move with the model too**, and are carried in the same map, but no test
  has a two-model session where only cache traffic differs. A provider that serves a
  cached prefix under one model and not the next is the case that would slip.
- **No `/cost` command.** `CostBreakdown`'s per-component accessors have no UI caller, so
  the user still cannot ask *where* the money went — only how much. That is the natural
  follow-up now that the total is trustworthy.
- **An alias or a custom model id** in a resumed session finds no catalog row and therefore
  marks the total as a floor. Correct (it is a lower bound) but possibly surprising; a
  session using a custom model now shows `$…+` where it used to show a number derived from
  the selected model.

## What nothing exercises

- **The real `/model` switch in a live session.** No test switches models mid-session and
  asserts the header's rendered string before and after. The pricing function is tested
  directly; the wiring through `App::model_id` → `metrics()` is verified by the compiler.
- **A resumed multi-model session.** This is the case the fix most improves and the one
  with no end-to-end coverage: `hydrate_from_history` must read each message's own
  `model_id`, and nothing pins that it does.
- **Three or more models.** The tests use two. `BTreeMap` makes the arithmetic obviously
  order-independent, but "obviously" is not "tested".
