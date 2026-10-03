# M275 — a measurement that stopped a large change aimed at the wrong phase

No product code changed in this milestone. That is the point.

## The question

M266 measured the desktop transcript at **104 ms per frame** for 800 assistant rows and
established that cost tracks **blocks**: the same rows cost 13 ms as plain labels, 32 ms with one
markdown block each, 104 ms with six.

That says the expensive thing is per-block. It says nothing about *which* per-block step, and
the fix is completely different for each:

| phase | who owns it | fix if it dominates |
|---|---|---|
| `markup::render` | ours | already cached (M266) |
| `markup::job` + `plain_of` | ours | cache the built job — small, safe |
| `ui.label(WidgetText::from(job))` | **egui's** | virtualise the transcript — large, risky |

`where_the_transcript_milliseconds_go` times the two we own, over the same 800 answers, outside
the frame:

```
  plain_of         (one String per block, per frame)        1.331 ms  <- paid
  markup::job      (one LayoutJob per block, per frame)     3.588 ms  <- paid
  OUR WORK                                                     4.919 ms  of the frame
  EGUI'S OWN       99.481 ms  (95% of it): text layout and shaping, per block, every frame
```

**Caching the jobs would buy 4.7%.** Ninety-five percent is egui laying out and shaping every
block of every row, every frame, whether or not the row is on screen — and the only thing that
addresses that is not building the widgets.

## What that bought

Without this, the two plausible next moves were:

* **cache the `LayoutJob`s** — a plausible-looking afternoon's work for **under 5%**, and it
  would have looked like progress because the frame did get faster;
* **virtualise the transcript** — the right fix, but a large change into a variable-height
  scroll area, entered without knowing that the cheap alternative was worthless.

The measurement picked the second and **priced the first out**. That is the same discipline as
M269's false finding and M274's blind instrument, applied to performance: *measure the phase
before choosing the fix.*

## The numbers that are deliberately not quoted

`markup::render` raw swings between **33 ms and 56 ms** across runs on this debug build. It
is printed because it is the obvious question, and labelled `RAW — the app caches it since
M266` because **the app does not pay it** and quoting it as a cost would be wrong. Only the two
figures marked `paid` are part of the frame, and the remainder is computed from the frame total
in `what_a_frame_costs` — which is why that number is a named constant here, so the two
measurements cannot quietly disagree.

## Verification

Battery `m275` — **GATE PASS: 1,909 tests across 77 binaries, 0 failed**, all seven gates,
**attempt 1** (`audit/evidence/gate-m275.log`). The count is **unchanged from M274**, and that
is the honest signal: no product code changed. The bench is `#[ignore]`d and prints, like its
two siblings, so no number in this milestone is a gate.

A side effect worth keeping: `FRAME_MS` in the new bench is the whole-frame figure from
`what_a_frame_costs`, as a named constant. The two measurements now **cannot quietly disagree**
— if one moves and the other does not, one of them is wrong and the arithmetic says which.

The bench is `#[ignore]`d and prints, like its two siblings, so no number here is a gate.

## What nothing exercises

- **Virtualisation is still not built**, and this milestone does not start it: it needs cached
  per-row heights, because egui's `show_rows` requires a uniform height and a transcript's rows
  are not uniform. That is a real change and it is now the *only* change that addresses the
  104 ms.
- **The 104 ms figure is a debug build, offscreen**, with no window and no present step. It is
  a relative instrument — the ranking of the phases is what it is for — and not a prediction of
  what a user's window costs.