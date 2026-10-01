# M237 — reviewing what the agent changed, and the window that was never idle

Two things an agent IDE needs and did not have: **a review pane for the agent's edits**, and
**a window that does not lie about being idle**.

## What changed on disk is the review, not what the call said

`crates/epsilon-desktop/src/diff.rs` watches the paths a tool call names and reports what
actually moved.

The snapshot happens on `ToolExecutionStart`, **which is emitted before the tool runs**, so
reading there is genuinely "before". Reading afterwards would produce an empty diff for every
edit — the failure mode of a review pane that silently shows nothing, which is worse than no
pane because it looks like one that has nothing to show.

**A call that changed nothing produces no row.** Not a row saying "no changes": a review pane
full of "no changes" is one nobody reads, and a reader who has learned to skip them will skip
the real one too. `a_tool_call_that_changes_nothing_produces_no_change_row` drives a real
offline turn and asserts exactly that.

### The diff is a prefix/suffix diff, and it says when it is being coarse

A coding agent's edits are overwhelmingly localised — one hunk in a known file — which a
common-prefix/common-suffix walk renders **exactly**. It is not an approximation: the affixes
are maximal, so the two middles *are* the changed region.

What varies is whether that region reads as a small edit or a rewrite. Past
`COARSE_MIDDLE_LINES` (8) the row says `N lines replaced` rather than implying a tidy hunk
the code did not compute.

**That threshold started as the wrong rule.** The first version called any two-sided middle
coarse, which labelled a one-line fix as a block replacement — caught by
`a_single_line_edit_is_shown_as_exactly_that`. Calling a one-line replacement "N lines
replaced" is wrong in the *other* direction, and a diff that overstates its own damage is the
same lie as one that hides it.

## Two gaps, named rather than discovered

**A `bash` command that edits files is not reviewed.** Its arguments carry a `command`, not a
path. Scraping a shell string for path-shaped tokens puts a filesystem probe on every tool
call and will happily report a file the user merely *mentioned*; doing it properly needs a
shell parser. **A wrong diff is worse than no diff**, because it is a diff the reader has to
check. `a_shell_command_names_no_file_so_nothing_is_watched` pins the behaviour so the gap is
a decision on the record rather than a surprise.

**A path outside the workspace is never watched.** `relative_if_inside` applies the same
containment check the tools apply before they write, so a tool call cannot talk the watcher
into reading `/etc/passwd`. The module doc originally overclaimed that shell edits were caught
"because the file was watched, not the tool" — a test caught the overclaim, and the doc is now
narrower than the code it describes rather than the other way round.

## The window that was never idle

eframe 0.36 splits `update` into `logic` (runs even when the window is hidden) and `ui` (does
not run at all while hidden). The drain was in `ui`.

So **a turn that finished in the background produced no `AgentEnd` on screen** — no status
change, no `idle` — until the window was shown again. That reads as "still working" for as
long as the window is in the background, which is the shape of the freeze this repository has
chased across four audits.

It now drains in both places, and the reason both are needed is the point:

- `logic` is what makes it **correct** — eframe skips `ui` entirely while hidden.
- `paint` is what makes it **testable** — a headless test drives the draw path directly.

Twice in a frame is not twice the work: the drain is a queue read, and the second call finds
it empty. Making it conditional on *how* the frame was reached would leave two paths that can
disagree about what is on screen.

## Verification

**Battery `m237` — GATE PASS on attempt 1, 1,735 tests across 72 binaries, 0 failed**
(`audit/evidence/gate-m237.log`). Desktop tests: 59 unit + 10 end-to-end; clippy clean across 14
crates; check-links and check-line-endings clean.

## What nothing exercises

- **A real agent editing a real file, end to end.** The live test asserts the *negative*
  (no change row for a call that changed nothing) because the offline scripted turn happens
  not to edit; the positive case is driven through the reducer, which is honest about the
  difference and says so. A script that edits would close this.
- **The `App::logic` path.** `logic` is called by eframe and by nothing in the suite — a
  test cannot construct an `eframe::Frame` — so the hidden-window fix rests on reading
  eframe's contract, not on a measurement.
- **Diff rendering at scale.** A hundred changed lines are capped at twelve rows plus a
  "N more" line; nothing exercises the cap's own wording at a realistic size.
- **Binary or non-UTF-8 files.** `OpenFile::read` refuses them with a message naming the
  path; that path is untested.