# M127 — a light theme, and a transcript that says when it is frozen

Two more items from the TUI design audit: **D8** (body text was invisible on a light terminal) and
**D11** (scrolling up during a run was silent, with no way back).

## What

**A light palette, selectable.** `ui/theme.rs` defined only `Palette::DARK`, `role()` hardcoded it,
and `text`/`assistant` were `Color::White` — which this project's own standard forbids in as many
words ("Do not use `Color::White` or `Color::Black` as defaults. They fight the user's terminal
theme"). On a light terminal the body text was near-invisible. There is now a `Palette::LIGHT`, a
theme mode carried on the theme, and a switch (`--light`, `EPSILON_THEME`) to select it.

**The transcript says when it is frozen.** PageUp during a run used to freeze the view silently;
`G` did nothing; nothing anywhere said where you were. The footer now carries the frozen/live state
and a position badge, and jump-to-live is bound through the **live binding table** so its hint cannot
drift from its handler.

## Key decisions

- **A light palette plus a switch, not runtime background detection.** This is the skill's explicit
  recommendation and the shape pi and opencode both use (a dark/light pair with a configured mode).
  Guessing the terminal's background is the failure mode that makes both palettes look wrong.
- **The badge's test reads cells.** `the_badge_is_drawn_onto_the_row_with_the_warn_colour` asserts the
  badge reaches the *cells* of the drawn row with `warn` styling. Without it, deleting the badge from
  `draw` would fail only string-level tests — which is precisely how M123's decoration was "built,
  tested, never drawn" for months.
- **The switch is part of the feature, not follow-up.** A palette nothing can select is the same
  defect shape, so the CLI flag and the event wiring were committed with it.

## Verification

Battery `m128a`: **all five gates exit 0, 1,148 passed / 0 failed / 2 ignored across 61 binaries**.
`epsilon-tui` at **283 tests** (was 252 before M126). The author additionally reported
`cargo test -p epsilon-cli` ok, clippy clean on both crates, `--demo` exit 0, and
`--light --provider faux -p "say ok"` exit 0.

## Limits

- **No author negative controls are recorded here.** The author was still writing them up when the
  slice was committed; the badge and flag tests exist, but the revert-and-watch-it-fail evidence does
  not. The independent read-only review is owed too.
- **The header's own colours were not re-audited under the light palette** — the body text, statuses
  and badges were, and the metrics row was checked for distinguishability rather than redesigned.
- **`NO_COLOR` glyph separation for the remaining roles is still open** (the audit's last item), as is
  **OSC 52** for `/copy` over SSH, where the command currently reports success while copying nothing.
- **One commit lesson, recorded because it nearly shipped:** `HEAD` was not self-consistent for a
  round — the dirty `main.rs` carried a `let_and_return` fix, so the clippy gate would have failed on
  the commit alone, while the passing battery built the *working tree* that contained it. Fixing it
  meant committing the file, not re-running the battery.
