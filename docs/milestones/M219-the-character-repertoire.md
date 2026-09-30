# M219 — the character repertoire, and the default that got it wrong

**Reported by the owner, on the installed binary: "when you open cmd or pwsh I can see some boxes
with `?`."** Found by a report, not by a test — and the measurement that confirmed it is one this
repository's own tooling could not do.

## What was wrong

**The measurement, and its blind spot.** `scripts/console-probe.py` reads the console **buffer**, so
the frames this repository's evidence has been made of show the *characters the harness wrote*. They
say nothing about what the console *renders*. A frame full of `╭ ╮ ╰ ╯ ▎ ✓ ✗ ❯ ▸ ≡` is perfect
looking in a probe and a row of boxes on a classic conhost that writes CP437, which carries none of
them. That is a gap in the method every "read a frame" rule in this project leans on, and it is
stated here rather than left as a footnote: **a buffer readback cannot see a missing font.**

**Three defects, in order of how badly they hurt:**

1. **`Glyphs::detect()` defaulted to Unicode everywhere.** The old comment says it plainly: *"We do
   not sniff the code page: the variable exists because the answer cannot be detected reliably, and a
   wrong guess is worse than a documented switch."* True as far as it goes, and wrong about which
   side the guess falls on — the **default was the guess**, and it guessed wrong for `cmd.exe` and a
   PowerShell window in a legacy console, which is the common Windows case. Now: Windows gets ASCII
   unless a UTF-8-capable *host* is positively identified (`WT_SESSION`, `TERM_PROGRAM=vscode`, or a
   VT-capable `TERM`), everything else gets Unicode, and `EPSILON_ASCII=1`/`0` overrides either way —
   `0` being how a legacy console with a Unicode font opts back in.
2. **The model picker drew `🔑`** for "this model has credentials". An **astral-plane** codepoint,
   four UTF-8 bytes wide: not a glyph some code page lacks, a character no legacy console can render
   at all. It is now `Glyphs::key` — `🔑` or `@`.
3. **The switch was documented nowhere a user would look.** `EPSILON_ASCII` existed only in a code
   comment and a milestone log. Someone seeing boxes had nothing to set and no way to find out what
   to set. It is in `--help` and in `docs/user/troubleshooting.md` now, in both directions.

## The fix, in the parts that are separable

- **`Glyphs::detect_from(override, wt_session, term_program, term, on_windows)`** is the rule with
  every input handed in; `detect()` reads the environment and calls it. The environment is
  process-global and the suite is threaded, so a test that *set* `EPSILON_ASCII` to assert a default
  would be asserting against whatever another test left behind. Splitting the decision out is what
  makes it testable at all.
- **`Glyphs::key` and `Glyphs::cycle`** (`🔑`/`@`, `⟳`/`~`). `cycle` needed `UiState` to hold the
  repertoire it writes markers with — a state holder that stores transcript lines has to commit to
  characters, and a marker it has written is one it cannot take back. `UiState::glyphs` defaults to
  **ASCII** and *no field reads the environment*, because `detect()` is a process decision and
  `UiState::default()` is a per-instance one; that is also why `Glyphs` now has
  `Default = ASCII` — "the repertoire nobody chose is the one that renders everywhere."
- **The exhaustiveness check was itself incomplete.** `Glyphs::ALL` is what the seven-bit test walks,
  and its own comment claimed a field could not be added to the table without a name there. It could:
  **`warn` was missing**, and so were the two added here — the one check that exists to keep the table
  honest skipped the very glyph that could not be rendered. `SINGLE_GLYPH_FIELDS` is a second,
  independent list, and the new test compares the two as **sets**; the old assertion compared counts,
  which passes when both lists are wrong in the same way.

## Verification

**Battery `m219`, GATE PASS** — seven gates, `audit/evidence/gate-m219.log`: **1,641 tests across 65
binaries**, against M218's 1,639 across the same 65: **+2**, the two new tests. Gate shape:
check-links 0 s, check-lint-tables 0 s, fmt 2 s, clippy 20 s, test 240 s, demo 7 s, real-check 5 s.
Run with `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0` (M208's disk knob).

**On a real console, in the mode that was broken.** With `EPSILON_ASCII=1`, driven through
`scripts/console-probe.py` at 100x30 with `/sessions` typed:

```text
+ commands --------------------------------------------+
| > /sessions  browse session files in this workspace… |
| ↑↓ pick · tab apply · esc dismiss                    |
```

The rounded corners, the block bar and `✓`/`✗` are gone; the selection bar is `>`. (The `…`, `↑↓` and
`·` remain — see the residual below, which is a list and not a shrug.)

**Two new tests, two controls.**

| Test | Control | Failed with |
|---|---|---|
| `the_repertoire_default_is_ascii_on_a_legacy_windows_console` | the pre-M219 rule (`if false && on_windows && …`) | `a console with no UTF-8 host must not be handed box-drawing characters by default`, printing the whole Unicode table the user would have got |
| `the_glyph_name_list_and_the_table_agree` | dropping `warn` from `SINGLE_GLYPH_FIELDS` | the two sets, differing by exactly `warn` |

Both controls had to be *applied* before they were believed: the first control run reported the
list-drift test as **passing**, because the replacement string it used had CRLF line endings and the
repository is LF since M216. A control that did nothing and a fix that did nothing look identical
from the output — which is the argument for checking that the edit landed, every time.

## Residual, named precisely

The characters still written as literals rather than taken from the table, and **whether CP437 has
them** — because "renders as a box" is not a property of a character but of a character *and a code
page*:

| Character | Where | In CP437? | Consequence on a legacy console |
|---|---|---|---|
| `●` | model picker's active row | **no** | a box |
| `★` | model picker's favourite row | **no** | a box |
| `…` | every truncation | yes (0x85) | renders |
| `↑` `↓` | hint rows, header gauge | yes (0x18/0x19) | renders |
| `·` | every separator | yes (0xFA) | renders |

So two rows of one panel can still show boxes, and they are recorded as **A22** rather than fixed
here: the sweep is ~150 literals across `footer.rs`, `header.rs`, `panels.rs`, `view.rs`, `metrics.rs`
and the notice/help strings, and doing it as a side effect of a report would be exactly the
unreviewed mechanical change this repository distrusts. The rule for it is written down in the plan
that will carry it: **a character the UI draws belongs in the table**, and the ASCII test then
protects it.

## Limits

- **The Windows half of the rule is a heuristic, not a measurement.** `WT_SESSION`, `TERM_PROGRAM`
  and `TERM` identify the *host*; reading the console's actual output code page needs `unsafe`
  (forbidden workspace-wide) or a new dependency. A console that is UTF-8 and advertises none of
  them now gets ASCII — visually plainer, never broken — and `EPSILON_ASCII=0` is the one command
  that fixes it.
- **Nothing in the suite can see this class of failure**, and that is now stated rather than implied:
  a `TestBackend` holds the characters, and a probe reads the buffer. Both are blind to the font. A
  check that could see it would have to ask the console what it can draw, which is the same question
  the code page answers and the same dependency the heuristic avoids.
- **`CliRunner`-style installed-binary checks did not exist when this was reported**; the frame above
  was driven by hand. The console-probe artifact for this milestone records the ASCII frame.

## Nothing exercises

`Glyphs::detect_from` is tested directly, and the two tables are tested seven-bit-clean. **What is not
tested is the *drawing* path's use of the repertoire end to end** — that the model picker's row shows
`@` under `EPSILON_ASCII=1` is asserted on a value, not on a painted frame. M219 changed the marker in
a panel and the glyph in a state holder; a frame test for each would catch a third path that reads the
wrong one of the two, and neither exists yet.
