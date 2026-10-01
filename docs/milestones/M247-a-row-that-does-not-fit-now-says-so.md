# M247 — a row that does not fit now says so

The sidebar drew its rows with `ui.colored_label`, which **clips silently**. A task
description, a model id, a session name that overflowed the 264-point pane simply stopped,
with no ellipsis and no marker — and on screen that is indistinguishable from the whole value.

## Why a clipped value is not a cosmetic problem

The two readings are opposites and both are wrong:

- A reader who believes a **truncated model name** is the full one is misled about *what
  actually ran* — which is exactly what the tasks pane exists to tell them.
- A reader who believes a **truncated description** is complete is misled about *what the
  agent is doing*.

This is the rule the rest of this repository keeps arriving at from different directions:
**a partial value must say it is partial.** The diff viewer appends `… (truncated)` for exactly
this reason, the composer refuses a blank goal objective, and the palette moves a colour until
it clears its contrast floor rather than shipping one that is invisible. A sidebar row was the
place the rule had been forgotten.

## The budget is an approximation, and deliberately one-sided

`fit()` assumes an average glyph advance of 0.55 ems. That is within about a fifth for
proportional text and wrong for a run of narrow glyphs, which is acceptable for one reason
only: **the result can only ever be shorter, never longer.** The budget is deliberately
conservative, so the worst case is a row a little shorter than it could have been. A budget
that could drop a readable character would not be acceptable, and that is the property the
test pins across six widths and seven lengths.

The cut is on a **character** boundary, so a row never ends in the replacement character a
byte slice would leave behind — asserted with Japanese text, which is where a byte-based
elision would visibly break.

## Applied where a single line can overflow

The tasks pane's description and its detail line, the session list's name and its count line.
The transcript is **not** elided: it is wrapped, and a wrapped line is visibly a wrapped
line. The editor is not elided either — it is monospaced and its gutter means a cut column
would be obvious.

## Verification

**Battery `m247` — GATE PASS on attempt 1, 1,794 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m247.log`). Up from 1,790: four tests.

The frame-level one is the important one. `fit()` being unit-tested would prove only that a
function works; `a_long_session_name_is_shown_as_cut_rather_than_clipped` opens a session
whose name cannot fit, paints the pane, and asserts the ellipsis is in the frame and the full
name is not. **A correct helper that is never called is the same shape of defect as a button
that is drawn and never wired** — and this repository has two of those already on record.

## What nothing exercises

- **A wide terminal.** The budget is a function of the pane width, and the layout's own
  tests cover the geometry; nothing measures what a genuinely wide window looks like with
  `AGENT_MAX` in play.
- **The tree pane.** Its rows are indented rather than elided, and a deep path in a narrow
  pane can still clip. Not fixed here — it needs the same treatment with an indent-aware
  budget, and doing it halfway would be worse than leaving it visible.
- **Real font metrics.** `AVG_ADVANCE` is an average, so a row of `iiiii` is shorter than it
  could be and a row of `WWWWW` is cut a little early. Elision is correct either way; only
  the tightness is approximate, and measuring it would need `egui`'s `FontsView` per frame.