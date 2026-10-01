# M248 — the window shows the conversation

Six defects in the desktop IDE, and the common cause is the one this repository has now
paid for three separate times: **a thing that was correct in every way a test could see,
and was not on the screen.** A pane laid itself out on top of the file tree. A permission
gate was painted with the keys that open it, and nothing opened it. A file viewer opened
every file at its last line. None of it was invisible to the suite — the suite was
measuring the wrong thing, which is worse, because green is what stops people looking.

## The one that mattered: the transcript was drawn in the wrong place

`paint()` gave every pane the **same** `egui::Ui`. A `Ui` has a cursor, the cursor walks,
and whichever pane drew first moved it — so the next pane started wherever the last one
stopped. Measured, not reasoned:

| | before | after |
|---|---|---|
| agent pane | `[920,42] – [1440,870]` | `[920,42] – [1440,870]` |
| transcript laid out at | **`[0,142] – [504,817]`** | `[920,42] – [1424,717]` |

The right-hand pane was empty and the conversation was painted **on top of the file tree**,
in the column the reader uses to find files. Every pane drew correct text. The 103 tests in
the crate passed throughout, because `FrameSummary::rows` records *what* was drawn and
nothing recorded *where*.

`layout.rs` had a non-overlap test sweeping ten widths and six heights, and it kept
passing — the **panes** never overlapped. Only their contents did.

**Why it survived so long.** It is invisible to any assertion that reads text, and this
crate had no way to read geometry at all until `FrameSummary` grew a `row_rects` field. The
fix is a `Ui` per pane with its own cursor and its own clip rect (`pane_ui`); the test is
`the_transcript_is_drawn_inside_the_agent_pane_and_nowhere_else`, which asserts every
painted agent row is inside the agent pane.

**Control:** with the agent pane handed an unpinned `Ui` again, that test fails with
`a agent row was painted at [[0.0 0.0] - [67.6 15.0]], outside the agent pane
[[920.0 42.0] - [1440.0 870.0]]` — the defect, reproduced on demand.

## The one that made the window unusable: a gate that could not be opened

The transcript painted `allow bash <pattern> ?   [enter] allow   [esc] refuse`.
`approve_pending` — the function that answers it — **had no caller anywhere in the crate**,
and the composer consumed `Enter` and `Escape` first, because it uses them to send and to
abort. So the window drew a permission gate and offered the reader two keys that both did
something else.

This is the fifth instance of the class `AGENTS.md` names as *"a faithful port with no
production caller"*, and the second in this crate. The keys are now handled **before**
anything else can want them, and the ask carries **Allow**/**Refuse** buttons beside it —
a gate that stops the whole run cannot be reachable only from the keyboard.

`a_real_turn_runs_to_completion_through_a_permission_ask` already walked this gate and
called `session.resolve` directly, describing it as *"the way the user would"* — which it
is not. It proved the *harness* could be answered and said nothing about the *window*.
`a_permission_gate_is_answered_by_pressing_enter_in_the_window` presses the key the row
names.

**Control:** with the `approve_pending` call disarmed, that test fails with the message it
names — *"pressing it left the gate standing — the window advertises a key that does
nothing."*

## The other four

- **The editor showed the tail.** `draw_editor` painted `lines[len - visible..]`, so
  opening a 900-line file showed its last forty lines with no scrollbar and no way to reach
  line 1. It now opens at line 1 and scrolls. The editor also draws **through a clip rect**
  — `OpenFile` keeps whole lines on purpose, so a long line used to paint straight across
  the transcript beside it.
- **A session that would not resume was reported to nobody.** The refusal went into
  `FrameSummary`, which is test instrumentation and paints nothing, under a comment
  claiming it was on screen. It now also goes to the transcript, which is painted.
- **The status line led with debug state.** It ended with the layout tier on every window,
  including the word `full`, which is a fact about size a reader cannot act on. It now
  appears only when the layout has actually given something up — which is the one case
  where the reader needs a pane's absence explained.
- **The activity bar was three unlabelled glyphs.** `SidebarView::label()` existed and was
  never drawn; the one control that decides what everything else shows had no name on
  screen. Now a tooltip.

## The tab strip, finished

An uncommitted slice had tabs half-built: `draw_tabs` recorded hotspots for a test that did
not exist, and `pressed_tab` read them back **earlier in the frame than the pane that fills
them**, so the list was always empty and **a tab could never be pressed**. Fixed by the
pattern `draw_tree` already uses — the draw returns what was clicked. Tabs now share the
strip's width, so the tenth open file is still clickable rather than drawn past the edge.

A test harness that can click took three attempts, and the note is worth keeping: **a
click is three painted frames**, and a frame that builds no widgets registers nothing for a
press to land on. The first version looked like a working harness and silently clicked
nothing — which is why the shipped strip was a picture of a tab strip.

## One dependency removed

`egui_extras` was declared with the `syntect` feature — a full syntax-highlighting engine —
and referenced by **no `.rs` file in the workspace**. Deleted: **14 packages** out of the
graph. Highlighting, if it is ever wanted, is a change with a reason attached.

## Verification

Battery `m248b` — **GATE PASS on attempt 1: 1,804 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m248b.log`). Up from 1,794 at M247. `epsilon-desktop` is 88 unit +
21 window tests.

The first attempt (`gate-m248.log`) failed on **this milestone's own new test** — which is
worth recording rather than quietly replacing. It passed alone and failed under the
battery, because it painted a frame, *then* asked whether a gate was waiting, and asserted
on the frame it had already painted. The agent's thread registers the ask on its own
schedule, so the assertion read a frame from before the ask existed whenever the thread
landed between the two calls. Check-then-act, in the test. The fix is the ordering, and
the lesson is the one this repository keeps writing down: **a failure that only appears
under load is still a real failure**, and "it passed when I ran it alone" is not a fix.
Re-run 6× after the change: 6/6.

**Negative controls, both measured rather than argued:**

| control | result |
|---|---|
| `approve_pending` call removed | `a_permission_gate_is_answered_by_pressing_enter_in_the_window` fails: *"pressing it left the gate standing — the window advertises a key that does nothing."* |
| agent pane handed an unpinned `Ui` again | `the_transcript_is_drawn_inside_the_agent_pane_and_nowhere_else` fails: `a agent row was painted at [[0.0 0.0] - [67.6 15.0]], outside the agent pane [[920.0 42.0] - [1440.0 870.0]]` |

The editor's two tests are structurally tied to the fix rather than controlled: the broken
version started at line 861 of 900 and ended at line 900, and both assertions reject that.

## Reinstall

**None needed, and deliberately so.** Everything changed here is in `epsilon-desktop` and
in two workspace manifests; nothing in `epsilon-cli` or `epsilon-tui` was touched, and the
desktop IDE is a separate binary the owner launches directly. A `cargo install` would
replace a working CLI with a byte-identical one.

## What nothing exercises

- **Nothing here has been measured through wgpu.** No screenshot, no text-throughput number
  under `vello_cpu`. Every claim above is a `TestBackend`/offscreen-`Context` measurement,
  which is egui's real draw path but is not a GPU rasterizer.
- **The tab strip has no close button.** Tabs accumulate until the window closes, and a file
  opened for one look stays. Deliberate for now — it is a new control, and new controls
  want their own slice and their own test.
- **`row_rects` covers the transcript and the editor, not the tasks and sessions panes.**
  Those panes share the isolation fix, but a regression that moved only those two would not
  be caught by the containment test as written.
- **The agent pane's containment is asserted at one size.** The layout's own sweep covers
  the geometry at ten widths; this covers the contents at one.