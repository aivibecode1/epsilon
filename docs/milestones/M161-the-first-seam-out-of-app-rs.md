# M161 — the first seam out of `app.rs`

## What this did

`crates/epsilon-tui/src/app.rs` was **4,526 lines** holding four separable concerns. This
milestone cuts the first and cleanest of them out: every *panel* — popup, overlay, or the
transcript itself — now lives in `crates/epsilon-tui/src/app/panels.rs` (803 lines), and
`app.rs` is down to 3,757.

Eleven functions moved: `draw_picker`, `draw_picker_stage2`, `draw_session_browser`,
`draw_transcript`, `draw_palette`, `draw_at_file`, `draw_timeline`,
`draw_slash_autocomplete`, `draw_dialog`, `draw_auth_prompt`, `draw_auth_picker`.

## Why this seam and not another

It is the one place where the boundaries could be read off the code rather than guessed at.
All eleven share a signature shape — a frame, the `App`, and a rectangle — and none of them
orchestrates: `draw()` composes them, and not one calls `draw()`. The other three bands in
the file (command dispatch, state mutation, viewport measurement) are separated by *kind*
rather than by shape, which is a guess wearing a plan's clothes.

## The rule this followed: move verbatim

The repo's own `mechanical-split` skill is explicit that a move must not become a refactor,
and the reason is not tidiness — **a refactor that also moves code cannot tell you which of
the two broke the build.** So nothing was renamed, reworded, reordered or cleaned up. The
complete set of edits the move required:

1. `fn` → `pub(super) fn` on the ten the parent still calls. `draw_picker_stage2` is called
   only from inside `panels.rs` by `draw_picker`, so it stayed private — the compiler said so
   via an `unused_imports` warning, which is the file telling you the truth about its own API.
2. An explicit `use panels::{…};` in the parent. Named, not glob-imported, so the set of
   things drawn over the transcript is readable in one place.
3. `mod panels;` next to the sibling `mod keys;` / `mod event;`.

**`composer_rows` deliberately stayed behind** in `app.rs`. It is a layout helper rather than
a panel, and leaving it there means `app/composer_viewport_tests.rs` — which reaches it
through `use super::*` — is **completely untouched**, which is the cheapest available proof
that the move changed nothing.

## How it was verified, and the checker that lied

A pure move's evidence is not "the tests still pass". It is that the moved text is the same
text. So the eleven bodies were extracted from `HEAD:app.rs` and from `panels.rs` by
brace-matching and compared.

**The first version of that checker reported all eleven as different, and the first
correction reported eight of eleven as different. Both were the checker, not the code.**

- The boundary search looked for `"\nfn "`, which does not match `"\npub(super) fn "` — so
  the slice in the new file ran to the end of the file, and the slice in the old file did
  not. Ten functions compared unequal because they had different amounts of text after them.
- The second attempt folded in the doc comment above each function by searching backwards for
  a blank line. For `draw_palette` that walked past the boundary and picked up the comment
  belonging to `mod composer_viewport_tests;` — a `mod` line that stayed in `app.rs` and so
  has no counterpart in `panels.rs`. Three functions compared unequal over a comment that was
  never supposed to move.

Compare **function bodies only**, and normalise the one keyword the move was allowed to
change. Then the result is exact: **eleven of eleven bodies identical.**

## The one real deviation, and why it is harmless

`draw_transcript`'s signature was one line in `HEAD`:

```rust
fn draw_transcript(f: &mut Frame<'_>, app: &mut App, area: Rect, geometry: TranscriptGeometry) {
```

Adding `pub(super) ` pushed it past rustfmt's 100-column limit, so rustfmt wrapped it across
five lines and — in doing so — added two parameter-trailing commas. That is the *entire*
content of the diff: two commas and a line break, from the formatter, not from me. It is
worth recording precisely because "I ran fmt" is exactly the sentence under which a real
edit hides.

## Verification

| gate | result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo test --workspace --features real` | **62 binaries, 1,288 passed, 0 failed** |
| `cargo clippy --workspace --all-targets --features real -- -D warnings` | exit 0 |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0, 6 messages, `tokens in/out: 300/150` |

**The test count did not move: 1,288 before, 1,288 after.** That is the result a
behaviour-preserving move should produce, and it is the only honest reading — no test was
added in this milestone. (This paragraph first claimed the count rose to 1,289, on a guess
that a `#[cfg(test)]` block had travelled with the eleventh function. The battery said
otherwise, and the battery is what is written here.)

## Limits — what this milestone does *not* prove

- **Three seams remain** in `app.rs` (3,757 lines): command dispatch, state mutation, and
  viewport measurement. They are separated by *kind*, not by shape, and will need the same
  read-the-code-first discipline before anyone cuts them.
- **No test was added**, deliberately. A move is verified by byte-identity plus an unchanged
  suite; inventing a test for it would be theatre.
- **No behaviour was exercised in a real console** for this milestone, because nothing about
  the frame changed. The bytes that reach the screen are the same bytes; the frame tests
  already covering the panels (364 in `epsilon-tui`) ran unchanged.
