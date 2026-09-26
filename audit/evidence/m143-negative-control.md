# M143 — negative control: the cost breakdown line

Captured, not composed. The control removes the `cost_summary` notice from the `/session`
handler — the state before this slice, where `CostBreakdown`'s four per-component accessors
had no caller anywhere in the workspace.

```
test app::tests::session_command_shows_where_the_money_went_not_just_how_much ... FAILED
test app::tests::session_command_reports_cache_attribution_beside_the_totals ... ok

test result: FAILED. 320 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

One test fails, and it is the right one. The sibling `/session` test — the one asserting
cache attribution — still passes, which is the point: the control removed a *line*, not the
command, so a control that broke every `/session` assertion would have been proving that
the command works rather than that this feature is detected.

## The first control attempt passed, and why that mattered

The first attempt removed the call from the **store-backed** branch of `Command::Session`,
and all 321 tests passed. That looked like a weak test, and it was — but the actual fault
was in the control, not the test.

`/session` has two branches: one for an agent with a durable session file, one for an agent
without one. The test's fixture (`test_app()`) builds an agent with **no** store, so it
takes the second branch, and I had removed the line from the first. The control removed
code the test never executed.

This is a third distinct way a control can be wrong, and it is worth writing down next to
the other two:

1. **M141's** control was *different* code rather than the *old* code — it double-counted
   instead of repricing.
2. **M142's** control was faithful but uninformative about one property by design.
3. **M143's** control was applied to the wrong branch — it exercised a path the test does
   not take, so it "passed" and proved nothing.

The general rule: **a control must be applied where the test's execution actually goes.**
Finding that out required reading the fixture (`test_app` builds a store-less agent), not
re-running the test more times. Re-running would eventually have produced the same
green result with the control still in place.

That is the real lesson, and it is stronger than "add more assertions": a control that does
not fail has told you something about **where your test runs**, which is information you
did not have before. The second control — same edit, applied to the branch the fixture
takes — failed immediately, which confirmed both the test's reach and the control's
fidelity.
