# M144 — negative control: the write before this slice

Captured, not composed. The control reverts `compact_now` to its pre-M144 behaviour: the
`Compaction` entry's `usage` set to `None`, and the `UsageAccount` append deleted.

## The first control passed, and that was the real finding

Reverting the write and removing the accounting append left **all 249 `epsilon-core` tests
green**:

```
test result: ok. 144 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 3 passed;  0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 4 passed;  0 failed; 0 ignored; 0 measured; 0 filtered out
```

Every M144 test I had written at that point exercised the **reader** — `usage_accounts()`
and `add_usage_accounts` — with hand-built rows. The reader was never given a chance to be
wrong about the writer, because nothing tested the writer. A fix that recorded nothing would
have passed every test in the slice.

That is a fourth distinct way a control can mislead, and it is the one this project has most
reason to care about:

| # | Control | Why it misled |
|---|---|---|
| M141 | implemented *different* code | double-counted instead of repricing |
| M142 | faithful but uninformative | its subject was agreement, not layout |
| M143 | applied to the wrong branch | the fixture took the other path |
| M144 | tested only half the slice | the reader was proven; the **writer** was not |

The general rule this adds: **a control must exercise the code the fix actually changed.**
A slice that adds a read path must also be controlled at the write path, or the two halves
can be independently broken.

## The test that closed it

`a_compaction_leaves_behind_a_bill_naming_the_model_that_was_charged` drives a real
compaction through the real agent with a store attached, then asks the store what it
recorded. Re-running the identical control:

```
---- a_compaction_leaves_behind_a_bill_naming_the_model_that_was_charged stdout ----

thread 'a_compaction_leaves_behind_a_bill_naming_the_model_that_was_charged' panicked at
  crates\epsilon-core\tests\compaction_scenarios.rs:788:5:
a billed compaction left no UsageAccount, so its cost is invisible

test result: FAILED. 10 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

**1 failed, 10 passed** — only the new test, with the failure naming the defect in the words
of the defect. The ten pre-existing compaction scenarios stay green, which is what makes it
a precise control: it removed billing, not compaction.

## A false RED, caught before it counted

The first attempt at this control also failed — and for the wrong reason:

```
the pressure trigger never fired, so nothing was billed
```

The fixture used 2,000-char echo turns where the neighbouring known-firing test uses 4,000,
so estimated tokens never crossed the 1,500 threshold and the run never reached the
assertion about billing at all. A test that fails before its subject is not evidence; it
would have been a "RED" that proved the threshold configuration was sensitive and nothing
else.

The fix was to copy the shape of `pressure_triggers_compaction_and_persists_entry` exactly —
4,000-char turns, a trailing `say`, the same limits — and to keep the trigger assertion in
the test so this specific false RED cannot recur silently.

## A guess the faux provider would have made meaningless

The test originally scripted a summarizer reporting `input: 9_000, output: 300` via a
`say_with_usage` constructor. **That constructor does not exist**: `FauxProvider` stamps a
fixed `Usage { input: 100, output: 50, .. }` in `blank()` (`faux.rs:167`) and offers no seam
to change it.

Asserting numbers the provider cannot produce would have produced a test that always
fails, and "fixing" it by inventing the constructor would have meant adding production API
pure to satisfy a test. The assertions now name the provider's real stamp, so the test
measures the pipeline rather than a fixture wish — and the slice adds no test-only API.
