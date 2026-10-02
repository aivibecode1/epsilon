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

1. ~~**`/compact` can only ever run once per session.**~~ **CLOSED at M264.** The flag is
   now an `Arc<AtomicBool>` the detached task clears, and `compact_now`'s error is named
   instead of discarded. The detail worth carrying: the comment above the spawn *described the
   defect as intended behaviour* ("the spinner stays until the next successful `/compact`"),
   which is how a permanent refusal reads as a design decision.
2. ~~**The desktop window never shows what a tool printed.**~~ **CLOSED at M264.** Rows carry
   the agent's `tool_call_id`, `ToolExecutionUpdate` is handled, output is a 40-line tail for
   the same reason the terminal keeps 40, `live_tool` is deleted, and the replay path writes
   the same field the live one does — so a resumed session and a live session finally read the
   same.
3. **The desktop has no slash commands at all** (`A38`). Still the largest parity gap: typing
   `/compact` in the window sends that literal string **to the model**. Worth knowing before
   sizing it — **`epsilon-desktop` already depends on `epsilon-tui`** (for the theme palette,
   ADR-020), so `epsilon_tui::picker::parse_command` is reachable with no new crate edge. The
   real question is where command *behaviour* lives: architecture law says the desktop owns
   pixels and changes `epsilon-runtime` when it needs behaviour the composition lacks. So a
   command whose effect is agent-level (`/goal …`, `/compact`) can be shared, and one whose
   effect is TUI-local (`/theme`, `/timeline`) cannot.
4. ~~**Security row A31 — the walk followed a link out of the workspace.**~~ **CLOSED at
   M265.** `paths::is_link`, used by both walkers; the Windows branch tests the reparse-point
   attribute because a junction is the form that needs no privilege *and* the form
   `is_symlink()` does not report. Still open in that group: **A33** (`.env.` / `.env::$DATA`
   bypassing the ask) and **A34** (extension and MCP children inheriting the credential
   environment).
5. ~~**Desktop per-frame cost — measure before sizing it.**~~ **MEASURED and FIXED at M266.**
   Two separate errors, both worth remembering:
   - The review's ranking was **inverted**: the sessions pane it ranked first costs **0.9 ms**
     for fifty files; the transcript it ranked fifth costs **104 ms** at 800 rows.
   - **My own first measurement had a hole**: it seeded the window's *view model* while
     `agent.goal()` reads the *durable store*, so it under-counted that cost by measuring an
     empty store. Properly measured, `agent.goal()` was **30 ms at 400 messages, 102 ms at
     1600**, twice per frame, **under the lock the agent writes through** — more than the
     whole transcript at an ordinary session size. Now cached and refreshed per event batch.
   Re-measure before sizing anything else here:
   `cargo test -p epsilon-desktop --test frame_cost -- --ignored --nocapture`
   **A benchmark that does not exercise the storage a code path actually reads will
   under-count that path, and it will look like a small number.**

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
  **fresh base label** every time, and copy the failing log into `audit/evidence/` *before*
  launching it.
- **`Avira LNK1104` on a fresh test binary.** The documented remedy is fresh test-profile
  metadata (`CARGO_PROFILE_TEST_DEBUG=1`), which renames every test executable and therefore
  forces a **full test-profile rebuild** — the test leg went from ~675 s to 2,402 s once and
  1,489 s another. Worth it once; worth knowing before you start one. It has now recurred on
  **the same path** (`epsilon_core-4a8081d0361ce74c.exe`), so the pin is not transient.
- **A long `cargo install`/battery can be killed by the harness's job runner**, not by a
  failing test: `3221225786` mid-`test`. That is not a gate result, and its partial log is
  evidence of something different from a failure — keep both.
- **A `#[test]` left above a `#[tokio::test]`** compiles as neither: *"async functions cannot be
  used for tests"*. When changing a test's async-ness, the attribute above it is part of the edit.

## The sweep's own limits, stated

Nothing below tessellation was observed (M252: no wgpu backend presents on this box), no
finding was reproduced, and no cost in the reliability report was measured — every magnitude in
it is read from the code plus the project's own recorded numbers. **The desktop frame path has
no measured number anywhere in the repository**, which is why sizing any fix for it needs a
measurement first.