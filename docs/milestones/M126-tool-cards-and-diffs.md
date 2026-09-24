# M126 — tool cards, a real diff, and four rounds spent on one assertion

The TUI audit's item D12 and the largest user-visible item left in it: a tool call was one flat line,
and an edit's diff was the literal `format!("{name} → diff:\n{diff}")` pushed through the **markdown
parser**. The single artefact a coding agent shows most often rendered as prose — no gutter, no
per-line colour, no line numbers, and nothing but the sign to tell added from removed.

## What

`crates/epsilon-tui/src/ui/card.rs` (new) owns the decisions, and states their reasons where a
reader will look:

- The body cap is a **row** budget (12, the skill's number), not a line count — a long line that
  wraps costs its wrapped rows, so a body cannot overflow the cap by wrapping under it.
- **Unified below a named width, split above it**: a unified diff that fits 60 columns beats a split
  one that wraps.
- Line numbers print only above a **column budget** (sign, number, padding) rather than being
  squeezed into a width that makes them unreadable.
- **Colour is never the only signal** — the gutter sign and the line numbers carry added-vs-removed
  under `NO_COLOR` and on 16-colour terminals, following the rule the theme already encodes.
- Expansion is **per card**, with the global toggle kept as "expand all" rather than removed from
  under a user who relies on it.

## Key decisions

- **Ctrl+X flips the global, and the selected card records "open if the global is now on, or it
  already was".** It used to record `!was_open`, so the second press — the one that turns the global
  back off — recorded the very card the first press had opened as explicitly *closed*. That is the
  opposite of the exception the per-card entries exist to provide. The fixture settled the reading
  rather than a guess: press 1 records the selected card open *alongside* the global, press 2 must
  leave it open while the others follow the global off.
- **The diff never reaches the markdown parser.** That was the defect, not an implementation detail:
  a diff is a structured thing with a gutter and two sides, and prose rendering is what made it
  unreadable.

## Gotchas

- **A test can be the thing that is wrong, and it can hide that for four rounds.** The last
  assertion expected `total_rows == all_open_rows - 4`, on the belief that the two short cards were
  "4 rows capped and 6 open". They hold a **three-line** diff against a **twelve-row** cap: nothing is
  hidden, open and capped render identically, and capping them gives back exactly zero rows. A card
  that cannot show the difference cannot be used to prove it.
- **The sequence of wrong theories is the lesson.** (1) "How many rows does a collapsing card give
  back?" — the wrong question. (2) "The cache fails to invalidate on a global flip" — refuted by
  reading `RenderCache::reconcile` and `CacheKey::of`, which do include `expand_all`. (3) Print the
  actual numbers — which found the defect immediately. **Print before theorising.**
- **A delegated author that keeps editing is not a collaborator, it is a moving target.** Staging a
  *directory* while it works swept its half-finished edits into commits; reverting two of its files
  then restored a state whose test target did not compile. Stage named files; never revert its work
  in progress.

## Verification

Battery `m126d`: **all five gates exit 0, 1,131 passed / 0 failed / 1 ignored across 61 binaries** —
fifteen more than M125's 1,116. `epsilon-tui` alone: 267 tests (was 252), including the frame-level
assertions that read **cells from `render_buffer` → `draw`**:
`an_edits_diff_is_drawn_with_a_coloured_gutter_not_as_prose` (the sign column's colour, in the
rendered frame), `the_diff_is_not_run_through_the_markdown_parser`,
`a_long_diff_is_capped_and_says_which_key_opens_it`,
`ctrl_x_opens_the_newest_card_and_leaves_the_others_alone`, and
`a_failed_tool_result_says_failed_on_its_title`.

`m126a`–`m126c` all failed and are not evidence: the first on fmt, the others on the author's
in-flight edits and on the wrong fixture arithmetic above.

## Limits

- **No author report and no independent review.** The delegated author was stopped after three
  rounds without convergence; the parent finished the slice and it has never had the read-only review
  the milestone workflow asks for.
- **`has_body()` short-circuits openness**, so a card with an empty body can never be opened — fine
  today, worth knowing before anyone adds a body that is empty until a later event.
- **The card cap is a constant, not a preference.** A user who wants longer cards has the per-card
  open gesture and nothing else.
- **The audit's other large items remain**: a light theme (`Color::White` is still body text), a
  frozen-view signal and jump-to-live, `NO_COLOR` glyph separation for the remaining roles, and
  OSC 52 for `/copy` over SSH.
