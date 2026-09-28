# M106 — the cache-hit gauge measures one prompt, not a session

## What

`Metrics::cache_hit_rate` divided the **session-total** cache reads by the **latest
prompt** size. Its own doc comment claimed the opposite — "numerator and denominator are
deliberately from the same call" — and the data path made that impossible: `App::metrics`
filled `cache_read` from `UiState::cache_usage`, which accumulates, while `context_used`
came from the latest single call.

The consequence was not subtle once stated plainly. On the second turn the numerator
already contained the first turn's cache reads, so the ratio inflated; once cumulative
reads passed one prompt the `.min(prompt)` clamp pinned the gauge at `cache 100%` and it
stayed there for the rest of the session. A reader watching the number would conclude
the cache was perfect, always — which is worse than showing nothing.

Found while verifying the visibility feature the owner asked to be first-class. The
suite had a test named `cache_rate_is_about_the_latest_prompt_not_the_session`, and it
passed: it moved the denominator while holding the numerator fixed, which is a model of
the gauge no session can produce. A test can be named for a property and still certify
its absence.

## Key decisions

- **The pair, not the formula.** The fix is not a better division; it is carrying the
  right numerator. `UiState` now keeps `latest_cache_read` beside `latest_prompt_tokens`
  — the value was already in hand in both usage arms, it was simply added to a running
  total and thrown away. Dividing like for like then needs no comment to be correct.
- **Names that say which is which.** `Metrics::cache_read` became
  `cache_read_session`, and the new field is `cache_read_last`, each with a doc comment
  naming what it counts. The old name was one word for two different quantities, which is
  how the bug survived a doc comment that described the correct behaviour: a reader could
  believe the comment and never check the field.
- **The session totals stay.** They are the honest answer to "how much of this session
  came from cache", which a wider layout can show; deleting them to fix the rate would
  have removed information rather than corrected it.
- **The clamp stays, with its reason written down.** A provider that reports more cache
  reads than prompt tokens is contradicting itself; clamping to 100% keeps the gauge
  inside its range instead of printing a lie with more decimal places.

## Gotchas

- **The plumbing was the bug, not the arithmetic.** Every unit test of `cache_hit_rate`
  passed with the wrong numerator available, because the field was *named* as if it held
  the right thing. The test that catches this class has to drive two calls through
  `UiState` — which is why `the_cache_numerator_never_borrows_from_earlier_calls` asserts
  a session total deliberately larger than the prompt.
- **A rate scoped to "the session" and a rate scoped to "the last prompt" look equally
  plausible in a header.** The one the reader is watching is the second, because the
  question is "is the cache tracking *this* conversation".

## Verification

Battery `m106a` (shared with M107 and M108 — the same three files, one round, one
battery; `target/gate-m106a.log`, copied to
[`../../audit/evidence/gate-m106a.log`](../../audit/evidence/gate-m106a.log)): all five
gates exit 0, **1,021 passed / 0 failed / 1 ignored across 59 test binaries**.

`epsilon-tui` reports **217 passed, 0 failed** after this round, down from 220 before it
— the arithmetic of the round: −8 tests deleted with the dead footer (M107), +4 for
`git::branch`, +1 for the cache numerator here. The workspace total is two lower than
M105's 1,023 for exactly that reason.

Global `epsilon.exe`: **reinstall not required for this slice.** The gauge is drawn from
state the frontend computes; no CLI-visible behaviour changes. The round's reinstall
decision is recorded in M108.

## Limits

- **Not exercised against a live provider.** The fix is verified by unit tests over
  `Metrics` and by the reducer that fills it; no paid call was made, so the claim is
  "the number is computed from the right pair", not "the number matched a provider's
  invoice". The real-session shape (a cache hit that reports, say, 90% of the prompt) is
  not in the suite.
- **Cost still ignores cached tokens entirely.** `App::metrics` prices
  `tokens_in × input + tokens_out × output`, and `tokens_in` excludes cached tokens by
  the providers' own split, so cache reads and writes are priced at zero. On a
  cache-heavy session the estimate is therefore low — roughly by the cached share times
  the input price. The bundled catalog has no cache prices to use
  (`ModelInfo` carries input and output cost only), so fixing it means extending
  `scripts/regen_catalog.py`, the `ModelInfo` schema and the generated table. Recorded as
  G18 in [`../../audit/history/11-current-assessment.md`](../../audit/history/11-current-assessment.md)
  rather than guessed at.
