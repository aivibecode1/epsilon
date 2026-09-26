# M153 — every tool card says something specific

## What

The gap analysis's #1, and the one that touches **every card a user will ever see**.

Every tool spoke the same dialect. A result was rendered as:

```rust
format!("{name} → {}", truncate(body, MAX_RESULT_CHARS))
```

and that whole string was the card's **header line**. So a `read` card's one-line summary was
400 characters of the file the user was already looking at — and the two facts they actually
wanted, *which file* and *which lines*, appeared nowhere, despite `read` reporting
`first_line`, `last_line` and `total_lines` in `details` and the path sitting in the arguments.
The call side was a flat "first of eight known keys" heuristic, which meant a `grep` card
announced only its pattern (never where it searched) and any tool using other argument names
fell through to a raw JSON dump.

Measured before and after, same card, same result:

```
✗ ✗ bash → boom                          (before: the header is the body's first 400 chars)
✓ ✓ read src/main.rs:10–50 of 300 · ok    (after:  the file, and the lines)
▸ ▸ bash $ cargo test -p epsilon-tui      (a running call reads as a command)
```

## The shape of the fix

`crates/epsilon-tui/src/renderers.rs` — `call_head` and `result_head`, a `match` per tool.
`Card` gains `args` and `head`; `ui/card.rs::title` renders the head. A result carries no
arguments, so `push_tool_result` recovers them from the *call's own* card by `tool_call_id`.

**The renderer invents nothing.** A line range comes from `details.read`, a truncation flag
from `details.truncated`, a path from the arguments the model itself sent. A missing field
degrades to something vaguer, never to a guess.

## Two bugs I introduced and caught in this slice

Worth recording because both were invisible until something was actually painted:

1. **`bash bash → boom`.** The renderer's fallback leads with the tool's name, because that
   string is *also* the collapsed transcript line, where the name is the only thing saying
   which tool ran. `title()` prints the name too. First fix was to suppress the prefix when it
   was redundant — which made it worse, `bash · failed`, because that dropped the whole
   headline. The correct fix strips the redundant prefix and keeps the phrase.
2. **A lost tool name.** The same fallback had briefly dropped the name entirely, which broke
   two pre-existing tests (`malformed_todo_details_fall_back_to_generic_rendering`,
   `edit_result_with_a_diff_commits_a_parsed_diff_card`). They were right and I was wrong: a
   collapsed line of output with nothing saying which tool produced it is not a fallback.

Neither would have been caught by a state-level assertion. Both were caught by the painted
tests in `renderers/tests.rs` and by the golden frames.

## Negative control

Reverted only `title()` to the pre-M153 shape (`let name = card.name.clone()`):

```
test result: FAILED. 340 passed; 8 failed
"✗ ✗ bash · failed · 0ms"      ← the pre-M153 frame, captured from the failing run
```

## Deliberate divergences from pi

- **A collapsed `read` keeps its body.** pi hides it entirely (`renderers/read.ts:120-122`
  returns `""`) because it pairs that with click-to-expand, which we do not have (analysis
  #12). Hiding the content with no way to bring it back trades information for tidiness.
  Revisit if the card gains an expand binding.
- **No live elapsed row.** pi renders `1 Hz Elapsed` for `bash`. Blocked on the same
  render-cache problem M152 recorded, and a renderer cannot honestly fix a number computed
  elsewhere.
- **No per-tool body truncation warnings yet.** pi's `grep`/`ls` render a
  `[Truncated: …]` line. The *headline* says so now; the body line does not.

**What nothing exercises:** the painted tests assert `card::title`, not a full frame. The
golden frames that do render a whole screen were re-pinned rather than added to — so a
renderer change that is correct in isolation but wrong in composition would pass. The
`--size` probe still cannot drive a live tool call, so no screen in a real console has shown
any of this.

## Re-pins

Ten assertions across `app/golden.rs` and `render_cache.rs` moved from the old shape to the
new. Each was checked to be a re-pin and not a regression: the frames that failed show the new
title rendering correctly. One assertion needed its *meaning* repaired rather than its string:
`!joined.contains("→")` became `!joined.contains("edit → ")`, because the new headline may
legitimately contain the output's first line, so the assertion had to be about the form
rather than the words.

## Verification

- `cargo test -p epsilon-tui --lib` — **348 passed, 0 failed** (14 new).
- `cargo clippy -p epsilon-tui --all-targets --features real -- -D warnings` — exit 0.
- Full battery: see `docs/history.md`.
