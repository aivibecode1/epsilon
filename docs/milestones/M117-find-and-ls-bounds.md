# M117 — the two tool defects inside G20's divergence

## What

The port re-check put "Epsilon registers twelve tools where pi's default active set is four"
on the gap list. Working out which part of that is a defect produced three answers, recorded as
G20 in [`audit/11`](../../audit/11-current-assessment.md): the **active set is a product
choice** (this harness's contract is its own, and removing capability to match a count would
make it worse, not more faithful), while **two things inside the row are bugs**:

- **`find` truncated silently.** It stopped at its cap and returned a list that looked
  complete. pi announces the cap (`tools/find.ts:281-285`); Epsilon's `grep` already announces
  its own byte cap in this crate, so the precedent and the wording both existed.
- **`ls` had no `limit`.** pi's takes one, defaults to 500 (`tools/ls.ts:13,23`) and says
  "N entries limit reached" (`:146-148`). Epsilon's took only `path`, so the tool had no bound
  it could report.

Both now behave like pi and say more than pi does about what is being withheld:

```
[2 results limit reached: 3 more file(s) match and are not shown. Use limit=4 for more, or refine pattern]
[2 entries limit reached: 3 more entries in this directory are not shown. Use limit=4 for more]
```

## Key decisions

- **Keep pi's wording, add the count.** pi's message says a limit was reached and how to get
  more; it does not say how much is missing. A reader deciding whether to raise the limit needs
  that number, and it is free here (`walk_files` already materialises the walk).
- **A limit under the cap says nothing.** Both notices are conditional, and both have a test
  asserting silence when nothing was withheld — a notice that appears when nothing was hidden
  trains the reader to ignore it.
- **The limit is part of the tool's schema, and the description says so.** `descriptions/ls.txt`
  now states the 500 default and the notice, because the model reads the description and a
  parameter it cannot see the effect of is one it will not use well.
- **`LS_DEFAULT_LIMIT` is exported and named by the test**, so the number lives in one place
  rather than being re-spelled in an assertion.

## Gotchas

- **A notice's second line broke an existing test's filter.** `find::tests::respects_limit`
  filtered output lines with `!l.starts_with('[')`, and the wrapped notice line starts with
  neither; the test now stops at the blank-line separator and additionally asserts the cap is
  exact (the first N shown, the rest absent) — which is what it was trying to say.
- **Advertised behaviour that does not exist is its own defect.** The subagent noticed that
  `find.txt` promises "1000 results or 64KB" while only the result count is enforced: the
  notice now makes the count half true and the byte half is still only claimed. Recorded in the
  Limits rather than quietly left.

## Verification

Battery `m117a`: all five gates exit 0. `epsilon-tools` 97 unit + 49 integration tests, four
new (`find_announces_the_results_its_limit_hides`,
`find_under_its_limit_says_nothing_about_a_limit`, `ls_announces_the_entries_its_limit_hides`,
`ls_under_its_limit_says_nothing_about_a_limit`).

**Negative controls, both fixes**, because each was reachable in its pre-fix form: with the
`find` notice disabled (`if total > shown` → `shown + 99`), `respects_limit` and
`find_announces…` both fail; with the `ls` notice disabled (`if limit_reached` →
`&& rows.len() > 99_999`), `ls_announces…` fails. Both restored, final run green.

## Limits

- **`glob` still has a silent cap** — the walker's own `WALK_MAX_ENTRIES` budget, outside the
  two defects this slice was scoped to. Worth its own row.
- **`find`'s description still promises a 64KB byte cap that no code enforces.** The notice
  makes the result-count half honest; the byte half is advertised, not implemented. Either
  implement it or take it out of the description.
- **`ls` has no per-entry byte cap** where pi has `DEFAULT_MAX_BYTES`; only `limit` was added.
- **The active-set divergence is unchanged and deliberate** — see G20. The prompt-token cost of
  twelve tool descriptions is real and still unmeasured.
