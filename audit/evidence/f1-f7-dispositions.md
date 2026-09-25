# The M126/M127 review's findings F1–F7: dispositions and controls

The review at [`m126-m127-review.md`](m126-m127-review.md) found five blockers and seven findings.
The blockers were closed in M129/M130. This is the F1–F7 slice: what was decided, what was measured,
and what was armed to prove the tests bite. Read-only review, so nothing here is the reviewer's claim
restated — each row says what the *code* was found to do.

**Verification of this slice:** `cargo test -p epsilon-tui` → `308 passed / 0 failed / 0 ignored`
(303 before it), `cargo check -p epsilon-cli` clean, `cargo clippy -p epsilon-tui --all-targets
-- -D warnings` clean, `rustfmt --edition 2024` applied to the changed files only. The workspace
battery is the parent's to run.

## F1 — the footer comment promised 60-column survival; the arithmetic needed measuring

**Measured, not argued.** A throwaway integration probe rendered the real frozen row (mode, badge, two
hints, location, branch) through `ui::footer::draw` at every width from 44 to 100, for five workspace
paths. The rows below are verbatim from it; the probe file was deleted in the same turn it was written.

With `~/Desktop/My Agent Space/Epsilon · main` (39 columns) on the right:

| width | row |
|---|---|
| 44 | `inse ~/Desktop/My Agent Space/Epsilon · main` |
| 50 | `insert ^ 1 ~/Desktop/My Agent Space/Epsilon · main` |
| 60 | `insert ^ 12/40 · g j ~/Desktop/My Agent Space/Epsilon · main` |
| 80 | `insert ^ 12/40 · g jump to live · pageup ~/Desktop/My Agent Space/Epsilon · main` |

Two things follow, and the review was half right about both:

1. **The badge does not disappear at 60 columns** — the review's stated failure. It is whole, and it
   is whole with no protection at all: the protected-span machinery can be disarmed entirely (control
   1, below) and this row is unchanged. What disappears at 60 columns is the *exit hint's own words*
   (`g j`), which is what the comment got wrong: it said the position and the exit survive together.
2. **The defect is one width-band lower, and it is worse than a missing badge.** At 50 columns the row
   reads `^ 1` — the numerator of `12/40` with the denominator gone, which is not less information but
   *wrong* information, and the class this project has been bitten by before (a value that looks
   measured and is not). Measured with the protection disarmed, the band is 26–31 columns:
   `^`, `^ 1`, `^ 12`, `^ 12/`, `^ 12/4`.

**Decision.** Three changes, because the comment could not be made true without a fix:

- `ui::layout.rs`: `side_by_side` gains `whole_span: Option<usize>`. The span at that index is emitted
  **entire or not at all**; everything else cuts as before. The reserve happens before the two halves
  are decided, so a location cannot starve the badge out of a row wide enough to hold it. `None`
  (the header's case) is byte-identical to the old behaviour.
- `ui::footer.rs`: `draw` protects the badge span, and the `Hints::frozen` doc comment now states the
  three-step survival order and the measured consequence at 60 columns instead of the claim that was
  false.
- Tests: `the_position_badge_is_never_half_written_at_any_width` sweeps 26..=60 and asserts the badge
  is whole or absent — never partial; `the_frozen_row_at_sixty_columns_keeps_the_position_and_the_exit`
  pins the row a reader at 60 columns actually gets (badge whole, `scroll back` gone, exit key
  present); `a_protected_span_is_kept_whole_or_dropped_whole` pins the layout rule itself.

## F2 — the card-open precedence was implemented twice

`App::card_is_open` (what a keypress records) now builds the same `RenderState` the draw path builds
and defers to `RenderState::is_open` (what is drawn). One rule, one place.

Two tests, because the two facts are different:

- `the_drawn_and_recorded_answers_to_an_open_card_are_the_same_answer` sweeps every input the rule
  depends on (both values of the global toggle × three per-card entry sets × every line in a fixture
  that holds both a card with a body and a row without one) and asserts the two answers are equal.
- `the_card_open_rule_is_borrowed_and_not_restated` reads `app.rs` and requires `card_is_open` to call
  `.is_open(id, has_body)` and not to contain an `unwrap_or` default of its own.

The second exists because of what the control showed, and this is the finding's real lesson: **the
agreement test passes for a second copy of the rule too.** With the original duplicate restored
verbatim (control 2), the agreement test is green — two implementations that agree cannot be told
apart by running them, which is exactly why the copy survived a whole slice and a review. The
structural guard is what fails on the duplicate.

## F3 — `Glyphs::warn` and `Glyphs::caret` have no reader

**Deleted, in both repertoires and in the struct.** Verified before deleting: `grep` over `crates/`
found `glyphs.caret` and `glyphs.warn` read only by the ASCII-glyph test's own checklist, and the
streaming tail is drawn as plain prose spans (`app.rs`'s `draw_transcript`) — there is no caret glyph
on screen to move or to lose. `theme.warn()` is the *role*, which is a different thing and stays.

Deleting them alone would have left the hole that produced them, so `Glyphs::ALL` + `Glyphs::named()`
now exist and the ASCII test iterates them: a glyph added to the table is checked by construction, and
a name in the list with no field panics. The two dead fields were added by hand to a hand-written
checklist, which is the shape that let them in.

## F4 — seven duplicated doc lines and one stale wording in `ui/theme.rs`

All eight removed (`dim`, `faint`, `dim_strong`, `thinking`, `danger`, `accent`, `border`,
`border_focus`, `surface_pending`; and the surface block's stale "Deliberately close to a dark
terminal background" line, which sat above the *light* palette's bands and described neither).
`set_palette`'s doc said "rebuilds styles rather than recolouring the screen" when it swaps which
table roles resolve from and repaints nothing.

`no_doc_comment_line_is_written_twice` guards it. A prose fix has no negative control — the guard is
the artefact, and it is a reading of *this* file, which is the file the review named as the palette's
ADR-shaped record.

## F5 — author artefacts

**`zz_frame_dump_for_the_report` and `zz_frame_dump_no_color` are not in the tree, and never were.**
Not merely absent from `HEAD`: `git grep` across every commit reachable from every ref (`git log
--all -S`) finds them only inside the review document itself. They were the previous author's
*uncommitted* working-tree artefacts and were gone before the review was committed. Nothing to
remove; recorded here because the memory note they came from still asserted they were in
`app/golden.rs`, and a later round would have hunted for them.

**`fn filler` was kept and made honest.** It was real, dead (`#[allow(dead_code, reason = "kept beside
the fixtures it describes")]`) and duplicated: the one test that wanted twenty filler diff lines built
them inline with its own loop. The `allow` was what let the two drift apart without a warning. The test
now calls `filler(20)` and the `allow` is gone — the helper is used, so nothing can rot behind it.

## F6 — documentation hygiene

- **`docs/history.md` has not drifted again.** It is current through M130 (rows for M126, M127, M128,
  M129, M130, all `Landed <sha>`). The review's "ends at M121" was true when written; round 47 brought
  it forward and nothing has moved it since.
- **`CHANGELOG.md` gained two entries** under `[Unreleased]`: the TUI design-audit slices
  (M125–M130) and `/copy` over SSH (M129). **Stated limit: M83–M121 have no changelog entries at
  all** — the file stops at M82 and the review only asked about the recent slices. Backfilling
  thirty-nine milestones from their dev logs is a different job from answering this review, and doing
  it badly would be worse than the gap; the entry added names only what it can support.
- **`docs/README.md` now links M126, M127 and M130**, at the top of "Current hardening work", with a
  note that the list was last kept current at M115 — so a reader can tell drift from "nothing landed".

## F7 — `fg_of` maps `None` to `Color::Reset`, so `Some(Reset)` was partly ratatui's default

The NO_COLOR loop in `a_diff_still_reads_with_no_colour_and_ascii_glyphs` now cites
`matches!(fg, None | Some(Color::Reset))`, the form the same file already uses for the `NO_COLOR` role
assertion, and `fg_of`'s doc says why the helper collapses the two values and which callers therefore
must not use it. The loop's message says "emitted a hue" rather than "emitted colour", which is the
property it can actually assert.

**Its limit, stated rather than papered over:** the relaxation itself has no negative control. A test
cannot fail because it now accepts `None` where it accepted only `Reset` — that is what relaxing an
assertion means. What *is* controllable is that the assertion still refuses a hue, and that is control
3 below.

## Negative controls

Each was announced before arming, armed for one test run, restored from a backup taken **before** the
arm into `%TEMP%\epsilon-f1f7-controls\` (outside the repository), verified byte-identical by SHA-256
*and* by grepping the restored file for the control text, and touched so cargo could not serve the
control binary.

| # | Armed | Result |
|---|---|---|
| 1 | `footer.rs`: `whole_span` → `None` (badge protection disarmed) | **FAILED** `the_position_badge_is_never_half_written_at_any_width` with `26: "insert ^ …"`, `27: "insert ^  …"`, `28: "insert ^ 1 …"`, `29: "insert ^ 12 …"`, `30: "insert ^ 12/ …"`, `31: "insert ^ 12/4 …"`. `the_frozen_row_at_sixty_columns_keeps_the_position_and_the_exit` **passed** — recorded because it is a finding in itself: at 60 columns the badge was never at risk. |
| 2 | `app.rs`: the original duplicated `card_is_open` body, restored verbatim | **FAILED** `the_card_open_rule_is_borrowed_and_not_restated`. `the_drawn_and_recorded_answers_to_an_open_card_are_the_same_answer` **passed** — the duplicate is invisible to a behavioural test. |
| 3 | `theme.rs`: `ColorMode::None` + `Emphasis::Plain` → `Style::default().fg(Color::Yellow)` | **FAILED** `a_diff_still_reads_with_no_colour_and_ascii_glyphs`: `row 5 column 0 emitted a hue under NO_COLOR: Some(Yellow)`. |

**F4, F5 and F6 are prose and have no control**, and none is invented here: F4's guard is the test,
F5's `filler` change is covered by `ctrl_x_opens_the_newest_card_and_leaves_the_others_alone` failing
to compile if it were wrong, and F6 is two documents.

## What was deliberately left alone

- The **review's stated F1 failure mode** (the badge vanishing at 60 columns) does not happen, and the
  comment was corrected to the measured behaviour rather than to the review's wording.
- **The ratatui-tui skill files** (`.agents/skills/ratatui-tui/`, six of them) — dirty from another
  work item; read, not edited.
- **`side_by_side`'s callers other than the two in-tree ones** — there are none; the signature change
  is contained to `ui/header.rs` and `ui/footer.rs`.
- **M83–M121 changelog backfill**, **M116–M125 pointers in `docs/README.md`**, and the older rows of
  that list. Recorded as scope, not silently extended.
- **The review's structural suggestion** (a "what nothing exercises" section per milestone) is a change
  to the milestone template, not to this slice's findings; it is worth doing deliberately.
