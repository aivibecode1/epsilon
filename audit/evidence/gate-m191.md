# Gate `m191` — the date conversion, measured

**Verdict: GATE PASS.** 63 targets, **1,449 results, 0 failed.**

| Stage | Exit | Time |
|---|---|---|
| `fmt --check` | 0 | 2s |
| `clippy` | 0 | 5s |
| `test` | 0 | 225s |
| `demo` | 0 | 5s |
| `real-check` | 0 | 1s |

- **Log:** `audit/evidence/gate-m191.log`
- **Source commit:** `d015d10`
- **Baseline:** M190B — 1,447 results, 63 targets
- **Delta:** **+2**, `epsilon-cli --lib` (114 → 116)

## What this milestone added

Two tests for `epoch_millis_to_iso8601`, which M190's own milestone recorded as
*"reasoned, not measured"*.

1. **`iso8601_conversion_is_correct_at_the_calendar_boundaries`** — the epoch, one second
   past it, `2000-02-29` (divisible by 400 → leap day), `2100-01-01` (divisible by 100 but
   not 400 → *not* a leap day), and the last millisecond of 2026.
2. **`every_iso_timestamp_round_trips_through_its_own_fields`** — parses the emitted
   string's fields back and re-derives the millisecond count. This is the test that earns
   its place: a format assertion (`len == 24`, `ends_with('Z')`) is satisfied by a string
   carrying the wrong instant, and pi's `getTree()` sorts children with
   `new Date(entry.timestamp)` — so a wrong string is a client-side throw.

## Verification outside the suite

**200,000 random instants** across the `u64` millisecond range, compared against Python's
`datetime.fromtimestamp(ms/1000, tz=utc)`: **0 mismatches.** This includes both century
cases above.

## Two hand-computed expected values were wrong

The 2026 year-end millisecond in the Rust test was off by **14,600,000 ms** — four hours
and one second — because I did the year arithmetic in my head rather than deriving it. An
earlier scratch computation made the same mistake for a different instant. **The function
was correct both times**, and a test built on a guessed expectation would have "proved" the
implementation broken. That is now a comment in the test file.

## Controls

| Control | Result |
|---|---|
| **F3** — `+1` day for November and December (`mp >= 10`) | **2 failed**: `left: "2000-02-30"` (a date that does not exist) and a round-trip off by exactly `86400000` ms |
| **F** — century-year branch on a condition no test instant satisfies | passed — **control inert** |
| **F2** — rewrites only the day number for century years, where all three test instants fall on the 1st | passed — **control inert** |

**The two inert controls are the notable part.** This is the third time a passing control
turned out to be a no-op rather than a success (after billing at `self.model_id()`, and
`get_tree` on a linear session). F was caught by reading the branch condition; F2 was
caught by running the mutated function standalone in Python and diffing the strings — they
were identical. The rule now in memory: **a control that passes is a control you have not
yet proven works.**

## Range limit found while writing the test

`1900-01-01` is `-2208988800000` ms — a negative value, and the parameter is `u64`. The
range starts at the Unix epoch. That is honest rather than a defect: the store's clock
starts when the process does, so no session entry predates 1970. Documented on the
function, because *trying to write the boundary test* is what made the limit visible.
