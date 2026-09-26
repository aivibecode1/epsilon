# M141 — negative control: the session bill was priced at the wrong model's rates

Captured, not composed. The control replaces `breakdown_per_model`'s body with the old
behaviour exactly: sum every model's token counts, then price the sum **once**, at the
rates of whichever model is selected at render time.

```
test ui::cost::tests::a_single_model_session_is_priced_exactly_as_before ... ok
test ui::cost::tests::a_session_with_no_usage_yet_is_zero_and_not_a_floor ... ok
test ui::cost::tests::a_model_that_costs_nothing_is_not_a_floor ... ok
test ui::cost::tests::every_bucket_is_charged_at_its_own_price ... ok
test ui::cost::tests::cached_tokens_are_not_priced_at_the_input_rate ... ok
test ui::cost::tests::the_sum_is_divided_once_so_small_components_do_not_vanish ... ok
test ui::cost::tests::an_unpriced_model_makes_the_total_a_floor_not_a_smaller_bill ... FAILED
test ui::cost::tests::the_total_does_not_depend_on_which_model_is_visited_first ... FAILED
test ui::cost::tests::a_switch_does_not_reprice_the_turns_that_came_before_it ... FAILED
test ui::cost::tests::the_old_pricing_would_have_overstated_this_session ... FAILED

thread 'a_switch_does_not_reprice_the_turns_that_came_before_it' panicked at
  crates\epsilon-tui\src\ui\cost.rs:274:9:
assertion `left == right` failed: each turn at its own model's price, not both at the current one
  left: 9000000
 right: 49500000

thread 'the_old_pricing_would_have_overstated_this_session' panicked at
  crates\epsilon-tui\src\ui\cost.rs:305:9:
assertion `left == right` failed: 3.00 + 1.50 + 30.00 + 15.00
  left: 9000000
 right: 49500000

thread 'an_unpriced_model_makes_the_total_a_floor_not_a_smaller_bill' panicked at
  crates\epsilon-tui\src\ui\cost.rs:328:9:
assertion `left == right` failed: only the priced model's tokens contribute to a known total
  left: 27000000
 right: 4500000

thread 'the_total_does_not_depend_on_which_model_is_visited_first' panicked at
  crates\epsilon-tui\src\ui\cost.rs:354:9:
assertion `left == right` failed
  left: 9000000
 right: 49500000

test result: FAILED. 10 passed; 4 failed; 0 ignored; 0 measured; 300 filtered out
```

## Reading it

**4 failed, 8 passed** — and the split is the evidence. The four that fail are exactly the
multi-model tests. Every single-model test survives, which is the point: a control that
broke the whole module would prove only that the module had been replaced, not that these
tests detect *this* defect.

`left: 9000000, right: 49500000` is the harm in one line. Under the control, a session that
spent one turn on a $3/Mtok model and one on a $30/Mtok model reports **$9.00** for work
that actually cost **$49.50** — an 82% under-report, and nothing on screen distinguishes it
from a correct figure. The direction of the error follows the direction of the switch:
moving to a *cheaper* model makes your session look cheap, moving to a *pricier* one makes
past work look expensive. Neither is visible.

The third failure is a different property and worth reading separately: under the control,
a model the registry cannot price is silently swallowed, so `an_unpriced_model…` sees
$27.00 instead of $4.50 and — more importantly — the total is no longer marked as a floor,
because the old path only asked about the one selected model. An unknown-price model would
have made the bill look both larger and complete.

## A control worth recording

The first attempt at this control was wrong in an instructive way. It priced *every* model's
tokens at the *first* model's rates inside the per-model loop, which double-counted rather
than repricing, and only one test noticed. The rerun was needed to make the control faithful
to the code it replaces. A control that is merely *different* from the old code proves
nothing; it has to be the old code.
