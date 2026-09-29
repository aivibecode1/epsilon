# Working note — the t2 ranked sweep, and what it changed about the queue

**Status:** t2 done. The deliverable is the **Reconciliation** section at the top of
[`audit/03-defects.md`](../../audit/03-defects.md), not a parallel file — a second
ledger is a ledger that drifts. Frames: `audit/evidence/frames/m199-baseline/`.

## The order work actually goes in, and why it is not `audit/07-plan.md`

`07-plan.md` ranks by *phase* (honesty → harness → TUI → numbers → port →
structure). That was the right order when the plan was written, because the
reconciliation above shows most of its early phases are **already done**: D1/D21,
D6/D19, D7/D18, D13, D15, D16 are all fixed and were fixed with measurements.

So the live order is now **user-visible impact × confidence**, and it starts with A1:

| Row | What | Why here |
|---|---|---|
| **A1** | draw path drops a character at the wrap boundary | Silent corruption of the conversation, at a width people use. Measured, root cause narrowed. |
| **A2** | header loses `↑` at 60 cols | A bare `0` reads as a value. |
| **A3** | cwd abuts the key hints | You cannot read the footer. |
| **A6** | composer placeholder drawn as a notice | Glyph meaning the wrong thing, on screen always. |
| A4/A5 | empty state repeats the header; three left edges | First impression, cheap to fix once A6's glyph work is in. |
| A7 | killed TUI blocks 30 s | Real, but a design decision (pid probe needs `unsafe`). |
| A8–A13 | lsp dep, `.gitattributes`, 8 `#[allow]`s, config ownership, missing legend | Low blast radius, mechanical. |
| A14/A15 | secret scrubbing and the `read`/`@` asymmetry | **Decisions, not patches.** Ask the owner. |

## The method fact worth keeping

A scratch probe (`wrap_line` over the exact fixture string, widths 56–61) proved
`wrap_line` is **content-intact at every width** — so the character loss in A1 is
in the *draw* path, and reading `wrap.rs` could not have found it because the
wrapper is correct. The repo already has a property test asserting no content is
lost; it passes, and the bug is downstream of the thing it tests.

**A test that proves a component is correct is not a test that the component is
used correctly.** That is the M198 lesson in a third costume.

## Two greps that would have produced false rows

1. `grep -rl "epsilon-core::" crates/epsilon-server/src` → **0 hits for all five
   deps**, because Rust crate paths use underscores. It looked like D2 confirmed.
   D2 is a *retraction*; the identifier was the error.
2. Counting `.unwrap()` across `crates/*/src` → 365 in the TUI, which says nothing:
   `#[cfg(test)] mod tests` is in the same files. Production unwraps need a
   rust-aware pass, not a grep. **Not attempted — recorded as a gap.**

## Gaps in this sweep, stated rather than hidden

- **No security advisory scan.** No `cargo audit`, no network. Dependency risk is
  therefore *unassessed* — exact pins are verified, their vulnerability status is not.
- **No `cargo deny`/license check.** Same reason.
- **The unwrap/panic surface in production paths is unmeasured** (see above).
- **D4's architecture-guide claims were not re-verified line by line.** The guide
  exists and cites 3 line references; whether they point at the right code is
  unchecked. It was re-derived 2026-09-28 and nothing has landed since that would
  move those three references, but "probably still right" is not "verified".
- **Port fidelity was not re-measured.** `02-port-fidelity.md`'s 38/22/24 split is
  inherited, not re-derived, and the goal is quality-first — so it stays inherited
  and is labelled as such.
