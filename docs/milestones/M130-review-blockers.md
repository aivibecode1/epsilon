# M130 — the independent review's four remaining blockers

The review `audit/evidence/m126-m127-review.md` found five blockers in the two TUI slices that landed
without one. B1 (the card duration) was fixed in M129; this closes B2, B3, B4 and B5 — and the third of
them turned out to be hiding a production bug.

## B2 — the light palette's ANSI-16 roles were the dark palette's

The light table overrode only `rgb` for the quiet roles, leaving `border`, `text_dim`, `text_faint`,
`thinking` and `code_block` at `Color::DarkGray` — **the same value as dark**, directly under a comment
saying these are "never `White`, never `DarkGray`, which is the pair that disappears on a light theme".
`ColorMode::Ansi16` is the **default** on a console that does not advertise itself, which is exactly the
user `--light` exists for, so secondary text went out as bright-black on white (xterm's `#808080`,
**1.4:1**).

| role | before | after |
|---|---|---|
| `border`, `text_dim`, `text_faint`, `thinking`, `code_block` | `DarkGray` (1.4:1 on white) | `Reset` + `DIM` — the terminal's own foreground, dimmed |
| `border_focus` | `Cyan` (1.9:1) | `LightBlue` (4.7:1) |

**Two limits are now asserted rather than implied.** ANSI-16 has no legible green or yellow on white
(`Green` 2.2:1, `Yellow` 1.7:1, and the "bright" variants are `#00ff00`/`#ffff00`), so `success`, `warn`
and `code` keep their hue and lean on glyph and word — which the skill already requires — and the test
**asserts exactly those three are under the floor**, so a fourth would be a decision rather than a silent
drift. The author's first attempt used `LightGreen`/`LightYellow` as "the dark members of the hue" and
the test's own contrast arithmetic refuted it: `Light*` is the bright half.

The test now reads `.ansi` for **every** role in **both** palettes, with coverage checked in both
directions (a role the table misses fails; a role the test invents fails).

## B3 — an unrecognised `EPSILON_THEME` fell through to the config file

`resolve_theme_mode` failed `from_name` and then consulted `[tui] theme`, so `EPSILON_THEME=solarized`
with a config saying `light` produced a **light** session — the opposite of what `--help` and two doc
comments state. It now falls back to dark, which is both the documented and the better behaviour: a user
who typed `solarized` meant to say something about the palette, and silently getting the *other* palette
because of a config key they forgot is the failure `--light` exists to fix. Unset and set-to-garbage are
different states, and only the first should consult the next source.

## B4 — a doc comment that contradicted its own pinned test

`parse_diff`'s comment claimed a one-for-one replacement is numbered `5`/`6`; the code numbers the
addition *before* incrementing so the sides **agree**, which is what the sibling test and the golden
frame assert. A future reader "fixing" the parser to match the prose would have broken a pinned test. The
comment now states the real rule and names both tests that pin it.

## B5 — the split diff had never been drawn, and drawing it found a bug

At `width >= SPLIT_MIN_WIDTH` (120) the two halves were **landing on top of each other**: `- 2 old = 1;
+ 2 new = 2;` was a **25-cell row instead of 138**, the right half 13 columns after the left instead of
69, and the blank "no counterpart" half gone.

Cause: `card::rows` padded each half to its column width and then re-wrapped the padded row, and
`wrap::wrap_line` **drops trailing whitespace by design** — right for prose, fatal for a layout whose
alignment *is* the padding. `RenderedLine::build` then deleted it a second time. A row that already fits
is now not re-wrapped, and unified rows no longer carry split-only padding (`pad_to_cell` vs `pad_to`).

**The same root cause was visible at 80 columns first**, which is how this arrived as a regression: with
the row no longer re-wrapped, the intra-line highlight's prefix span (the line's own four-space
indentation) survived where `wrap_line` had been eating it by accident, so a diff would have printed the
code indented twice — once from the file, once from the row's number column. Fixed by dropping the prefix
whitespace in `intra_pair` explicitly. The pinned layout is byte-identical again and **no assertion was
moved**: the assertion was right and the code was wrong, and only a test that renders at 140 columns
could see it.

## Verification

Battery `m130b`: **all five gates exit 0, 1,169 passed / 0 failed / 1 ignored across 61 binaries**.
`epsilon-tui` 302 tests; `epsilon-cli --bin epsilon` 13, green under `EPSILON_THEME` unset, `solarized`
and `light`. `m130a` failed clippy on a run-time assertion between two constants in the new split test —
now a **compile-time** assertion, so a lowered `SPLIT_MIN_WIDTH` fails the build rather than one test.

Three negative controls, each announced before arming, short-lived, restored and touched: reverting the
light quiet roles fails `the_ansi_column_is_checked_for_every_role_in_both_palettes`; reverting the theme
wiring fails `the_environment_value_decides_including_a_malformed_one`; reverting the split fix fails
`the_split_diff_draws_both_sides_at_a_wide_frame` ("the right half's body must begin exactly 69 columns
after the left half's — left: 19 / right: 75").

## Limits

- **B3's discriminating test needs `EPSILON_THEME` set.** A test process cannot write the variable
  (`unsafe_code` is `forbid` on the command line and the compiler rejects a local `allow`), so the wired
  test bites only under a set variable; the recipe is in its doc comment. Closing it properly needs config
  injection — `load_config_string` already has an `_in` variant, so `resolve_theme_mode` could take the
  config directory, which is a public-API change and was not made unasked. Worth recording that the
  author's *first* version of this test asserted only the pure helper and **passed under its own negative
  control** — the gap was found by arming the control, not by reading the test.
- **B4 has no test guard of its own.** A comment cannot have one; the property it describes was already
  pinned in two places, which is why the old prose was dangerous.
- **ANSI-16 light has three roles below the contrast floor** (success, warn, code) — asserted, not fixed.
- **One restore silently did not land** during the author's controls (it had copied the armed file over
  its own backup) and was caught by grepping the content rather than trusting the copy. Worth knowing:
  a restore that reports success is not a restore that happened.
- The review's findings **F1–F7 remain open** (footer hint-yielding at 60 columns, the duplicated
  card-open precedence, unused glyphs, the `zz_*` artefacts, doc hygiene, `fg_of` and `Color::Reset`).
