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

## The measurement had a hole in it, and finding it was worth more than the cache

The first version of this milestone concluded that the two remaining per-frame costs —
`agent.goal()` and the sessions pane — were "real and now *small*", and left them alone.

**That conclusion was wrong, and the reason it was wrong is the interesting part.** The frame
benchmark seeds the window's **view model**. `agent.goal()` reads the **durable store**. So
every number in the table above was measured against a store this benchmark never wrote, and
the cost of reading the goal was measured on an almost-empty file.

Measured properly — a real `SessionStore`, real appended messages:

| stored messages | `agent.goal()` | ×2, as the window calls it |
|---|---|---|
| 50 | 3.3 ms | 6.6 ms/frame |
| 400 | 30.5 ms | **60.9 ms/frame** |
| 1600 | 102.3 ms | **204.6 ms/frame** |

**At 400 messages — an ordinary session — reading the goal twice cost more than the entire
transcript did.** `SessionStore::goal()` walks `read_all()`, which deep-clones every entry, and
it does that **under the mutex the agent's own append path takes**: not a cost, a stall.

So the honest sequence is: measure, find a hole in the measurement, fix the hole, measure
again. And the second half of this milestone is that fix.

### The goal is cached, refreshed where it can change

`Agent::goal()` is now read into `IdeApp::goal` and refreshed at exactly two places:

1. **a frame that applied events** — `pump` re-reads only when `drain` returned something, so
   this is per *event batch*, not per frame. Every event is a way the model can have changed
   the goal, since `create_goal` and `update_goal` ride inside a turn.
2. **`run_goal_command`** — the palette's own verbs.

Both are named on the field, and the field documents that a caller reaching into the agent
directly must call `refresh_goal()` itself. The three existing goal fixtures do exactly that,
and each now says so.

`refresh_goal` is **public** for that reason, not for testing convenience: the alternative was
to invent a `*_for_test` name for something the product legitimately does.

**The test that matters is the one that does not call it.** The three fixtures set a goal up on
the agent and refresh by hand; `a_goal_created_from_the_palette_is_on_screen_on_the_next_frame`
goes the way a reader does — palette, then a frame — so if the refresh inside `run_goal_command`
were ever lost, the band would simply never appear and nothing else in the crate would notice.

## What the remaining transcript cost actually is

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

Battery `m266b` — **GATE PASS: 1,883 tests across 77 binaries, 0 failed**, all seven gates,
**attempt 1** (`audit/evidence/gate-m266b.log`). Up from 1,879 at M265: three cache tests, one
palette-refresh test, and one new binary (`frame_cost.rs`, whose two tests report as ignored by
design). `m266` was the earlier battery for the first half of this milestone; both are kept.

Tests, and why each is the one it is:

- `a_goal_created_from_the_palette_is_on_screen_on_the_next_frame` — the staleness guard. It
  refreshes nothing by hand, so it is the only test that can catch the product's own refresh
  going missing; the other three goal fixtures call `refresh_goal()` themselves and would pass
  with the product broken.
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
- **The sessions pane still re-summarises every file on every paint.** Measured at 0.9 ms for
  fifty files, so it is left alone deliberately — but the *estimate* beside it (202 ms at 500)
  should not be cited again as if it were a measurement.
- **These are offscreen numbers.** Nothing below tessellation has been measured here (A25), and
  a real window adds a present step these frames do not.