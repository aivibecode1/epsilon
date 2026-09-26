# M149 — a result the harness knows is short stops saying "ok"

## What

M148 made a shell call that lost the race against a surviving background process report
`output_complete: false`. Nothing read that flag. The tool card still rendered a green tick and
the word **ok** for any result that was not an error, so the fix was invisible to the one person
it was written for.

This slice makes the transcript tell the truth: a result that is known to be short is **partial**,
which is neither `ok` nor `failed`.

## The distinction this rests on

"Did not fail" and "told the whole truth" are different questions, and a card with one status field
can only answer one of them. A command that exits 0 while a background process holds its output
pipe has genuinely succeeded — so `Failed` misreports it — and the harness knows the text below is
short — so `Ok` over-reports it. Before this milestone the TUI always answered "did not fail", and
answered it confidently.

A green tick is the worst of the three options available, and not only because it is green: a tick
is the specific glyph a reader has learned to mean *this finished and this is all of it*. Spending
it on a result known to be incomplete spends the reader's trust on the exact case where the answer
does not matter. A wrong exit code gets investigated. A green tick on short output gets believed.

## What changed

- `CardStatus::Partial` (`state.rs`) — a third status, not a footnote on `Ok`.
- `status_word` (`ui/card.rs`) — the warning glyph and the word `partial`, in **both** themes.
- `Glyphs::warn` (`ui/theme.rs`) — `!` in the unicode and ascii tables. Not the tick, not the
  cross: the tick says the result is whole, the cross says the command failed, and neither is true.
- `push_tool_result` reads `details.output_complete`; `is_error` still wins, because when a command
  both failed *and* was cut short, "it failed" is the more useful of the two facts and they are
  not exclusive.

The word is not decoration. `status_word` is documented to return both the glyph and the word
because the glyph is what a glance catches and the word is what survives a font that cannot draw
it. `partial` is the only one of the three words that tells the truth here.

## Negative control

Reverted **only** the status selection, leaving the flag parsed and ignored, so the control
isolates "did the transcript read it" from "does the flag exist":

```
test state::tests::a_result_known_to_be_short_is_partial_rather_than_ok ... FAILED
assertion `left == right` failed: exit 0 with output_complete:false is neither ok nor failed
  left: Ok
test result: FAILED. 328 passed; 1 failed
```

`left: Ok` is the defect, quoted back. **1 failure out of 5 new tests** is also the point: the other
four are controls of each other — no flag still means `Ok`, an error still beats partial, and `ok`
and `failed` keep their own glyphs — so a change that broke them would have shown up here rather
than passing quietly. Restored from backup; the diff is 114 insertions and 0 deletions.

## Verification

- `cargo test -p epsilon-tui --lib` — **329 passed, 0 failed** (5 new).
- `cargo clippy -p epsilon-tui --all-targets --features real -- -D warnings` — exit 0.
- `cargo fmt --all -- --check` — exit 0.
- `cargo test --workspace --features real` — **62 binaries, 1,249 passed, 0 failed**, exit 0 (from 1,244).
  `cargo run -q -p epsilon-cli -- --demo` — exit 0, 6 messages, `tokens in/out: 300/150`.

**What nothing exercises:** the state tests build the `details` map by hand, so nothing proves the
*real* `BashTool` still writes `output_complete` into it — the two sides are joined only by a
string key. An end-to-end check that runs a real shell call and renders the card would close that,
and it needs a descendant that outlives the command, which is exactly the M148 scenario. M147's
bwrap half still has no executed coverage on this host (`cfg(unix)`; reported as 0 tests, and CI
covers it on `ubuntu-latest`). And the model still sees the short output with nothing marking it
truncated: this milestone changed what the *user* is told, not what the model is told, so a
truncated result can still mislead a turn even though the card now warns a human.
