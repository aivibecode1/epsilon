# Working note — the M262 review sweep, and what it hands the next round

**Status: four parallel read-only reviews ran at M262 and their findings are in the ledger, not
here.** The queue of record is the new section at the end of
[`audit/03-defects.md`](../../audit/03-defects.md) — rows **A27**–**A44**. This note exists for
the two things a ledger cannot hold: why the sweep was run, and what to do first.

## What the sweep was, and what it cost

Four subagents, one per slice — desktop IDE, TUI interaction, security/data-integrity,
reliability/performance — each told explicitly: **read-only, do not run cargo**, because this
machine has one build owner and parallel builds fail with `0xc0000142`. They were also told to
mark every claim *read* or *inferred*, and to put anything they could not determine in a
"what I could not determine" section rather than rounding up.

That instruction is the reason the results are usable. Four of the sharpest findings were
**predicted failures, flagged as such by the reviewer**, and two of them turned out to be real
once measured:

| Finding | Reviewer said | Measured |
|---|---|---|
| Goal band at x=0 | "predicted failure, needs a probe" | `[0,97]-[520,131]` vs pane `[920,42]-[1440,870]` |
| Every key in `/search` quits | "inferred from the return convention" | `Esc ended the session from inside the search view` |

**The cost:** about 40 minutes of wall clock that overlapped the battery, and the sweep paid for
itself twice over (A27 was the most severe defect found in the round).

## Do these first, in this order

1. **`/compact` can only ever run once per session.** `compact_running` is set at
   `app/commands.rs:1605` and **never cleared** — five mentions in the crate, one writer, one
   reader, no reset. The comment above it describes the bug as intended behaviour. Every later
   `/compact` refuses with *"compact already running — wait for it to finish"*, for ever.
   Compaction is a core capability and the message is plausible, which is what makes it nasty.
   Fix: clear it when the task completes, and stop discarding `compact_now`'s error into `let _`.
2. **The desktop window never shows what a tool printed.** `ToolExecutionUpdate` falls into
   `_ => {}` (`desktop/src/session.rs:497`), so a reader sees `▸ bash {…}` then `✓` and never
   the output — **while the resume path does show it** (`:257-268`), so the same session shows
   two different transcripts before and after a session switch. `RowKind::Tool { name, failed }`
   has no field for output; the terminal's pattern is the model: `partial` is a **delta**, keep a
   bounded tail (M152).
3. **Security rows A31–A37** if the round is going to security: the symlink/junction escape
   (A31) is the highest and is **already specced by the project** at
   `audit/evidence/f09-path-policy-map.md:26,29` — recorded in 2026-09 and never implemented.

## Two things the reviews found that are about the *method*, not the code

- **A defect in the wrong unit is invisible to every fixture written in the other one.** Six
  display sites counted `char`s where the screen counts columns; every fixture in the suite is
  ASCII, where one column is one character. 261 milestones of card tests could not have seen it.
- **A green sweep over a fixture that cannot produce the thing you are worried about is not a
  green sweep.** The desktop containment test iterates every `row_rects` entry and would have
  caught the goal band on sight — its fixture seeded no goal.

## Traps this round paid for

- **`gate-until-pass.ps1` takes the BASE label on attempt 1**, not `<label>-r1`. Retrying with
  the same `-Label` I had already used for a manual `gate.ps1` run **truncated the failing
  log**, which is exactly the loss `AGENTS.md` warns about twice. Give the retry wrapper a
  *fresh* base label.
- **`Avira LNK1104` on a fresh test binary.** The documented remedy is fresh test-profile
  metadata (`CARGO_PROFILE_TEST_DEBUG=1`), which renames every test executable and therefore
  forces a **full test-profile rebuild** — the test leg went from ~675 s to **2,402 s**. Worth
  it once; worth knowing before you start one.
- **A `#[test]` left above a `#[tokio::test]`** compiles as neither: *"async functions cannot be
  used for tests"*. When changing a test's async-ness, the attribute above it is part of the edit.

## The sweep's own limits, stated

Nothing below tessellation was observed (M252: no wgpu backend presents on this box), no
finding was reproduced, and no cost in the reliability report was measured — every magnitude in
it is read from the code plus the project's own recorded numbers. **The desktop frame path has
no measured number anywhere in the repository**, which is why sizing any fix for it needs a
measurement first.