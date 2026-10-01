# M236 — the file tree and an editor that admits it is read-only

The workspace and editor panes drew three placeholder labels and one line of text. They now
walk the workspace and open what the reader clicks.

## The walker moved, because its own module doc said it would

`epsilon_tui::at_files` owned the only workspace walk, and it ended with a prediction:

> *If a third consumer of this walk ever appears, the walk belongs in core beside `IgnoreSet`
> — two copies of "what to skip" is one more than the rule allows.*

The desktop file tree is that third consumer. So the walk is now
`epsilon_core::walk::walk_files`, its five tests come with it, and `at_files` is a thin
front-end wrapper keeping `@`'s own budget — **one traversal, two budgets**, which is the
right shape: `@` is capped for typing latency, a browsable tree is not.

Writing a third traversal from `IgnoreSet`'s rules was the alternative, and it is worth being
concrete about what that costs: a file `grep` refuses to search appearing in the editor's
tree. Neither bug gets reported, because both look like a list being a list.

## Two things the tree is careful about

**It says when it is partial.** The pane's footer prints `N files`, or `N+ files · list is
partial` when the walk hit its budget. A tree that silently ends looks like a repository with
no more files in it, which is a claim the pane cannot support.

**It is drawn with `show_rows`, not a recursive `Tree`.** A widget per node *per frame* is
the difference between a tree that opens instantly and one that spends its first second
building widgets for files nobody asked for — and that cost lands on the window the agent is
streaming into.

An empty directory is **not** in the tree: directories are synthesised from the files beneath
them, so a directory with no files is not something the agent can act on, and listing it
would be the tree claiming content the workspace does not have.

## The editor is read-only, and says so on every frame

`only.rs  ·  3 lines  ·  read-only`. The empty state says *"Pick a file on the left to read
it"* rather than being blank, because a blank editor pane reads as a broken one — and a test
asserts that an editor with nothing open does **not** claim to be showing a file.

Editing is not here and is not faked. A half-built editor that drops the caret on a resize is
worse than a read-only view that is honest about being one.

## Three bugs, and the shape they share

Every one of them was found by a test that pressed where a row was *drawn*, and every one of
them was "the geometry is wrong somewhere I did not look".

1. **The scroll area was never positioned.** `draw_tree` took a `rect` and used it only for
   height and width; the scroll area started at the root `Ui`'s cursor, which is `(0, 0)`.
   Every row was recorded at the left edge of the *window*.
2. **Nested scroll areas.** A `show` wrapping a `show_rows` builds its `Ui` from the scroll
   area's own origin, not the pane's — the same symptom, different cause.
3. **The row rectangles were computed by hand** from `next_widget_position`, which inside
   `show_rows` is relative to the content rather than to the screen.

The fix is one idea applied three times: **allocate the row, do not position it.**
`allocate_exact_size` returns the screen rectangle *and* the click handling together, so the
two cannot drift — and the geometry that goes into a hit test is the geometry that was drawn.

The other half is the one line that made the test possible: `FrameSummary` now records a
**hotspot** per clickable thing. A painted row has no response of its own, and a caller that
needs to press one cannot recover where it was drawn from the text alone — indentation is not
geometry. A test that guessed a `y` would have passed or failed with the sidebar's height,
which is the layout module's business and not the test's.

## One more thing the harness taught

The click is **three frames**: move, press, release. Sending the press and the release in one
input frame is not something a person can do, and egui treats the two as separate passes. The
first four attempts all used a one-frame "click", which tests a state the product is never in
— the same mistake the keystroke work made, and the reason that one is written down in
`tests/window.rs` rather than quietly fixed.

## Verification

**Battery `m237` — GATE PASS on attempt 1, 1,721 tests across 72 binaries, 0 failed**
(`audit/evidence/gate-m237.log`). Up from 1,704: the walker's five tests moved to core, plus
thirteen for the tree, the editor and the click. clippy clean across 14 crates;
check-links and check-line-endings clean.

## What nothing exercises

- **Editing.** The editor is read-only by design here.
- **Collapsing.** `FileTree::toggle` works and is unit-tested, but no click test drives a
  directory row end to end.
- **The filter box as typed.** The pane carries a `filter files` box; the filter is tested
  on the tree, not through the box.
- **Files with awkward encodings.** `OpenFile::read` uses `read_to_string`, so a binary or
  non-UTF-8 file is refused with a message naming the path. That is the right behaviour and
  it is untested.