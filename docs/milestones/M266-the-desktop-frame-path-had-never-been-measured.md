# M266 — the desktop frame path had never been measured, and it is not what the review said

## The measurement, first

M252 established that **no wgpu backend presents on this machine**, so every claim about what
the window *looks like* is a hypothesis. What had never been established was a claim about how
fast it *draws* — and that turned out to be the more useful of the two to know.

So: painted through the real `paint` at 1440×900, offscreen egui, 20 frames each after one
warm-up. It lives at `crates/epsilon-desktop/tests/frame_cost.rs` and is `#[ignore]`d, for the
same reason the terminal's `frame_dump` is: timings on one machine would be a flaky gate
anywhere else, and the value is in running it when the draw path changes.

| transcript | before | after |
|---|---|---|
| empty | 0.9 ms | 0.9 ms |
| 50 assistant rows, rich markdown | 11.5 | 8.5 |
| 200 assistant rows, rich markdown | 41.3 | 27.0 |
| **800 assistant rows, rich markdown** | **149.9** | **104.4** |
| 800 assistant rows, one block each | — | 31.2 |
| 800 plain rows, no markdown path | — | 13.2 |
| 800 rows + **50 session files**, Sessions pane | — | **+0.9** |

## What it said, and why that matters more than the fix

**The review ranked the sessions pane first, and it costs 0.9 ms.** The project carries a
recorded estimate for it — 202 ms at 500 sessions, `epsilon-session/src/discover.rs:296-298` —
and fifty files measured at *under a millisecond*. The transcript, which the review ranked
fifth, costs **103 ms**.

So the highest-value thing this milestone did was **not fix the sessions pane**. It measured
first, which is the one step that costs nothing and the one every honest estimate in the
repository would have benefited from. The ledger's ranking was a claim; this is a number, and
the number disagrees.

The estimate itself is not wrong, it is extrapolated — and extrapolation is exactly what a
measurement is for.

## What the remaining cost actually is

The cache bought 20–30%, which is real and is **not the bulk**. Attribution says the cost
tracks **blocks**:

| 800 rows, same count | ms/frame |
|---|---|
| plain labels (no markdown path) | 13.2 |
| one markdown block each | 31.2 |
| the fixture's rich markdown (~6 blocks) | 104.4 |

A markdown row costs roughly ten times a plain one, and the multiplier tracks its **block
count** — not its length, not its parse. **The transcript builds a text widget for every block
of every row, on screen or not.**

That is the finding, and the fix it names is *not* in this milestone. The file tree two
functions away already virtualizes (`workspace.rs` uses `show_rows`, with a comment saying
virtualization is "not optional"); the transcript cannot, because egui's `show_rows` requires
a uniform row height and a transcript's rows are not uniform. Making it virtual means caching
per-row heights, which is a layout change with its own correctness surface — and the number
above is the thing to size it against, which is the point of having measured first.

## The cache, and why its key is the text

`markup::Cache` maps the **exact source string** to its parsed blocks.

An id-keyed cache would be faster to look up, and it would need every site that rewrites a
row's text to invalidate it — five of them, including the streaming ones. One that forgets is
a transcript showing yesterday's answer, and nothing in the suite would catch it. Keying by
the text cannot go stale: **the key changes precisely when the content does.** Hashing a few
hundred bytes is nanoseconds against about 185 µs to parse them.

Bounded at 512 entries and cleared wholesale rather than evicted LRU: this is a cache, not a
second copy of the transcript, and an LRU's bookkeeping would cost more than the parses it
saves at this size.

It is a `RefCell` field because the transcript draws inside a `ScrollArea::closure` that
borrows `self` — a `&mut self` field would be a self-referential borrow, and a `Mutex` a lock
taken once per assistant row for one thread's private cache. `Cache::blocks` hands back an
`Rc`, so the borrow ends on the call.

## Verification

**PENDING** — battery not yet run this round.

Battery `m266` — **GATE PASS: 1,882 tests across 77 binaries, 0 failed**, all seven gates,
**attempt 1** (`audit/evidence/gate-m266.log`). Up from 1,879 at M265: three cache tests, and
one new binary (`frame_cost.rs`, whose single test reports as ignored by design).

Three tests, because the dangerous failure here is invisible to correctness:

- `the_cache_parses_once_per_distinct_text` — asserts the **miss count**, not just the answer.
  A cache that stopped being consulted returns *correct* blocks forever, and every frame test
  in this crate would still pass. **Negative control: with the lookup disabled it fails with
  "the same text must not be parsed twice".**
- `a_different_text_of_the_same_length_is_not_served_from_the_first` — the obvious way to get
  a cheap key wrong is to key on length or row index, so two answers of the same size share a
  slot and the second wears the first's styling.
- `the_cache_is_bounded_and_still_correct_when_it_empties` — emptying changes nothing.

## What nothing exercises

- **The transcript is still O(rows), not O(visible).** At `MAX_ROWS` of 2,000 the window would
  paint at roughly four frames a second. That is the honest state of it and it is measured,
  not estimated.
- **Virtualization is not built**, for the uniform-height reason above.
- **`Agent::goal()` still deep-clones every session entry twice per frame**
  (`app.rs:1905`, `:725`), and the sessions pane still re-summarises on every paint. Both are
  real and both are now *small* — which is itself the finding, and the reason they were left
  alone this round.
- **These are offscreen numbers.** Nothing below tessellation has been measured here (A25), and
  a real window adds a present step these frames do not.