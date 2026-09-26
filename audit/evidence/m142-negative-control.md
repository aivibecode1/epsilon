# M142 — negative control: the per-model accumulation

Captured, not composed. The control changes one line in `UiState::record_usage`: instead of
keying the row by the message's own `model_id`, it keys every turn under the constant
`"collapsed"`. That is the pre-M141 state — the model that produced a turn was not recorded
at all — and it is the exact thing the M141 fix had to introduce.

```
test state::tests::each_turn_lands_under_the_model_that_produced_it ... FAILED
test state::tests::a_second_turn_on_one_model_adds_to_that_models_row ... FAILED
test state::tests::the_flat_totals_still_equal_the_sum_of_the_rows ... ok
test state::tests::a_resumed_session_keeps_each_messages_own_model ... FAILED
test state::tests::resuming_replaces_the_previous_sessions_rows ... FAILED
test state::tests::a_reply_lands_on_its_own_model_even_after_a_switch ... FAILED

thread 'each_turn_lands_under_the_model_that_produced_it' panicked at
  crates\epsilon-tui\src\state.rs:1253:9:
assertion `left == right` failed: two models, two rows:
  {"collapsed": TokenCounts { input: 3000, output: 300, cache_read: 0, cache_write: 0 }}
  left: 1
 right: 2

thread 'a_resumed_session_keeps_each_messages_own_model' panicked at
  crates\epsilon-tui\src\state.rs:1322:9:
assertion `left == right` failed: a resumed two-model session must not collapse into one row:
  {"collapsed": TokenCounts { input: 5000, output: 500, cache_read: 0, cache_write: 0 }}
  left: 1
 right: 2

thread 'resuming_replaces_the_previous_sessions_rows' panicked at
  crates\epsilon-tui\src\state.rs:1346:9:
assertion `left == right` failed: the old row must be gone, not merged in:
  {"collapsed": TokenCounts { input: 500, output: 50, cache_read: 0, cache_write: 0 }}
  left: ["collapsed"]
 right: ["new-model"]

test result: FAILED. 315 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out
```

## Reading it

**5 failed, 1 passed.** The failure messages print the map itself, so the control explains
itself without commentary: two turns on two different models produce **one** row holding
their combined 3,000 tokens, and a resumed two-model session produces one row holding
5,000. That is not a subtle numeric drift — it is the information the whole pricing fix
depends on, visibly gone.

The survivor is the interesting one, and it is a property of how the test was written rather
than luck. `the_flat_totals_still_equal_the_sum_of_the_rows` compares the flat total against
the sum of the rows, so collapsing every row into one leaves both sides equal and it still
passes. That is correct: the test is asserting that the token counters and the bill agree,
and under the control they *do* agree — they are just both wrong about which model was
involved. A test that had asserted something more specific about the rows would have failed
here too, and would have been testing the row layout rather than the property that matters.

## Why this control is worth the run

M141 shipped six tests, all of them inside `ui/cost.rs`, all of them calling
`breakdown_per_model` with a hand-built `BTreeMap`. None of them could catch a bug in
`record_usage` — the code that *builds* the map. A pricing function can be perfect and the
accumulation feeding it wrong, and M141's evidence genuinely did not cover that.

This control is the demonstration that the new tests reach the other half. It is a smaller
and cheaper fix than M141 with no user-visible change on its own, but it closes the last
named gap in the cost chain: **the numbers are now correct, and now we can prove it.**
