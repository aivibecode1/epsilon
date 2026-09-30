# M220 — every drawn character a CP437 console can actually draw

**Closes A22, and starts the check that should have existed before M219.** The owner's report (boxes
in `cmd.exe`) was fixed twice; this is the part that makes the third occurrence impossible to miss.

## The idea, and why it is a test rather than a sweep

The obvious work is a sweep: find the literals, move them into `Glyphs`, done. The problem is that
the *next* literal is one character away, and nothing about the code makes it visible — the glyph
table's own test only knows the table's own fields. So the slice starts by writing the check and
letting **it** say what is wrong:

```rust
// crates/epsilon-tui/src/app/golden.rs
fn an_ascii_frame_draws_no_character_a_legacy_console_cannot()
```

It paints twelve frames — the plain shell and the three overlays, at 80x24, 100x30 and 120x40 —
with the **ASCII** repertoire, reads the drawn rows back, and fails on any character outside CP437.
Its first run named **94 offending rows**. It is a check over *drawn output*, which is why it sees
things neither `ascii_glyphs_are_seven_bit_only` nor a reader of `panels.rs` could: the table was
green the whole time.

## The promise, stated precisely (and corrected)

"Every drawn character is one a CP437 console can render" — **not** "everything is seven-bit".
Harness prose is allowed to be readable: `·` is 0xFA and `—`… no. Which is the correction:

> **An em dash is not in CP437.** 0x97 there is `ù`. `—` is 0x97 in **Windows-1252**, a different code
> page that every modern terminal happens to speak.

M219's milestone wrote "`—` (0x97) is in CP437, so prose may keep them". That was wrong, in exactly
the way this repository keeps warning about: **the constant is not the predicate.** The frame check
found it on its first run and the correction is in `Glyphs::dash`'s doc comment. The tables in the
test are transcribed CP437 (`CP437_MARKS` for 0x01–0x1F, where `↑` and `↓` live, and `CP437_HIGH`
for 0x80–0xFF), so the promise is checkable rather than asserted.

## What moved, and the arithmetic that came with it

| Glyph | Unicode | ASCII | Why it was a literal |
|---|---|---|---|
| `dot` | `●` | `*` | the picker's "you are here"; not in CP437 |
| `star` | `★` | `+` | favourites; not in CP437 |
| `ellipsis` | `…` | `...` | 105 sites, but two helpers produce almost all of them |
| `up`/`down` | `↑`/`↓` | `^`/`v` | CP437 *has* these (0x18/0x19) — a preference, not a repair |
| `dash` | `—` | `--` | panel titles; not in CP437 |

`…` and `format_path` were the bulk, and both took the mark as a **parameter** rather than reading a
global: `format_path(path, home, width, ellipsis)`, `truncate(s, max, ellipsis)`,
`truncate_public(...)`, and through the renderer chain (`call_head`, `result_head`, `first_line`,
`with_command`, `collapse`) so a card headline's mark comes from the theme that draws the card.

### The defect this slice introduced, and caught

The ASCII mark is **three cells** where `…` is one, and two pieces of layout arithmetic assumed one:

- `format_path` budgeted `+ 2` for the mark and the separator, so the ASCII path label came back
  **two columns wider** than the row allowed — and the footer's answer was to drop a hint. The
  symptom was a *test elsewhere* failing with "the footer does not name the way back: `insert ^ 2/10
  · g · pageup`". The bug was not the footer; it was a string that was two columns too wide.
- `list_row` reserved one column for the mark, so its clip budget went **negative**, the guard fired,
  and the hint was *dropped* instead of shortened: at 54 columns the slash popup lost every
  description. The M218 test caught this immediately.

Both are the same mistake twice: **a repertoire change is also a layout change**, and the budget has
to count the mark's real width. Both sites now subtract `display_width(mark)`, and both say so where
the arithmetic lives.

## Verification

**Battery `m220-r2` GATE PASS** — seven gates, `audit/evidence/gate-m220-r2.log`: **1,642 tests across
65 binaries**, against M219's 1,641 across the same 65: **+1**, the frame check. Gate shape:
check-links 0 s, check-lint-tables 0 s, fmt 2 s, clippy 13 s, test 213 s, demo 7 s, real-check 3 s.
Run with `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0` (M208's disk knob).

The first attempt (`m220`, kept as `audit/evidence/gate-m220-FAILED-clippy.log`) failed on **clippy,
not a test**: an empty line left between a doc comment and its function by one of the edits above, and
a `String::into()` in the `view.rs` arrow binding. Both were in the code this slice wrote, which is
the check doing its job — and the second one was a real smell rather than a lint: it had been
calling `Glyphs::detect()` **twice** in one expression, and it now asks once.

**On a real console, ASCII mode**, `/models` then Enter, read back from the buffer:

```text
+ models -- filter: '' -----------------------------------------+
| >   ·  openai                                        38 models |
|     ·  github-copilot                                28 models |
|     @ deepseek                                         4 models |
```

`--` for the dash, `@` for the credential marker (it was `🔑`, four UTF-8 bytes that no CP437 console
can draw), `*` for the active row, and `·` which CP437 does have at 0xFA.

**The control.** `dot: "*"` put back to the literal `●` fails the frame check with the offending row
spelled out, three sizes:

```text
the ASCII repertoire drew 3 row(s) a CP437 console cannot render, across 12 frames:
80x24 panel 1: ['●'] in "        |   ●@ faux                                           1 models |"
```

**Two tests stopped asserting literals.** `oversized_tool_output_is_truncated` asserted
`ends_with('…')` against a `UiState::default()` that is the ASCII repertoire since M219; it now reads
`st.glyphs.ellipsis`. So does the M218 row test. Both were asserting *the repertoire* rather than the
behaviour, which is the mistake the sweep exposed in the tests as well as in the drawing code.

## Residual, named

**Prose is not swept.** The harness's own sentences — help text, notices, tool descriptions — still
contain `—` (U+2014) in roughly two hundred strings, and those are drawn into the transcript. So a
legacy console still shows a box *inside a notice*, while every structural character is now
CP437-safe. This is recorded as **A23** with the count, and the rule for whoever takes it: prose
keeps its punctuation in the Unicode repertoire and the *repertoire decides*, which means notices
have to be built with the glyph in hand rather than typed — the same shape as the renderer chain
above, one layer out.

## Limits

- **`CP437_MARKS`/`CP437_HIGH` are transcribed by hand.** A wrong character makes the test either too
  strict (a false failure, cheap) or too lax (a box slips through). Both were typed from the DOS OEM
  table and are cross-checked by the test's own behaviour: the low range is what makes `↑`/`↓` pass,
  and the absence of `…` is what fails without it.
- **The frames hold no user or model prose**, because that text is not the harness's to change — a
  message in Greek is a message in Greek. So this says nothing about content rows.
- **A card headline is still asserted on a value, not on a painted row**; the mark now reaches it,
  but no frame test paints a card in ASCII mode. That is A23's other half.
