# M157 — a search that moves the view

## What this replaces

`/search <query>` scanned the **session log on disk** and printed its hits as transcript
notices:

```text
• 3 hits for 'error':
• #41 user: …the error is in the parser…
• #88 assistant: …three errors reported…
```

A list of places you *could* go, printed into the one place you already were. The view did not
move, nothing was highlighted, the matches were prose rather than the rows on screen, and it
needed a session log attached — so with none it answered "search unavailable this run", which
is exactly when a user most wants it. pi searches the rendered screen
(`packages/tui/src/alt-screen-search.ts:22-30`), because that is what the reader is looking at.

It now searches the rendered transcript, opens a panel naming the query and the position in
the match list, and **moves the view to the match**. `n`/`p` step, wrapping; `esc` closes and
puts the view back.

## The trade, stated rather than hidden

The log reached further back than the viewport holds — the old command could find a message
from fifty turns ago. It could not show you the thing it found. This is recorded in
`search.rs` rather than left as a surprise.

## Two bugs, both caught by the frame test

1. **The jump could not resolve.** The line-to-row mapping lives in the render cache, and the
   cache is reconciled *during* the draw — so at key time it is empty and `row_of_line`
   returned `None`, meaning "do nothing", silently. The search reported its match perfectly
   and did nothing. The fix is a `pending_jump` the measurement applies.
2. **The fix then made it worse, in the most instructive way.** The first version also
   called `apply_pending_jump()` from the key path — and the cache there is the *previous
   frame's*, a perfectly good map for the wrong transcript. It consumed the target against
   stale offsets, cleared the flag, and left the measurement with nothing to do. The debug
   print read `scroll=67 pending=None`: a jump that had been applied to a transcript that had
   since grown by 81 lines.

## And a test that could not fail

`stepping_through_matches_moves_what_is_on_screen` asserted `opened.contains("findme")`. With
the jump disabled the view sits at the tail — where the **second** match happens to be — so it
**passed with the feature broken**. The control caught this: only 1 of 2 tests failed. Fixed
to assert the *first* match specifically, after which the control fails both.

That is the same rule as every other control in this repo, and it had to be applied to a test
written in direct imitation of those controls. Worth recording: writing the lesson down does
not mean you are exempt from it.

## What is not here

No incremental re-filter as you type — refining a query means leaving and retyping. No
match highlighting inside the matched row. No scrollbar. These are recorded in `search.rs` as
the obvious next slice rather than half-built, because a filter box that does not filter is
worse than none.

**What nothing exercises:** no console probe has driven a search in a real window. The frame
tests render at 80×24 with a pinned theme, so a search panel that overflows a 40-column
terminal is still unproven — the same width-dependence gap M156 recorded.

## Verification

- `cargo test -p epsilon-tui --lib` — **361 passed, 0 failed** (12 new).
- Control (jump disabled): **359 passed, 2 failed**, both naming the missing view movement.
- Full battery: see `docs/history.md`.
