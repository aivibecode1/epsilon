# Grapheme-safe cuts

**Status: landed.** Batteries `gate-grapheme-cuts-b-20261013`,
`gate-sound-marks-a-20261013` and `gate-file-editor-undo-20261014` — 15/15 legs each,
0 failed.

## The root cause, in one table

Four functions decided where to cut text with `unicode-width`'s **per-character** rule while
the wrap decision used its **per-string** rule. Measured:

| sequence | `UnicodeWidthStr::width` | sum of `UnicodeWidthChar::width` |
|---|---|---|
| family ZWJ emoji | 2 | 8 |
| thumbs-up + skin tone | 2 | 4 |
| heart + VS16 | 2 | 1 |
| keycap 1 | 2 | 1 |

The two rules cannot agree about where a cluster ends, so a cut decided with one and
measured with the other landed mid-cluster.

## What the audit found (criterion 1)

1. **A hang.** `wrap_line`'s hard-split loop advanced by `cut.len()`; `split_at_width`
   returned `""` when the row was narrower than the first grapheme. Reachable through
   `app/panels.rs:1057`'s `.max(1)` description budget. A probe hung past 60s.
2. **Emoji split at row boundaries** at widths 2–7 and 10 of 1–10 (8 and 9 clean).
3. **A false comment** on `clamp_line` claiming a `unicode-width` version skew. The lock
   resolves both to 0.2.2; the real disagreement is ratatui's `CellWidth for str` adding
   +1 per halfwidth katakana sound mark (U+FF9E/U+FF9F).

## The shape of the fix

`wrap::prefix_at_width` and `wrap::suffix_at_width` — grapheme boundaries, `UnicodeWidthStr`
per grapheme. Four callers now go through them, deleting four hand-rolled copies. The
hard-split arm gains a progress guarantee.

**Deliberate trade**: at width 1 a 2-column cluster is emitted whole on its own row and
overhangs by one column. The alternatives were dropping it (silent loss) or halving it
(a box). Bounded by the grapheme's own width, so it never accumulates.

## Traps for whoever touches this next

- **A "row ends in a zero-width char" detector is wrong in both directions.** `❤️` is a
  complete grapheme whose last char is a zero-width VS16, so that check reports every heart.
  And a metric over the *concatenated* rows reports nothing, because concatenating re-joins
  the cluster. The check that works: walk the input's graphemes once, consume exactly as
  many as each row holds.
- **`suffix_at_width` must drop from the FRONT until the remainder fits**, then stop at the
  first point where it does (that is the longest suffix). Walking forward accumulating and
  returning the held offset produces a suffix wider than asked for — measured
  `"ix👨‍👩‍👧‍👦👨‍👩‍👧‍👦"` for a budget of 4.
- **Adding tests to `epsilon-tui` moves line numbers cited by `docs/parity/pi-tui.md`.**
  `check-citation-placement` fails when a cited line lands inside a test module, which is
  how the first battery went red. Re-point with `check-parity-citations.py` to confirm the
  new line still resolves.
- ratatui's own `Buffer::set_stringn` was already grapheme-aware. Before adding a new cut
  helper, check whether ratatui can do it — the repo was carrying a worse copy beside it.

## The halfwidth katakana sound mark (U+FF9E/U+FF9F) — closed

`unicode-width` reports both as **zero-width** (they carry `Grapheme_Extend`); ratatui's
`CellWidth for str` (`ratatui-core/src/buffer/cell_width.rs`) is `unicode-width` **plus one
column per occurrence**, citing Ruby reline PR #832. So there are two different rules in one
process, and this crate had **four** measurement sites on the wrong one:

1. `ui::layout::display_width`
2. `wrap::prefix_at_width` / `suffix_at_width` (per-grapheme)
3. `wrap_line`'s word-fit and hard-split loop guards
4. `ui::card::cols` — **the consequential one**: `render_cache` asks `card::row_width`
   whether a row needs wrapping, so an under-measured row was accepted as fitting, never
   wrapped, then clipped.

`display_width` now carries the correction once; the other three delegate to it. It is
therefore the *same formula* ratatui applies, and the agreement is asserted against
`str::cell_width()` rather than a number, so a ratatui bump fails the test instead of
drifting.

### Traps this slice produced — all verified the hard way

- **A test that shares its unit with the code under test proves nothing.**
  `assert display_width(row) <= width` passes under both rules. Assert the *renderer's*
  rule (`row.cell_width()`) when the claim is "the buffer can place this".
- **The same trap in reverse for verification scripts.** I measured a katakana frame as
  119 columns and nearly "fixed" correct code. The identity is:
  `cell_width = unicode_width + marks = (chars − marks) + marks = chars`, because the
  marks are the only zero-width chars present. Adding the marks to a char count that
  already contains them double-counts. Measure the number the code reads.
- **A frame's own overspill guard can be blind.** `frame_text` counts `char`s; the bug is
  in *columns*. A frame "with no overflow" can still be wrong — check by identity.
- **`frame_text`'s guard also only sees `chars().take(w)`**, so column overspill is
  truncated away and invisible.
- **Adding tests to `epsilon-tui` moves cited line numbers.** `display_width` went 330 →
  417 → 502 across these slices; both `pi-tui.md` citations needed re-pointing twice.
  Run `check-citation-placement.py --verbose` after adding tests and re-point with
  `check-parity-citations.py` to confirm they still resolve.
- **Two independent sites can be wrong for one reason, and a single control will not find
  it.** Reverting `display_width` alone caught the wrap bug; reverting only the cut
  functions' measurement (with `display_width` fixed) *also* caught it. Control each half.
- **A notice row does not reach `wrap_line`.** For frame evidence of wrapping, drive
  *tool output* (`ToolExecutionUpdate`) — that goes through `render_cache`.

## The file editor had no undo, and `Delete` did nothing (fixed)

Two gaps found by asking what the editor *did* rather than what the docs claimed:

1. **`Delete` fell into the edit-mode `_ => {}` arm** and was silently discarded while
   `Backspace` beside it worked. `delete_forward` on `FileViewer` is now the mirror of
   `backspace`, and joins the next line at end of line.
2. **No undo at all in the file editor**, while the composer has had it since M16.

`UndoStack<S = EditorSnapshot>` is now **generic**: the payload is a type parameter and
`EditorSnapshot` is the default, so every composer call site still reads `UndoStack`.
The file viewer uses `UndoStack<FileSnapshot>` over `(Vec<String> lines, row, col)`.

**Two traps in writing the stack itself:**
- `#[derive(Default)]` on a generic struct emits `impl<S: Default> Default`, which fails
  for `UndoStack<FileSnapshot>`. Write `Default` by hand — `Vec<S>: Default` needs no
  bound on `S`.
- `UndoStack<S>` must also derive `Clone`, because `FileViewer` (which holds it) does.

**Every mutating primitive pushes a snapshot first** — `insert_char`, `split_line`,
`backspace`, `delete_forward` (twice: line-join and mid-line). The stack's duplicate
suppression is what stops that flooding. `start_edit` resets the history, so a fresh
session has nothing to undo until the user changes something.

**The chords are raw, not table entries** — matching the existing `Enter`/`Backspace`/
arrow arms in that match. It runs before table dispatch, so there is no collision with
`tui.editor.undo`.

**Frame witness for this slice**: `audit/evidence/frames/file-editor-undone-edit.*.txt`
shows `the quick brown fox!` with `(modified)` in the title;
`file-editor-undone-restored.*.txt` shows the same row without it. The title's own
marker is what makes the undo legible on a frame — assert nothing, just read it.

**Deliberately still absent from the file editor** (recorded so it is not mistaken for
oversight): selection, cut/copy/paste, multi-cursor, `Tab`.

## Find in edit mode works; replace was REMOVED as unshippable

`/` now opens find **over the edit buffer**, and `find_step` moves the caret to the
match. Before this the key typed a literal slash into the text: the edit-mode key arm
`return false`d before the find arm ran, and the hit list was computed from `lines`
(the file as opened), so it named text the user had already changed.

**Find-and-replace was built, tested, and then deleted.** It is worth knowing why,
because the bug is still there:

- `replace_all`/`replace_one` worked and were unit-tested. The tests caught **two real
  bugs I wrote**: `push_undo` after the mutation makes undo restore the replaced text
  (so `Ctrl+Z` appears to do nothing); and a no-op replace still pushes a snapshot,
  poisoning the rail. Both fixed by snapshotting *before*, and only when the
  replacement will actually change something.
- It was deleted because **the replacement line was never drawn.** The panel pads
  `lines` to `capacity` with a `while` loop, *then* pushes the query line, the
  replacement line and the hint row. A `Paragraph` paints only its first
  `inner.height` rows, so the extras fall past the edge. Reserving rows didn't fix it
  on the first two attempts (see below).
- Shipping an input the user cannot see is exactly the silent no-op AGENTS.md
  forbids, so the whole feature came out rather than half-ship.

**The row-budget bug is REAL and still open, and it is the next thing to fix.** In
`draw_viewer` (`tree.rs`):
- `capacity = inner.height - 1`, then `end = start + capacity` sizes the body window.
- `while lines.len() < capacity` pads to exactly `capacity`.
- Then find `+1`, replace `+1`, hints `+1` are appended → total `capacity + 3`.
- The `Paragraph` shows the first `capacity + 1`, so how many input rows survive
  depends on how long the body is — which is why the find line appeared in one frame
  and vanished in another.

**UPDATE — the row-budget diagnosis was NOT confirmed, and I chased a phantom.**
I built a frame test with a 200-line body and a control. The control passed: the
find line was always drawn regardless of the reservation, so the change fixes
nothing. And my second assertion () **passed for the wrong reason** —
that string is in the *application footer* outside the panel, so it proved nothing.
It has been deleted rather than kept as decor.

**The honest state:** the find line was never clipped. The replace line was never
drawn — and I cannot now say why, because I removed the feature before diagnosing
it properly. Any future attempt must re-add the field first, then measure, then fix.
Do not trust the row-budget narrative below; it was reasoning, not measurement.

**The fix** (as *hypothesis*, not verified): reserve the rows up front, in one place. Compute
`input_rows = 1 (hints) + find_open + replace_open`, set
`body_rows = capacity - input_rows`, use `body_rows` for BOTH the `end` bound and the
pad target. Do not reserve only for `replace_open` — that was my third wrong attempt,
and it is wrong because find's row was never reserved either; the frames that appeared
to show it did so by accident of body length.

**Then re-add**: the two fields, the two ops, `Ctrl+R`/Enter/`Ctrl+A` (the frame test
`find-replace-*` and the unit tests are recovered from this session's history), and the
frame assertion `both.contains("> pin")` which must pass before the feature ships.

## Evidence

- `audit/evidence/gate-grapheme-cuts-20261013.log` (red — citation placement)
- `audit/evidence/gate-grapheme-cuts-b-20261013.log` (green, 15/15)
- `audit/evidence/gate-sound-marks-a-20261013.log` (green, 15/15)
- `audit/evidence/gate-file-editor-undo-20261014.log` (green, 15/15)
- `audit/evidence/frames/katakana-sound-marks.{100x30,80x24,60x20}.txt`

## P-051: an aborted turn settles its in-flight cards

`StopReason::Aborted` pushed the "Response was stopped before completion." line and
left the card `Running` — so the transcript showed a tool still working on a turn
that had ended. Session repair at the *next launch* synthesised results for those
calls, which fixed the stored session and never the screen.

`UiState::settle_cards_for_abort` settles `Pending`/`Running` cards to a new
`CardStatus::Stopped` and appends one line to the body. `status_word` renders it as
`warn` + "stopped" — **not** the cross + "failed": the tool did not fail, the turn
was stopped, and a cross reads as a defect where there was a decision (C-110's
convention). Called from both the live `apply` path and the resume
`hydrate_from_history` path.

**The trap, and it is the repository's own evidence rule:** the unit test calls
`settle_cards_for_abort` directly, which proves the *helper* and not the *wiring* —
removing both call sites left it passing. The control has to go through `apply`:
a `MessageEnd` carrying a `ToolCall` block (that is what **commits** the card;
`ToolExecutionStart` only promotes `pending` → `running`), then a `MessageEnd`
with `StopReason::Aborted`.

**The frame found the bug that mattered.** `restale_cards` names a card for the
render cache only when `ended_at_ms` is set, so changing status+body without it
left the row cached as drawn — a live abort (which has already drawn a frame) kept
the ghost spinner forever. `settle_cards_for_abort` now sets `ended_at_ms`.

**Controls:** removing `ended_at_ms` fails the frame test. Naming aborted cards in
`restale_cards` was built, controlled, found **redundant** (naming runs after
`apply`, so `ended_at_ms` is already visible to it) and deleted. A test that calls
`app.ui.apply` directly proves nothing here — the naming lives in the dispatcher
(`apply_with_scroll`), so the witness must go through it.

**CLOSED BY DISPROOF (row budget).** The belief that the find line and hint row
were clipped was reasoning about a `capacity + 1` paragraph, never a measurement.
A panel-scoped frame test (rows carrying the `│` gutter; the earlier
`"esc close"` check matched the *application footer* and could never fail — it is
deleted, not kept) shows **both are drawn** even when a 200-line file fills the
window. The control — reverting the reservation — changed no frame at all. The
reservation itself has been **reverted** from `tree.rs`; the frame test stays as a
regression guard and was proven sensitive (it fails when told to look for an absent
string).

**MEASURED — the panel geometry, at 80x24, for a 200-line file:**
```
row 21 | line 14 of the body
row 22 | /line 42█  1 of 1 · enter next · esc close     <- find line, drawn
row 23 | ↑↓/pgup/pgdn scroll · / find · e edit · esc close   <- hint row, drawn
row 24 | (blank)                                        <- a spare row
```
So **every input row is drawn, and there is a spare row left over.** The clipping
theory is dead as a matter of measurement, not argument. Whatever made the
replacement text invisible, it was not the paragraph height.

**Both redirects disproved — `find_recompute` (tree.rs:404-409) already selects
`edit_lines` when editing and `lines` otherwise.** The code was correct; my
suspicion was wrong again. The clipping theory above is likewise dead by direct
measurement (every input row is drawn, with a spare row to spare).

**Conclusion after four turns.** Three diagnoses for the invisible replacement line
— clipping, row budget, wrong source collection — have each been disproved by
measurement, and no fourth has been found. The feature was **withdrawn on a
diagnosis I never verified**: the withdrawal was the error, not the render path.
Re-adding it is the honest next step, and the clipping argument should not be
reopened.

**Still unproven:** nothing in P-051; status, body and frame are all controlled. The settled line is on the card's *body*, and the
viewer panel only paints a body the reader has opened, so `render_rows` does not
show it. The golden test asserts the body text the renderer reads and says so
plainly — the frame witness (open the card, then abort) is the next step, the same
way find/replace's invisible input line was.
