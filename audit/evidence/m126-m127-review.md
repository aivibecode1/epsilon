# Independent read-only review — M126 (tool cards, diffs) and M127 (light theme, frozen view)

Commissioned after both slices landed, because their delegated authors were still working when the
parent committed and neither slice had the review `AGENTS.md`'s milestone workflow asks for. Read-only:
no file edited, no cargo run. Every claim cites `file:line`.

## The check this project keeps failing — and this time it passes

**Nothing in either slice is built-and-never-drawn.** Traced to the production draw path:
`ui::card::rows` ← `render_cache.rs:113` ← `logical_lines` ← `RenderedLine::build` ←
`RenderCache::reconcile` ← `App::measure_transcript` (`app.rs:3302`) ← `draw` (`app.rs:2897`);
`transcript::decorate` at `app.rs:3635`; the frozen badge at `footer.rs:287`; the light palette at
`theme.rs:502-507`; `--light` through `main.rs:1432` → `TuiConfig.theme_mode` → `event.rs:50-51`.

## Blockers

**B1 — The card duration is always `0ms`; the field the slice advertises carries no measurement.**
`state.rs:564-565` sets `started_at_ms` *and* `ended_at_ms` to the same `now` for the result card, so
`Card::elapsed_ms()` (`state.rs:165-168`) returns `Some(0)` for every result card ever drawn. The
pending card holds the real start (`state.rs:527-528`) and `settle_call` (`state.rs:581-593`) already
looks that card up — it just never returns the start to the caller. What the docs claim:
"a title line carrying the tool's name, its status and its duration" (`M126...md:28-29`), and
`card.rs:107-109` reasons explicitly that `412ms` beats the `0s` "a general formatter would print for
nearly every call — which reads as a field that does not work". The user sees `✓ ✓ edit · ok · 0ms`.
The test named for the property (`golden.rs:1150-1188`) asserts only that the tail ends in `ms` and the
rest is digits (`:1174`, `:1178-1181`) — so `0ms` passes it. **This is the project's "test named for
the right property, asserting the wrong quantity" failure mode again.** Fix: return the settled card's
`started_at_ms` from `settle_call` and use it, then require `ms > 0` in the golden assertion — or make
`elapsed_ms` return `None` for a result card and stop claiming duration in the doc.

**B2 — The ANSI-16 light palette reuses the dark palette's quiet roles, under a comment forbidding it.**
`theme.rs:394-439` overrides only `rgb` for `text_dim` (`:405`), `text_faint` (`:407`), `thinking`
(`:414`), `code_block` (`:428`) — every one `Color::DarkGray`, the same as dark (`:335,337,343,355`),
and `border` (`:398`) too. The comment directly above says these values are "**never `White`, never
`DarkGray`, which is the pair that disappears on a light theme**" (`:383-386`), and `:43-45` says the
same. `ColorMode::Ansi16` is the **default** on a console that does not advertise itself
(`ColorMode::detect()`, `:81-94`) — exactly the user `--light` exists for — so secondary text goes out
as bright-black on white. The contrast test the comment points at (`:892-929`) reads `.rgb` only;
`.ansi` is never inspected except for `text`/`assistant` and four hues (`:962-984`), and `Role::code`
and `Role::code_block` are missing from it.

**B3 — `EPSILON_THEME` with a bad value falls through to the config file, not to dark, in three places
that say otherwise.** `resolve_theme_mode` (`main.rs:1549-1559`) fails `from_name` (`:1554`) and then
falls through to the config key (`:1557-1558`). Contradicted by `main.rs:122` (`--help`), `:1537-1548`,
and `theme.rs:118-122`. With `EPSILON_THEME=solarized` and `[tui] theme = "light"` the session is
light — the opposite of all three statements. The existing test (`main.rs:2515-2522`) asserts a
`None`-or-anything range and would pass either way.

**B4 — `parse_diff`'s doc states the opposite of the code and of its own test.**
`state.rs:270-276` claims a one-for-one replacement is numbered `5`/`6`; the code (`:303-319`) gives the
addition `new_no` *before* incrementing, and `state.rs:1446-1451` plus the golden frame
(`golden.rs:1236-1243`, with the reasoning at `:1227-1234`) assert they **agree**. A future reader
"fixing" the parser to match the comment would break a pinned test.

**B5 — The split diff is production code no test has ever drawn.** `card::rows` → `diff_rows` →
`split_rows` at `width >= SPLIT_MIN_WIDTH` = 120 (`card.rs:73,218-224,310-349`). Every golden diff test
renders at 80 columns and the cache test at 60. The geometry checks out by hand
(`left + right == body_width`, `pad_to` per half), which is why this is a **coverage** blocker and not
a width bug — but "I read it and it looks right" is what B1 also looked like.

## Findings

- **F1 — `footer.rs:179-189` claims 60-column survival the arithmetic does not give.** The trim is
  `Priority::Right` (`:309`), so hints yield from their *near end*; with the badge this test uses and a
  realistic 28-column location, a 60-column row begins `/40 · g jump to live …` — the badge's marker,
  the thing that says "scrolled back", is what disappears. The test that promises the rule
  (`:473-488`) exercises the *idle* row at 80 columns, and the frozen narrow test (`:515-521`) asserts
  only `width <= 60`.
- **F2 — the card-open precedence is implemented twice**: `RenderState::is_open`
  (`render_cache.rs:66-73`) decides what is *drawn*; `App::card_is_open` (`app.rs:2980-2989`) decides
  what a keypress *records* (`keys.rs:1158`). Identical today, untested for agreement — and the whole
  per-card convention lives in that agreement.
- **F3 — `Glyphs::warn` (`theme.rs:190`) and `Glyphs::caret` (`:173`) have no reader** in `crates/`.
- **F4 — seven duplicated doc lines in `theme.rs`** (`:576-577`, `:582-583`, `:588-589`, `:605-606`,
  `:617-618`, `:645-646`, `:651-652`) plus a stale `:628`.
- **F5 — author artefacts**: `zz_frame_dump_for_the_report`, `zz_frame_dump_no_color`, and
  `fn filler` with `#[allow(dead_code)]` (`golden.rs:1692-1696`).
- **F6 — documentation hygiene**: no M122–M127 rows in `docs/history.md` (ends at M121), no
  `CHANGELOG.md` entries, neither milestone linked from `docs/README.md`, and no review record until
  this file.
- **F7 — `fg_of` maps `fg: None` to `Color::Reset`** (`golden.rs:734`) while a NO_COLOR test asserts
  `Some(Color::Reset)` literally (`:1461-1469`), making that assertion partly about ratatui's default.

## What is genuinely good

The features really are drawn (above). The light palette's ratios were **independently recomputed** and
hold where the tests assert them (light `text` 15.5:1 on white — the comment's 14.8 is close; dark
`text` 1.39:1, matching the 1.19:1 the test documents as the defect). The light-palette frame test pins
**exact RGB values, not adjectives** (`golden.rs:740-780`), with a comment saying why an assertion on
"dark-ish" would also pass for timestamps — the standard this project keeps failing to reach, reached.
The frozen-view work is well-built: `View::badge` is one value with one reason (`footer.rs:35-81`), the
jump-to-live hint resolves through the live table (`:196`, `keybindings.rs:296-298`) with a rebinding
test (`:618-623`), the key arm is guarded on the frozen state so the bare letter stays typeable when
live (`keys.rs:1200`), and `apply_with_scroll` keeps a frozen window on the same content
(`app.rs:2699-2717`). Precedence lives in exactly one place. And the negative control that found the
viewport bug is the right response to a latent defect.

## Could not verify (stated, not guessed)

Suite totals were re-summed from the gate logs and match the docs' passed/failed/ignored figures, but
the "61 binaries" line in the docs does not match the 58 `Running …` lines in those logs, and no test
was run. Whether the committed bytes are exactly what the batteries built is **not** established —
`gate-m128a.log` predates `546a9cb`, whose message says the dirty `main.rs` carried the clippy fix the
battery compiled, so the repo's own "evidence must cover the shipped bytes" rule is in question for
that battery. The four M127 negative controls have no probe logs in `audit/evidence/`. The demo and
`--light --provider faux` runs quoted in the milestone are not in any log. And the cell-level evidence
all comes from `TestBackend`, which is trustworthy for style and less so for what a real console does
with `Color::Reset` + `DIM`.

## The structural suggestion worth keeping

Both milestone docs list `Limits`, but neither lists **what no test exercises** (the split diff) or
**which fields nothing asserts** (the duration, `.ansi`). A one-line "what nothing exercises" section
per milestone would have surfaced B1 and B5 without a reviewer.
