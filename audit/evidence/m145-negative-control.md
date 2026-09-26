# M145 — negative control: the bill on the entry that incurred it

Captured, not composed.

## The control

Restore M144's shape: the `Compaction` entry's `usage` and `model_id` set to `None`, and the
bill appended as a **separate second entry** afterwards — the arrangement F23 rejects.

```
---- a_compaction_records_its_bill_on_its_own_entry stdout ----

thread 'a_compaction_records_its_bill_on_its_own_entry' panicked at
  crates\epsilon-core\tests\compaction_scenarios.rs:789:5:
a billed compaction recorded nothing, so its cost is invisible

test result: FAILED. 10 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

**1 failed, 10 passed.** Only the new test, with the failure naming the defect, and the ten
pre-existing compaction scenarios green — precise, since the control removed billing and not
compaction.

## Three wrong expectations before that, all mine

Getting to a trustworthy control took three attempts, and the failures are worth recording
because in every case **the code was right and the test was wrong**.

**1. `left: 200, right: 100`.** The faux provider stamps usage in `blank()` (`faux.rs:167`),
and I read that as a constant 100/50. It is not: a summarizer's prompt carries the whole
shadowed history, so the real stamp is larger. Asserting `blank()`'s constants was a fixture
wish, not a measurement.

**2. `left: 100, right: 50`, then `left: 100, right: 200`.** After fixing input I still had
output wrong, and then I compared the reader's total against a *single entry's* usage. But
the reader returns a **sum** and an entry holds **one bill** — the pressure threshold fires
twice in this fixture, so 100 + 100 = 200. The invariant is not equality between reader and
entry; it is that the reader's total equals the sum over entries. The test now asserts that,
which is both true and the property actually worth having.

That one is worth spelling out, because it is a distinct failure mode from the other two: the
first two were *fabricated expectations*, and this one was a **category error** — comparing an
aggregate to a component and calling the disagreement a bug. A test that says `a == b` should
be asked what `a` and `b` are *made of*, not what their values are.

## What the control proves that M144's could not

M144's control (reverting the write) left all 249 `epsilon-core` tests green, because every
test in that slice exercised the reader with hand-built rows. This control is aimed at the
*same* code as M144's — `compact_now`'s append — and this time something goes red, because a
test now drives a real compaction and asks the store what it recorded.

The difference is entirely in what the tests reach. M145's test exercises the production path
end to end: real agent, real `compact_now`, real store, real reader. M144's tests never left
the reader.

## The structural assertion

The test also asserts that **no separate accounting entry exists**:

```rust
assert!(
    !kinds.iter().any(|k| k == "usage_account"),
    "a separate accounting entry is exactly the shape F23 rejects: {kinds:?}"
);
```

That is the regression guard for F23 itself. F23 was not "a bug in the accounting entry" — it
was the *existence* of that entry next to the compaction, because it made a bill the file's
last entry and therefore reachable by a positional rollback. A test asserting only that a bill
is counted would pass again the moment someone reintroduced a second entry, which is exactly
how M144 shipped something that F23 then had to reject.

## The restore

After capturing the output, the fix was restored in
`crates/epsilon-core/src/lib.rs` — the `None`s reverted to `Some(answer.usage)` /
`Some(model.id.clone())` and the separate append removed. `git diff` on that file afterwards
carries only M145's real change and **no `CONTROL` marker**, and the battery that followed
rebuilt the file (`Compiling epsilon-core` in `target/gate-m145c.log`), which is the check
that matters: a `Copy-Item`-style restore preserves the backup's mtime, cargo skips the
rebuild, and the *control* binary silently runs green. That cost two rounds on 2026-09-21 and
2026-09-22 before it was written down.
