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

- **The transcript window was measured from the wrong rectangle, and a control found it.** The fourth
  control — feeding `draw_transcript` the whole frame instead of the transcript region — failed two
  *pre-existing* tests, because the code really did slice a `Rect::height`-row window while painting
  only `transcript.height` rows. At 60×20 that is a 20-row window in a 16-row region, so a
  tail-following view sat four rows above the tail it claimed to follow **and painted over the
  composer's rows**. Fixed by `App::transcript_window` / `measure_transcript_window` — one place that
  knows the split the shell will perform. A control that fails an old test is a bug report, not a
  control artefact, and this is why running one against *neighbouring* behaviour is worth the minutes.
- **The header's own colours were not re-audited under the light palette.** The metrics row was
  checked for distinguishability only; `?/1.0M` is the audit's separate item 4, untouched.
- **No mid-session `/theme` toggle.** `Theme::set_palette` returns the palette it replaced, so a
  `/theme` command is a small follow-up — but repainting a full-screen band mid-session is a different
  review from choosing a palette at startup.
- **`NO_COLOR` glyph separation for the remaining roles is still open** (the audit's last item), as is
  **OSC 52** for `/copy` over SSH, where the command currently reports success while copying nothing.
- **One commit lesson, recorded because it nearly shipped:** `HEAD` was not self-consistent for a
  round — the dirty `main.rs` carried a `let_and_return` fix, so the clippy gate would have failed on
  the commit alone, while the passing battery built the *working tree* that contained it. Fixing it
  meant committing the file, not re-running the battery.

## Negative controls (four, each restored byte-identically and touched)

| Control | What failed |
|---|---|
| `Theme::colors()` hardcoded to `DARK` | `the_light_palette_changes_the_body_text_the_frame_draws`, `body_text_is_never_a_hardcoded_white_or_black`, `the_palette_is_selected_by_name_and_survives_a_bad_one` |
| the badge built but **not drawn** | `the_badge_is_drawn_onto_the_row_with_the_warn_colour`, `a_frozen_view_says_so_and_names_the_way_back`, `the_position_badge_is_the_only_thing_that_distinguishes_frozen_from_live`, `paging_up_a_running_view_shows_it_is_frozen_and_where` |
| the jump-to-live arm deleted from `handle_key` | `the_jump_to_live_key_returns_the_view_to_the_tail` |
| the viewport measured as the whole frame | `a_long_transcript_never_pushes_the_composer_off_the_screen` and `the_advertised_scroll_key_scrolls_the_real_frame` (**both pre-existing**) plus the two D11 frame tests |

The second is the one that matters most: without a cell-level assertion, deleting the badge from `draw`
would have failed only string-level tests — exactly how M123's decoration stayed built, tested and
never drawn.
