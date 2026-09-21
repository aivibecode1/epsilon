# TUI redesign: chrome economy

**Status:** presentation layer landed and green (213 `epsilon-tui` tests). The
overlay chrome migration is complete for every list panel and prompt but sits
**uncommitted** in the working tree. Owner priority: the TUI looked dated. This
note records the design rules and the traps found while building it, so the next
slice does not rediscover them.

## The design rule

The dated look is not about the widget toolkit. It is nested boxes: a frame
around the screen, a box around the composer, a box around each turn, a label on
every box. All three reference TUIs (pi, opencode, hermes) refuse it. At most
**one** border may sit between the terminal edge and any content.

Separation comes from, in order of preference:

1. **A fixed two-column gutter** (`> ` user, `  ` assistant, `| ` thinking,
   `* ` tool call, `+ ` ok, `x ` error, `* ` notice). This is the primary device
   and it is the one that survives `NO_COLOR` and a 16-colour terminal.
2. **A full-width background band**, truecolor only, reserved for the user turn
   and tool cards. Invisible on a plain terminal rather than approximated.
3. **Whitespace**, computed once per region instead of once per message.

Colour is meaning, never decoration: one accent, three semantic states, and dim
for every piece of metadata. `NO_COLOR` is honoured, and "dim" never relies on
SGR faint.

## Layout

```
header      1 row    run state on the left, live measurements on the right
rule        1 row
transcript  fills    conversation, tool cards, notices, or the first-run screen
rule        1 row
composer    1..8     grows with the draft, capped at a third of the screen
footer      1 row    keys that work right now, then location · branch
```

Floor is 44×10; below that the screen says so instead of drawing a mangled frame.

## Where the numbers live

`ui/metrics.rs` is the visibility feature, and it is a data model rather than
three lines in a draw call:

- **Context**: `context_used / context_window`, with warn at 70% and danger at
  90%. Unknown prints `?`, never a fabricated zero.
- **Cache hit rate**: cache reads divided by *the prompt they belonged to*, not
  by a session total. A session average falls every time the model answers, which
  is the opposite of what the reader is watching for. A prompt under 1000 tokens
  reports `—`: a 120-token "hit" proves nothing.
- **Cost**: integer micro-USD throughout (ADR-010), priced from the catalog.
  An unpriced model omits the field rather than claiming `$0`.

## Traps found the hard way (all now covered by tests)

- **Character counts are not columns.** `⠹`, `┌` and `界` are each one `char`
  but one, one and two columns. Every width in `ui/` goes through
  `layout::display_width`. Using `chars().count()` made rows one cell too wide
  and they wrapped.
- **`Cell::symbol()` is not a character.** ratatui coalesces runs of identical
  adjacent cells into one cell whose symbol repeats, so a row containing `100%`
  reads back as a cell whose symbol is `%%`. Test helpers must append the whole
  symbol and step by its display width (`layout::row_text`).
- **`''.chars().all(..)` is vacuously true**, so "count the rows that are only
  dashes" counted every blank row too. Tests assert on non-empty rows.
- **Trim from the far end.** A status row is written most-important-last, so the
  half that yields is trimmed at the end touching the other half. Trimming the
  near end keeps `12k` and loses `DeepSeek V3.1` — the one fragment that says
  what every other number means.
- **`clamp_line` is the structural backstop** for "this row must never wrap".
  Two width tables (ours and ratatui's) can disagree by a column on rare code
  points; clamping on the way out makes wrapping impossible rather than unlikely.

## The owner's "shrink and freeze", measured (2026-09-21)

The report is in the task brief: launching the TUI in CMD or PowerShell "shrinks" the
terminal and freezes it, with nothing typed or streamed ever appearing. The ConPTY test
in `crates/epsilon-cli/tests/terminal.rs` **cannot** answer that question — under ConPTY
the console is emulated inside the terminal application, so no assertion it makes can
observe the console window being resized. So there is now a second harness,
`scripts/console-probe.py`, which opens a real conhost window, attaches to it, measures
the buffer and window geometry before and after the TUI starts, types real `KEY_EVENT`
records in, and reads the screen back.

**Result: no shrink, and typing works, in four geometries** — default 166×43, forced
100×30, 100×20 over a 9001-row scrollback, and maximized 168×44. The *window rectangle*
never moves; the active buffer does lose its scrollback height, which is what an
alternate screen is for. Full table: `audit/evidence/console-probe-2026-09-21.md`.

The report was two real defects, both already fixed and both measured at the time: the
run loop redrew a whole frame per keystroke and per streamed delta (M94 — a 99-character
burst echoing at ~30 characters/second), and every Windows `bash` call loaded the user's
PowerShell profile, 6.2 s for a one-word `echo` (M98), which from inside the TUI is
indistinguishable from a deadlock. **Lesson kept: when the TUI "looks frozen", measure
before reading rendering code.**

One defect did reproduce while probing, and it is open:

- **A killed instance locks the next launch out for up to 30 s.** The session lease
  (`crates/epsilon-session/src/lease.rs`) calls a holder dead only when its lock file is
  older than `STALE_AFTER` (30 s, against a 10 s heartbeat). Close the console window or
  End-Task the process, relaunch within that window, and the second launch prints
  `another epsilon is already writing this session (pid=…)` and **exits**. Reproduced:
  the 80×25 probe run refused to start because the previous run's killed TUI still held
  the lock. The fix is a design decision, not a patch — a pid liveness check needs a
  platform call and this workspace forbids `unsafe` outside a reviewed per-crate
  override, so the alternatives are a narrow liveness probe or a refusal that waits the
  window out with a countdown.

## Open threads

- **The `crate::theme` shim is deleted** (commit `refactor(tui)`), and every
  surface reads one passed-in `Theme`: `md_to_lines` and the render cache take
  `&Theme` as an argument, which is also what makes them testable in more than
  one colour mode. Do not reintroduce a free-function palette.
- **`app.rs` is 3,594 lines** (from 6,700) and the split is done. Beside it:
  `app/keys.rs` (the keyboard model, 1,191), `app/event.rs` (the loop, 226),
  `app/golden.rs` (whole-screen tests, 206), `app/tests.rs` (1,545),
  `app/composer_viewport_tests.rs`, `app/tests_support.rs`. Every one compiles as
  a child of the same module, so `super::*` still reaches private items — **keep
  that property, it is what makes extraction free.** `app.rs` is now state, the
  draw path, and the adapters that feed the shell.
- When extracting from `app.rs`, two things repeatedly need care: a moved item
  used by the *parent* needs `pub(crate)` plus a re-export (the compiler's
  "private, and cannot be re-exported" error tells you which), and state that
  parent code touches (the OAuth event type, the `LoginUi` impl, `clipboard_set`)
  should stay with the state rather than travel with the functions that happened
  to sit beside it.
- Capacity arithmetic when moving a `Block` panel onto `Panel`: `Panel::inner` is
  two rows shorter than a `Block::inner` of the same frame (title row + bottom
  rule), and it insets two columns a side, so the old `area.height - 4` window
  becomes `inner.height - 2` — that keeps the visible row count identical, which
  is what the pickers' `scroll_start` window depends on.
- **`cargo fmt --all -- --check` is clean as of `style:` commit 46c4ead**, which
  normalized 33 sites of pre-existing drift. If it fails again, check whether the
  drift is older than the change you are looking at before assuming it is yours.
- **Terminal coverage now has two halves.** The ConPTY test
  (`crates/epsilon-cli/tests/terminal.rs`) drives the built binary and asserts on an
  emulated screen; `scripts/console-probe.py` drives it in a real conhost window and
  measures the window itself. Between them, "the TUI renders nothing" and "the TUI
  resized my terminal" are both answerable. The old scratch harnesses
  (`.tmp-console/{conprobe,ptyharness}`) are gone; the probe replaced them.
- **`@`-file completion still does not honour `.gitignore`**, and `audit/11`'s G5 row
  claims it does. `App::at_file_rows` (`crates/epsilon-tui/src/app.rs`) keeps its own
  recursive walker and its own hardcoded skip list (`.git`, `target`, `node_modules`,
  `dist`, `.epsilon`), so `@` offers files the tools would refuse to read.
  `epsilon-tui` already depends on `epsilon-core`, so the fix is to call
  `epsilon_core::ignore::{IgnoreSet, skip_dir}` from that walker — no new dependency
  and no new crate edge. Queued as its own slice; the audit says "queued", not "done".
