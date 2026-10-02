# M263 — a band painted across the file tree, and a budget in the wrong unit

Two findings from the same parallel review sweep, sharing one shape: **a value computed
against the wrong reference.** Once it was the wrong pane, once it was the wrong unit, and
both were invisible to a suite that was measuring something else.

## 1. The goal band was painted at x = 0

`draw_goal_band` took a **width** and built its rectangle at `egui::pos2(0.0, top)`
(`desktop/src/app.rs:1907`). `Painter::with_clip_rect` *replaces* the clip rather than
intersecting it, so the pane's own clip did not contain it either.

Measured, with the negative control rather than the arithmetic:

```
the goal band was painted at [[0.0 97.0] - [520.0 131.0]],
                    outside the agent pane [[920.0 42.0] - [1440.0 870.0]].
```

At 1440×900 with the sidebar open, the agent pane starts at x=920 — so a 520-wide goal strip
landed on the activity bar, the file filter box and the first tree rows, drawn *after* them.

The fix is the pane's `left()` instead of a zero, and one parameter instead of a bare width.

### Why 104 desktop tests did not see it

**The containment sweep already iterates every `row_rects` entry and would have caught this on
sight** — the band records its own rectangle at `app.rs:1970`. Its fixture seeds eight user
rows and **no goal**, so `draw_goal_band` returns `None`, and the band is never in `row_rects`
to be caught.

> A green sweep over a fixture that cannot produce the thing you are worried about is not a
> green sweep.

The new test seeds a real goal through a real session, because the harness that most tests use
(`IdeApp::without_agent`) cannot have one. That is also why this defect lived entirely in
`tests/window.rs` territory and not in the unit tests.

## 2. Truncation that budgeted in characters where the screen budgets in columns

`state::truncate` is the one helper every card headline goes through, and it counted `char`s.
So did the timeline preview, the approval dialog's detail summary, the tree filter echo, the
label field and the `/tasks` preview.

Two things are wrong with that, independently:

- **The unit.** A hundred Japanese characters are two hundred columns, so each of those rows
  overflowed the pane the preview was built to fit — the opposite of what a truncation is for,
  and invisible until someone names a file in Japanese.
- **The boundary.** `chars().take(n)` can cut a combining sequence in half or leave half a
  wide glyph behind.

**For ASCII this is byte-identical to the old behaviour**, which is why 261 milestones of card
tests never saw it: every fixture in the suite is ASCII, where one column is one character.

> A defect in the wrong unit is invisible to every fixture written in the other one.

### One helper, both marks

`…` is one column on a modern terminal and **not representable at all in CP437**, where the
ASCII repertoire passes `...` — three cells against one. That difference already cost the
footer a hint at M220, so the mark is now *measured*, never assumed: `fit_width_with(text,
budget, mark)` reserves `display_width(mark)`, and `fit_width` is the one-column special case.
`state::truncate` is a call to it, so there is one truncation policy rather than two.

### A candidate that was checked and rejected

M261 named `frame_dump.rs` as a likely source of CP437 trouble because it measures overflow
with `chars().count()`. It does not: `render_rows` reads ratatui **buffer cells**, and one
cell is one column. The instrument was already correct. Recorded so the next reader does not
"fix" it.

## Verification

Battery `m262` — **GATE PASS: 1,872 tests across 76 binaries, 0 failed**
(`audit/evidence/gate-m262.log`). Up from 1,859 at M261. `epsilon-desktop` is 136 (104 unit +
32 integration); `epsilon-tui` is 502 unit tests.

**The battery needed the Avira workaround, and it cost the honest log.** Attempt 1 failed with

```text
LINK : fatal error LNK1104: cannot open file
  'C:\...\epsilon-target\debug\deps\epsilon_core-4a8081d0361ce74c.exe'
```

which `agents/memory/05-machine.md` documents as Avira pinning a freshly linked test artifact
— a machine fault, not a code failure, and the documented remedy is fresh test-profile
metadata. **Two mistakes were mine.** I re-used the base label `m262` for the retry, and
`gate-until-pass.ps1` takes the **base** label on attempt 1, so `target/gate-m262.log` — and
with it the LNK1104 evidence — was truncated at the start of the retry. That is precisely the
rule `AGENTS.md` states twice, and the wrapper's own synopsis says the base label "stays free
for the definitive run". The failing linker line is quoted above; the full log is gone and is
not recoverable. `CARGO_PROFILE_TEST_DEBUG=1` also forced a full test-profile rebuild, which
is where the 2,402 s test leg went — against ~675 s for a warm run.

- `the_goal_band_is_drawn_inside_the_agent_pane` — reads the band's rectangle out of the frame
  and the pane out of the layout and compares them. **Negative control: fails with the exact
  measurement above** when the band is put back at `pos2(0.0, top)`.
- `a_wide_script_in_the_tree_filter_is_cut_and_says_so_on_the_frame` — on the **frame**, not on
  the helper, and scoped to the filter's own row, because an assertion over the whole screen
  would pass on a build where other rows' ellipses happened to be on it.
- `a_card_headline_is_cut_to_columns_not_characters` and
  `a_card_headline_reserves_the_columns_its_mark_actually_uses` — the one helper every card
  goes through, asserted once.
- `the_ascii_mark_costs_its_own_three_columns` — `abc...` in six, not `abcd...` in seven.

## What nothing exercises

- **The label prompt's caret branch is still unbounded.** `shown` is 60 columns of column-aware
  truncation now, but it is used **only** when the cursor is at the end; with the cursor
  anywhere else the code splits the *full* text around the caret and draws it uncut, so a
  pasted 500-character label still overflows its panel. A complete fix wants a
  cursor-anchored window, which is new code rather than a call site, so it is named here
  rather than half-done.
- **The desktop's own `fit()` budgets in points/em, deliberately** (`app.rs:529-535`) and its
  comment says why: it is an approximation that can only ever shorten. It is left alone.
- **`epsilon-cli`'s `doctor.rs:570` still truncates by characters**, on our own diagnostic
  output rather than user text. Noted, not changed.
- **Truncation that feeds the *model* was deliberately not touched.** `tools/webfetch.rs`,
  `tools/search.rs`, `llm/providers/mod.rs` and `core/lib.rs` cap output in characters on
  purpose: that is a token and context budget, and a column is not one.