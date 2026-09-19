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

## Open threads

- Overlay chrome is done for the palette, approval dialog, auth prompt, slash
  popup, both model-picker stages, the `/sessions` browser and the timeline: all
  of them now go through `ui::overlay` (`Panel`, `list_row`, `hint_row`,
  `Width::PANEL`). Two surfaces still draw through the `crate::theme` shim and a
  hand-built box: `draw_at_file` (the `@` file popup) and `draw_auth_picker` (the
  `/connect` provider list). `draw_transcript`'s streaming tail still needs the
  shim for two styles, too.
- Capacity arithmetic when moving a `Block` panel onto `Panel`: `Panel::inner` is
  two rows shorter than a `Block::inner` of the same frame (title row + bottom
  rule), and it insets two columns a side, so the old `area.height - 4` window
  becomes `inner.height - 2` — that keeps the visible row count identical, which
  is what the pickers' `scroll_start` window depends on.
- Gate note (2026-09-19): HEAD was **not** `cargo fmt --all --check` clean under
  this machine's rustfmt 1.9.0-stable/1.97.1 — 33 sites across 11 files
  (epsilon-core, epsilon-tui `ui/`, `app.rs` tests). Running `cargo fmt --all`
  normalizes them, so its diff carries unrelated reflow; the pristine-HEAD probe
  is `%TEMP%\eps-pty-head-fmt-check.txt`.
- `app.rs` is ~7000 lines and owns the event loop, key handling and the legacy
  draw functions. Splitting it (`event.rs`, `update.rs`, key handling per mode)
  is the structural half of this work and is not done.
- No terminal-integration test runs in CI yet. `.tmp-console/conprobe` (real
  conhost, drives the installed binary) and `.tmp-console/ptyharness` (ConPTY +
  vt100 emulator) are working local harnesses; they should become a checked-in
  test target rather than living in a scratch directory.
