# M218 — a list row that reads as one word

**A frame was read, and the first thing in it was wrong.** The defect class is M198's and M201's —
something every state-level test agrees with and only the drawn row can show — and it was found by
driving the **installed** binary in a real console window, which is also how it is proved fixed.

## What

`overlay::list_row(theme, selected, label, meta, width)` lays a list row out as a left label and a
right-aligned `meta`. The padding between them was

```rust
let pad = width.saturating_sub(used + meta_width);
```

so when the two exactly fill the row, `pad` is **zero** and the metadata is appended flush against
the label. The shipped slash popup at its own width does exactly that:

```text
│ ▎ /sessionsbrowse session files in this workspace an │
```

`/sessionsbrowse` is one word. The eye cannot tell where the command ends, and the description's
last letter is a clip with nothing marking it as a clip. `/sessions` is 9 columns, its description is
37, and the popup's inner width is 54 — the row is full by construction.

## The fix

**A minimum gap, and the metadata is what gives way.** `MIN_META_GAP = 2` columns are reserved
between the two; if the metadata does not fit in what is left it is clipped with an ellipsis, and if
it cannot fit *with the gap* it is dropped rather than glued on.

```text
│ ▎ /sessions  browse session files in this workspace… │
```

**The label is never shortened to protect the hint.** The label is what the user is typing and what
`Tab`/`Enter` will insert; a description is worth less than a command name that stays readable. That
asymmetry is the design decision, and it is why the budget is taken from the metadata side.

**Why two columns.** One column is enough to stop a run-together and still reads as a missing
space rather than a gap; two is where the eye reads two fields.

This is one function, so every list in the harness is fixed by it: the command palette (which
right-aligns a key binding), the slash popup, the model picker, the session browser and the `@`
file popup all draw through `list_row`.

## Verification

**The new test draws the row and asserts on the drawn text**, because the defect is a property of the
frame: `a_row_too_narrow_for_both_still_separates_them` covers the exact case from the console
(54 columns, `/sessions`, the real description), a roomy row that must stay right-aligned and exactly
`width`, and a cramped row where the hint is dropped. A test asserting on the *fields* — that the row
has a label and has a meta — passes against this defect; the one that fails is the one that reads
the string.

**The control.** The API is old, so a pre-fix RED exists, and it is run rather than assumed: with
`MIN_META_GAP = 0` the new test fails with the shipped frame verbatim.

```text
panicked at crates\epsilon-tui\src\ui\overlay.rs:448:
the two must never read as one word: "  /sessionsbrowse session files in this workspace and…"
```

Restored, mtime touched, re-run green.

**And the fix is confirmed on a real console**, not only in a `TestBackend`: the frame above is
`scripts/console-probe.py` driving the binary at 100x30 with `/sessions` typed, read back from the
screen buffer.

## A failure that was not this slice's

The first battery (`m218`, kept as `audit/evidence/gate-m218-FAILED-ext-load.log`) **failed on two
tests in `epsilon-ext`'s `host_protocol`** — a crate this slice does not touch:

```text
a_child_that_never_stops_sending_lines_is_cut_off_by_volume
  expected Protocol once the byte budget is spent, got Timeout
a_request_after_the_reader_stops_is_refused_even_though_the_child_is_alive
  a request issued after the reader stopped did not return within 2s
  (this is a hang, not a slow test; note that every line the child sends can restart a per-read timer)
```

The facts, and they are what the claim rests on:

- the same tree passed at `m217-r2` (**1,638 tests, 0 failing**) minutes earlier;
- re-run **in isolation** (`--test-threads=1`) the binary is green: **29 passed, 0 failed**, in
  58.97 s — against **14.87 s** inside the battery, where those 29 tests run concurrently, each
  spawning a child process and asserting against a **2-second** deadline;
- `epsilon-ext` was not modified by this slice, and `overlay::list_row` is in `epsilon-tui`, which
  `epsilon-ext` does not depend on.

So: **load-sensitive, cause not established.** The second test's own message names the mechanism it
suspects (a per-read timer restarted by every line a child sends), which is consistent with 29
concurrent subprocess tests on a machine that also had a release build's worth of page cache churn —
but "consistent with" is not a cause, so it is recorded as an open row rather than fixed by a
timeout bump that would only hide it. The honest follow-up is to **serialize that binary**
(`--test-threads=1` for `epsilon-ext`) or give those two deadlines headroom, and to say which.

**`m218-r2` is the record for the commit: GATE PASS, 1,639 tests across 65 binaries** — `+1` against
M217's 1,638, which is exactly the new row test.

## Limits

- **The clip is by character, the layout is by display width.** `truncate_public` counts `char`s,
  so a description in a wide script could need more clipping than the budget assumes. The row's own
  guard (`used + gap + meta_width <= width`) is what keeps that from overflowing, and the terminal
  clips whatever exceeds the panel — so the failure mode is a shorter hint, never a broken frame.
- **No test drives the palette's binding hint, the model picker's prices or the session browser's
  rows** through this change; they are the same function and therefore the same fix, but their
  *content* at narrow widths is unmeasured. A row whose label alone exceeds the width still clips,
  which was true before this change and is unchanged by it.

## What nothing exercises

Nothing asserts the *gap* on a rendered popup: the new test builds the row through `list_row` and
reads its spans, which is one level below the painted panel. Painting the slash popup itself would
catch a second cause — a panel that hands `list_row` a width smaller than it draws — and that cause is
not excluded here; it was simply not observed.
