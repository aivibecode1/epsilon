# M160 — `/theme`, and the cache that would have hidden half of it

## The audit claim that was wrong (again, and differently)

`audit/12` gap **G4** said Epsilon has "no theme selector at all", citing zero hits for
`theme_pick`, `ThemePicker` and `set_theme`. That grep was too narrow, and the truth is
considerably better than the claim:

- **Two palettes exist** — `ThemeMode::{Dark, Light}` (`crates/epsilon-tui/src/ui/theme.rs:110`).
- **They are contrast-tested**, in both directions: `the_light_palette_keeps_body_text_readable_on_a_white_terminal`,
  `every_light_role_clears_its_floor_against_every_surface_it_can_sit_on`,
  `both_palettes_keep_the_conversation_legible`, `body_text_is_never_a_hardcoded_white_or_black`.
- **They are selectable three ways already** — `EPSILON_THEME`, `[tui] theme` in config, and the
  CLI's own resolution order with an explicit "the flag was absent, not that dark was asked for"
  (`crates/epsilon-cli/src/main.rs:187`).
- `app.palette` (`ctrl+p`) turns out to be the **command palette**, not a theme picker.

The one real gap: the palette was decided **before the process started**. `ThemeMode::flipped()`
had carried the doc comment *"for the runtime toggle"* for a long time with **no production
caller** — `set_palette` had exactly one, in `app/event.rs:51`, applying the CLI's choice at
startup. A user who guessed wrong had to edit a file and restart to find out.

That is now `/theme`.

## The bug underneath it, which is the real content of this milestone

The transcript is drawn through `render_cache`, and a cache entry carries the escape codes it was
drawn with. `reconcile` took the theme as an input to **rendering**:

```rust
pub fn reconcile(&mut self, lines: &[TranscriptLine], width: usize,
                 state: &RenderState<'_>, theme: &Theme, restale: &[u64])
```

and never consulted it when deciding what to **rebuild**:

```rust
let invalidate_all = width != self.width || self.state.as_ref() != Some(&key);
```

So adding a runtime switch naively would restyle the chrome — drawn fresh every frame — and leave
every cached transcript row in the old palette. **Half a theme change is worse than none, because
the result stays legible enough to look deliberate.** The palette is now part of `CacheKey`, so
the cache decides through the mechanism the design already has.

## Two controls, and the first one is the story

The first version of the frame test called `render_cache.clear()` in the test body, and **its
control passed 363/363**. A test that cannot fail is not evidence — and the reason is instructive:
`clear()` was doing the invalidation, not the cache key, so the test was measuring the blunt
instrument. `clear()` is O(transcript) — precisely the work the cache exists to avoid.

Removing it exposed a second problem: the test read the **first non-blank cell** of the row, which
is the gutter glyph `•`, and the gutter is painted outside the cached row span. That is why the
first control *also* passed after `clear()` was removed. Reading a cell inside the body text fixed
it, and the control then failed with exactly the evidence that matters:

```
assertion `left != right` failed: the transcript row must be re-rendered in the new palette
  left: DarkGray
 right: DarkGray
```

A system line still wearing the dark palette's grey while the chrome had already moved to light.

`render_rows` strips styling, so the very first attempt — comparing row *strings* — could never
have failed however broken the cache got, because a system line is byte-identical in both
palettes. The doc comment on `render_buffer` already warned about exactly this. It is the second
time that lesson has had to be relearned rather than read.

## What `/theme` does

- `/theme` toggles; `/theme light|dark` sets (twice is idempotent — it does not bounce).
- The switch is **reported**, not silent: `palette: dark → light`.
- A bad name is refused with the two valid ones, and changes nothing.
- The choice is **not written to config**. It is a view preference, and a cosmetic toggle should
  not quietly rewrite a file the user may have under version control. `EPSILON_THEME` and
  `[tui] theme` remain the way to make a default permanent.

## Verification

- `cargo test -p epsilon-tui --lib` — **364 passed, 0 failed** (2 new).
- Control (`palette: ThemeMode::Dark` pinned in `CacheKey`) — fails, `left: DarkGray,
  right: DarkGray`. Restored byte-identically; the `.ctrl` copy is gone.
- Full battery: see `docs/history.md`.

## What nothing exercises

The palette switch has never been driven in a real console window, and the frame test pins
`ThemeColorMode::Ansi16` — so a *truecolor* terminal switching palettes mid-session is unmeasured,
as is any interaction with the `NO_COLOR` path. Both are the same width/depth gap M156, M157 and
M158 each recorded.
