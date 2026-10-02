# M249 — the window follows the agent, and the keyboard goes where you put it

Two defects with the same shape: **the window was a snapshot, and so was the keyboard.**
Both were invisible to 113 tests, and both are the state this repository keeps refusing to
ship — the thing the user asked for happened, the window said it happened in one pane, and
every other pane stayed exactly as it was.

## The tree and the editor did not move

`FileTree::load` ran once, in `new`. The editor held whatever the file said when it was
opened. So:

- a file the agent **created** was absent from the tree,
- a file the agent **edited** still showed the text the edit replaced — the worst kind of
  stale, because it looks current and is confidently wrong,
- a file the agent **deleted** stayed on screen in full, presenting itself as a file.

All three are the same defect: the panes are views of the filesystem and nothing connected
them to it. The transcript already knew — `push_change` records every path a tool call
touched — so the signal existed and had no consumer.

`Transcript::take_changed_paths` is that signal, and `refresh_changed_files` spends it: the
tree is re-walked, and the **open** file is re-read, but only when its own path is among
those changed. That last condition is the reason a vanished file is trustworthy: a file
that cannot be read now really is gone, rather than transiently locked by an antivirus scan,
so the editor closes it **and says so** rather than keeping stale contents on screen.

The filter query deliberately survives a reload — a reader filtering for a file the agent
has just created should still be filtering for it.

## The composer stole the keyboard back

The composer called `request_focus()` every frame, unconditionally, and the comment said
why: *"there is exactly one text input in this window… When a second input arrives this
becomes 'unless something else has claimed focus', which is a one-line change and a comment
here rather than a rewrite."*

A second input had arrived. The file filter is a text box. So a reader could click it,
watch it take focus, and have the keyboard pulled straight back out of it on the next frame:
a control that takes focus and cannot be typed into. `Escape` was worse — it cleared a
draft in a box the reader could no longer see, which reads as a broken key rather than a
focus change.

The exception the comment promised is now the code: the composer takes the keyboard only
when nothing else holds it, read from egui's own focus state so it cannot disagree with
the widget that owns it, and `Escape` brings the keyboard home. **A reader can now get
back to the composer from the keyboard**, which is the part that turns "focus is somewhere
else" from a trap into a state.

The filter's rectangle is now recorded in `hotspots`, because until it was there was no way
— for a test or for an assertion — to put the keyboard in that box and see what happened.

## What Ponytail's review of this diff changed

Run against my own change, it found two things and both were mine:

- `delete:` **`Transcript::changed_paths` had no caller.** A definition with no production
  caller is this repository's most-tracked defect class — `set_tool_hooks`,
  `branch_with_summary`, `approve_pending` — and this one had been added in the same
  commit that fixed another instance of it. Removed.
- `shrink:` **`Harness::press` returned a `FrameSummary` only to feed an assertion
  `region("composer").iter().any(|_| true)`** — true of any non-empty region, so it could
  not fail. The return value and the assertion are both gone; the test's real claim is that
  typing reaches the draft after `Escape`, and that one is real.

## Verification

Battery `m249` — **GATE PASS on attempt 2: 1,808 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m249-r2.log`). Up from 1,804 at M248. `epsilon-desktop` is 92 unit +
21 window tests.

**Attempt 1 failed on `check-line-endings`, and it was my fault from the previous
milestone.** The six Ponytail `SKILL.md` files copied in M248 landed with CRLF endings in
the working copy. Git normalised them on commit exactly as `.gitattributes` promises — the
committed blobs are LF, byte for byte (`hash-object` equals the HEAD blob) — so the damage
was confined to the checkout. This is precisely the failure that gate exists for, and its
own message is the best argument for it: *"`git diff` shows a CRLF working copy as
UNCHANGED… a source-parsing test then fails with a message about something else entirely."*
Fixed, re-verified at **876 tracked files, 0 with CRLF**, and attempt 2 passed.

That the gate caught it a milestone late is the lesson worth keeping: **a passing battery
records the bytes it ran against, and the bytes it ran against were the source files — the
markdown it also checks was wrong, and nothing in the round noticed.**

**Negative controls, measured:**

| control | result |
|---|---|
| `refresh_changed_files` not called | all three change tests fail: created / edited / deleted |
| composer takes focus unconditionally (the old code) | `the_file_filter_takes_the_keyboard_and_escape_brings_it_back` fails: `typing must reach the control the reader clicked` |

**Reinstall:** none. `epsilon-cli` and `epsilon-tui` are untouched; this is the desktop
binary, which the owner launches directly.

## What nothing exercises

- **A click on a button may still move the keyboard.** egui gives a clicked `Button`
  keyboard focus, and this window has buttons (tabs, activity bar, session rows). The
  composer yields to *whatever* holds focus, so clicking a tab may leave the keyboard on
  that button. **No test covers it and it may well be a defect** — it is the first thing to
  check next round, and the fix is likely to narrow the yield to text inputs by giving the
  filter a stable id like the composer's.
- **The tree reload is driven by the agent's own change events only.** An edit made outside
  the window — in another editor, or by the user in a terminal — does not refresh the tree.
  That needs a filesystem watcher, which this does not have.
- **Nothing has been measured through wgpu.** No screenshot, no text-throughput number
  under `vello_cpu`. Every claim above is an offscreen-`Context` measurement, which is
  egui's real draw path but is not a GPU rasterizer.
- **`row_rects` still does not cover the tasks and sessions panes.** They share the M248
  isolation fix; a regression moving only those two is still not caught.