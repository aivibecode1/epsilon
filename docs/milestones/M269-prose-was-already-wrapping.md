# M269 — the window's primary content was invisible to every measurement

A review reported that **assistant prose never wraps** in the desktop window — that every
paragraph in its main pane is one line, clipped at the edge — and ranked it the worst of the
UI findings. This milestone measured it. **It is false**, and measuring it turned out to be
worth more than fixing it would have been.

## The finding, and what the measurement said

The claim rested on egui's `Label` defaulting to `wrap: false`, which the reviewer **explicitly
flagged as "from memory, not from a vendored copy"** — and marking it as a guess was exactly
right, because it was the guess that was wrong.

The same assistant paragraph, painted through the real `paint` at 1440×900:

| | paragraph | tall row |
|---|---|---|
| **without** `.wrap()` | 12 sentences | **135 px** |
| **without** `.wrap()` | 2 sentences | **30 px** |
| **with** `.wrap()` | 12 sentences | **135 px** |
| **with** `.wrap()` | 2 sentences | **30 px** |

Identical. A label that did not wrap would be one line — about 18 px — at any length. egui
0.36's `Label` built from a `LayoutJob` wraps already.

So the `.wrap()` this change first carried was **a no-op that only looked like a fix**, and it
is not in the code. That is the first half of the milestone and it is the part worth copying:
**a predicted defect that turns out not to exist still has to be measured**, or "we looked at
it" is indistinguishable from "it was fixed".

## What actually changed, and why it mattered more

Assistant text recorded its **text** in `FrameSummary.rows` and never its **rectangle** in
`row_rects`.

That is not a detail. `row_rects` is the only record of *where* anything was painted, and the
containment sweep in `app.rs` iterates it — so **the window's primary content sat outside every
containment check the crate has.** More sharply: it is why the false finding above survived a
careful reading. Nothing could ask the frame whether prose wrapped or was clipped, because the
frame did not record where prose was.

Every block now records its rect, which means:

* the assistant's text is inside the containment sweep for the first time;
* "is this one line or several" is a question the frame can answer, and the test above is
  written that way.

## Getting the measurement right took three attempts

Worth writing down, because each failure was a different wrong instrument:

1. **An absolute threshold** — `assert!(tallest > 40.0)` — **passed with the wrapping
   removed.** It was measuring the composer, which stands taller than the paragraph. A control
   that passes against the broken build is not a control.
2. **A width differential** — the same paragraph at 1440 and at 760. Still false: the composer
   alone is 135 px tall at 1440 and 180 px at 760, which swamps whatever the paragraph does.
3. **Paragraph *length* at a fixed width** — the one variable that moves only the paragraph.
   A label that does not wrap is one line however much text it holds, so two lengths produce
   identical frames, and nothing else in the pane moves.

## Verification

Battery `m269final` — **GATE PASS: 1,898 tests across 77 binaries, 0 failed**, all seven
gates, **attempt 1** (`audit/evidence/gate-m269final.log`). Up from 1,897 at M268.

Two attempts before it failed on **`fmt --check` and nothing else** — I ran clippy after my
last edits and skipped `cargo fmt`. Both logs are kept
(`audit/evidence/gate-m269-FAILED-fmt.log`), because they are also the second half of the
tooling finding below.

- `a_long_paragraph_is_not_clipped_to_one_line` — the differential above. It passes today,
  and it is a **regression guard rather than a proof of the old defect**: it pins "long prose
  occupies more than one line", so a future change to the label construction that stopped
  wrapping would fail it. It does *not* discriminate `.wrap()`, and this milestone says so
  rather than claiming otherwise.

## The retry wrapper's fingerprint, again, and worse

M268 recorded A46: the fingerprint attributes a failure to the wrong gate. This round it
produced the opposite failure — an **empty** fingerprint (`tests:|gates:`) for a battery that
failed only on `fmt --check`.

The cause is the same regex viewed from the other side:

```powershell
'^===== ([a-z-]+) =====\r?\n[\s\S]{0,4000}?^\[exit=([1-9])'
```

**A 4,000-character window.** `cargo fmt --check` prints a diff, so its `[exit=1` lands far
past the window and nothing matches at all. The wrapper then reported *"same fingerprint as the
previous attempt — that is not a flake, that is the code"*, for a formatting difference, with
no gate named.

Two properties, worth fixing together: the window must not depend on how much a leg printed,
and **an empty fingerprint should stop the wrapper** rather than let it conclude "not a flake".

## What nothing exercises

- **The other two A39 findings were not touched**: the permission ask is still printed twice
  (a row plus an answerable prompt), and a submit refusal still sticks in the status bar for
  the rest of the session. Both are in the ledger with acceptance criteria.
- **`row_rects` still misses the tasks and sessions panes**, so containment is still unmeasured
  there — recording the transcript is the precedent, not the completion.
- **These are offscreen numbers.** Nothing below tessellation has been measured on this box
  (A25), and a real window adds a present step these frames do not.