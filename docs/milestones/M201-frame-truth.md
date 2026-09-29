# M201 — every part of a status row is whole, and the first screen says what to do

**Status:** landed. GATE PASS `m201` — see [Verification](#verification) for the measured
counts. Reinstall: **required** — every change here is visible in the TUI the owner runs.

## What

Seven rows of the audit's ranked table (`audit/03-defects.md`, the 2026-09-29
reconciliation) plus one defect found while fixing them. All of it is the same class:
**a screen that is wrong in a way no state-level test can see.**

| Row | Was | Now |
|---|---|---|
| **A1** | The draw path lost a character at the wrap boundary. At 60 columns "…in the workspace and **then** read…" rendered as "…and **the**", and at 80 the final `.` of the answer was gone. | `wrap_line` counted the separator it was about to flush *after* deciding the word fit, so a row could be emitted one column over budget and the renderer clipped its last cell. Measured at body width 58: a 59-column row, pushed to 61 by the two-column gutter in a 60-column frame. |
| **A2** | The header printed `idle 0 ↓0` at 60 columns — a bare `0` where `↑0 ↓0` had been. | See *the two defects the frames found* below: the first fix was wrong, and the frame said so. |
| **A3** | The footer printed `… pagedown scroll forward · ct …\Epsilon\crates\epsilon-tui` — `ctrl+enter` cut to `ct`, abutting the working directory with no gap. | The hints are budgeted before the layout runs and emitted whole (`key action`, or the key alone, never a fragment of either), and the budget reserves the same one column `side_by_side` does. |
| **A4** | The empty state repeated the header: `model Faux (offline scripted)` and `?/1.0M` appeared in both, two rows apart, with twenty blank rows underneath. | The header owns the model and the context. The panel stopped restating them. |
| **A5** | Three left edges (`E P S I L O N` at column 1, `model` at column 3, the instruction at column 3) and an instruction drawn in the same style as the data above it. | One left edge, one instruction in a body weight, and the affordances nothing else states. |
| **A6** | The composer's placeholder was drawn with the notice bullet: `• ask anything`. | The prompt glyph (`❯`), which is also what a submitted turn keeps — the row the user types into is the row their message becomes. |
| **A13** | The `/help` legend described `42%/200k` and `cache 87%`; the header has printed neither since the `ctx` label and the cache volume landed, and the three tokens a reader cannot decode (`ctx ?`, `CH n/a`, the trailing `offline`) were in no line at all. | The legend moved into `ui::metrics`, beside the formatters it documents, with a test that *generates* each placeholder from the same fixture and asserts the legend quotes it. |

## The frame, before and after

`audit/evidence/frames/m199-baseline/` and `audit/evidence/frames/m201/`, both written by
`app/frame_dump.rs` (three fixtures × three sizes).

**60×20, brand new session — before:**

```text
|idle 0 ↓0 · R0 W0 CH n/a · Faux (offline scripted) · offline|
|E P S I L O N                                               |
|a coding agent in your terminal                             |
|  model    Faux (offline scripted)                          |
|  context  ?/1.0M                                           |
|  ready — describe a task and press enter                   |
|• ask anything                                              |
|insert · enter send · pageup sc …\Epsilon\crates\epsilon-tui|
```

**after:**

```text
|idle      ?/1.0M · ↑0 ↓0 · Faux (offline scripted) · offline|
|E P S I L O N                                               |
|a coding agent in your terminal                             |
|Describe a task, then press enter.                          |
|@ files · ! shell · / commands                              |
|❯ ask anything                                              |
|insert · enter send · pageup    …\Epsilon\crates\epsilon-tui|
```

## The two defects the frames found that reading had not

**The header's pin was keyed on a role that changes.** The first version of the whole-gauge
rule dropped leading fragments, which kept the model (last) and lost `ctx ?/1.0M` — how much
room is left, the number a reader watches — while keeping `R0 W0 CH n/a`, a gauge that says
nothing at all when the provider cannot report a cache rate. Rewriting it to drop the *tail*
and pin the model then produced:

```text
|idle ?/1.0M · ↑0 ↓0 · R0 W0 CH n/a · Faux (offline scripted)|
```

— `offline` dropped, the model next in line — because the pin was found by
`MetricRole::Model`, and `Metrics::parts` draws an **offline** model with `MetricRole::Warn`.
The pin therefore stopped matching in exactly the sessions whose row is longest. The parts
list was correct throughout; every state-level assertion in the file passed. The pin is now a
field on `MetricPart` (`pinned`), and the regression is pinned by
`an_offline_session_keeps_the_model_and_its_qualifier_when_the_row_narrows`.

**A test whose verdict depended on the environment.** `markdown.rs`'s tests took
`Theme::detect()`, which reads `NO_COLOR`, `COLORTERM` and `EPSILON_ASCII` from whatever
runner happens to be in the room. With `NO_COLOR` set — which is how this repository's own
machine is configured — every role collapses to `Style::new()`, so
`a_table_becomes_columns`, which asserts the header row is *visibly distinct* from the body,
failed for a reason that had nothing to do with tables. Twelve tests now build a theme with
every role filled in, chosen by the test. Found because the battery was run with the
environment a contributor actually has, not because a test named it.

## Two smaller corrections

- **A word wider than the whole row was appended to whatever the row already held.** The
  hard-split path measured each chunk against the full budget while drawing it into the
  leftover, so the row came out as wide as the text before it plus a full row; at width 1 a
  row read `I w`. It now starts a fresh row, which is also what makes the wrap point eat the
  trailing space the way the word path does.
- **`side_by_side` lost its `whole_span` parameter.** The footer protected its position badge
  after the cut; it now budgets for it, so "whole or absent" is structural rather than
  post-hoc — and the parameter, its 26-line helper (`cut_keeping_whole`) and the
  layout-level test that exercised it are gone with it. The property they guarded is still
  asserted, at the caller, by `the_position_badge_is_never_half_written_at_any_width`.

## Limits — what this does not prove

- **A hint may lose its words.** At widths where `pageup scroll back` does not fit whole, the
  row prints `pageup` — a real key, honestly named, one word short of an explanation — rather
  than dropping the hint. The alternative loses the hint entirely, and on the frozen row that
  hint is the only thing on screen saying there is a way back to the live end. The rule is
  "never a fragment of a key or an action"; it is not "never fewer words".
- **Below 44 columns the layout's clamp is still the backstop** and can cut inside a word.
  Nothing is drawn below 44×10 — the frame reports itself too small — so the cuts are
  unreachable in the product and exist to keep the arithmetic total in tests.
- **A9 (`.gitattributes`) is not in this slice.** It is repo hygiene rather than a screen,
  and it must not be done casually: adding `* text=auto` marks every file for
  renormalization, and the next `git add` would then rewrite line endings across the tree in
  one commit. It wants its own commit where the diff can be checked to be line endings only.
- **A7 (a killed TUI blocks the workspace for 30 s) is untouched.** The CLI shows a
  countdown; the TUI enters the alternate screen and shows nothing. That is a startup-path
  fix, not a frame fix.

## What nothing exercises

- **No test drives a real terminal for any of this.** The frames are `TestBackend`
  renderings. `crates/epsilon-cli/tests/terminal.rs` proves the binary reaches a real
  pseudo-console; it does not read the rows back at 60 columns.
- **The `pageup`-without-its-action shape is asserted as legal, not as desirable.** The
  footer sweep asserts that whatever appears is whole; nothing asserts that dropping an
  action rather than a hint is the better trade, because that is a judgement.
- **`EPSILON_ASCII` is not in the frame evidence.** The frames are rendered with the ASCII
  repertoire by `frame_dump`; the Unicode repertoire is covered by unit tests
  (`the_composer_prompt_is_not_the_notice_bullet` runs both) and not by a frame.
- **The `/help` legend is asserted against `Metrics`, not against the screen.** A notice row
  that wrapped or was dropped would not fail anything.

## Verification

GATE PASS, 2026-09-29 — battery **`m201`**, run against this commit's contents with the
documentation already final. Seven gates, exit 0 each: `check-links` 0 s (428 relative links,
0 broken), `check-lint-tables` 0 s, `fmt --check` 3 s, `clippy --workspace --all-targets` 3 s,
`test --workspace` 512 s, `demo` 12 s, `real-check` 10 s.
**1,577 tests across 63 binaries, 0 failed** — summed from the 63 `test result: ok.` lines in
the log, not read off a previous run's figure. M199 measured 1,568 across the same 63
binaries, so this slice adds 9.

Evidence: [`audit/evidence/gate-m201.log`](../../audit/evidence/gate-m201.log) (the full battery)
and [`gate-m201.md`](../../audit/evidence/gate-m201.md) (the summary and what it does not cover);
frames before and after under `audit/evidence/frames/m199-baseline/` and
`audit/evidence/frames/m201/`.

The first battery passed, which is worth stating plainly because it is unusual here — and it
is partly luck of ordering: two of the defects this milestone found were found by *reading a
rendered frame*, and one of them (the role-keyed pin) was a bug I introduced in the first
correction and removed after looking at the 60-column row it produced. The `m199-baseline`
frames were the only reason either was visible.

**Reinstall:** required. Everything in this slice is reachable from the installed binary —
the header, the footer, the composer, the welcome screen and `/help` are all on screen from
the first frame — so `cargo install --path crates/epsilon-cli --features real --force` was run
after the battery passed.

**One gate is a doc gate and one is a lint-table gate**, and `test --workspace` includes the
CLI's `include_str!` checks over `docs/history.md`: a documentation change in this repository
can fail a test. That is why the history rows and the doc map were written *before* the
battery rather than after it.
