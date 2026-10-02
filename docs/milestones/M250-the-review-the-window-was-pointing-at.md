# M250 — the review the window was pointing at

Three things, and the middle one is the reason this milestone exists.

## A review instruction with no control beside it

The transcript reported what the agent changed: a summary line and up to twelve diff rows,
then *"… N more changed lines — open the file"*. **There was nothing to open.** No row was
clickable, no pane showed a diff, and the twelve rows were interleaved with a conversation
and gone the moment the transcript trimmed.

So the pane named in `layout.rs` as *"The editor or diff"* had never been a diff pane, and
the editor had no second mode.

A **changed row is now a link**, and clicking it shows the whole change in the editor pane.
Three things it does that a coloured text row does not:

- **Added and removed are different colours, not different characters.** A `+` and a `-` are
  the entire signal, and this project drew them as two greys side by side. The colours are
  the palette's existing `success` and `danger` roles, which `theme.rs` already proves clear
  their contrast floor and are already proven not to be the same colour.
- **A coarse change says so.** A block replacement past eight changed lines is not a
  line-accurate diff, and a diff that quietly presents itself as one is worse than the prose
  it replaced. The pane says *"middle replaced as a block"*.
- **The clipping is the pane's**, so a long diff line cannot paint across the transcript.

`Transcript` now keeps the last 200 `Change`s. Bounded on purpose: this is a review surface,
not an archive, and the session log is the archive — it already holds every one.

Opening any file ends the review. The reader clicked a file in order to read it, and leaving
a diff on screen because they did would be the window ignoring its own click. **One way
back, not two** — the tree and the tab strip are already the answer, so the pane grows no
close button.

## A green test that was about the wrong control

`clicking_a_tab_switches_the_editor_to_that_file` passed from M248. It was not clicking a
tab.

`FrameSummary::hotspot` returns the **first** match, and the file tree records the same id
for the same path as the tab strip does. The test asked for `hotspot_id("alpha.rs")`, got
the **tree row**, and clicked it — which opens alpha.rs, so the assertion passed, while the
control named in the test's own name was never touched.

Giving tabs their own `tab:<path>` id is what exposed it. With the test pointed at the real
tab it **failed**, and the failure was a genuine defect rather than a bad assertion:

```
left:  [("alpha", false), ("beta", true)]
right: [("beta", false),  ("alpha", true)]
```

egui hands a widget its click result *while drawing it*, so the strip was drawing `beta` as
current, the click was then applied, and the body switched to alpha — **one frame in which
the pane contradicts itself**, which is worse than either half of it being wrong alone.

The fix is the same data M248 already had, read in the opposite order. `pressed_tab` tests
the pointer against **last frame's** tab rectangles *before* the strip is drawn, so the file
is open and the marker agrees by the time it is drawn. M248 read the frame's hotspots
**after** the pane that fills them and therefore always got an empty answer; the shape was
right then, the order was wrong. This is worth more than the bug: **when a mechanism
returns the answer, check where you read it relative to where it is produced.**

## The risk M249 left, measured

M249 stopped the composer stealing the keyboard, which could have meant a clicked button
took it instead. Both kinds of clickable row are now asserted — a tree row (a bare click
allocation) and a tab (a real `egui::Button`) — followed by typing. **Neither takes the
keyboard.** The risk was not real, and it is now a guard rather than a note.

## The disk knob, now earning its keep

M250's battery ran with `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0`, and
the log says so on its first lines — the M249 change working on its first real run:

| gate | with debug info | without |
|---|---|---|
| clippy | 1,058–1,273 s | **46 s** |
| test | 1,696–1,709 s | **381 s** |
| demo | 290–370 s | **149 s** |

A battery that took ~50 minutes takes ~11. The price is a panic backtrace in a failing test
that is close to useless, so the round that debugs a failure must drop the two variables —
and now the log will say which way it was run.

## Verification

Battery `m250` — **GATE PASS on attempt 1: 1,813 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m250.log`). Up from 1,808. `epsilon-desktop` is 97 unit + 21 window.

**Negative control:** with the changed-row click not applied, all three review tests fail
(`clicking_a_changed_row_opens_the_change_for_review`, `a_coarse_change_is_labelled_as_one`,
`opening_a_file_ends_the_review`); the other 94 pass.

**A test corrected rather than adjusted.** `clicking_a_tab_switches_the_editor_to_that_file`
was **failing** once pointed at the real tab, and the fix went into the product, not the
assertion. The general rule: when a test starts failing after a change that should not
affect it, the first question is whether the test was ever exercising what it names.

**Reinstall:** none. `epsilon-cli` and `epsilon-tui` are untouched.

## What nothing exercises

- **The diff pane has never been seen.** Every claim here is an offscreen-`Context`
  measurement of egui's real draw path, not a GPU rasterizer and not a screenshot. Added and
  removed lines differ in colour, but no test can say they are *legible* — that needs eyes.
- **No reviewed/approved state.** OpenCode's diff viewer tracks which files the reader has
  reviewed; this one has no such notion, so "what is left to look at" is not answerable.
- **Only the newest change per path is retained** in practice: `push_change` appends, so a
  file edited three times offers three rows and the last one clicked wins. Which change a
  row refers to is decided by matching the row's summary text, so two identical summaries
  would collide — possible, and not tested.
- **`row_rects` still does not cover the tasks and sessions panes.**
- **No filesystem watcher.** The tree refreshes on the agent's own change events only; an
  edit made in another window still does not appear.