# Working note — the t2 ranked sweep, and what it changed about the queue

**Status:** t2 done; the TUI half of the queue is now **closed by M201**. The deliverable was
the **Reconciliation** section at the top of [`audit/03-defects.md`](../../audit/03-defects.md),
not a parallel file — a second ledger is a ledger that drifts. Frames:
`audit/evidence/frames/m199-baseline/` and `audit/evidence/frames/m201/`.

## What is left, in the order it should go

M201 closed A1, A2, A3, A4, A5, A6 and A13, plus a defect no row named. The rows still open,
re-ranked by what each one costs:

| Row | What | Why here |
|---|---|---|
| **A12** | configuration is read in four places (`core/config.rs`, `llm/custom.rs`, `llm/oauth_login.rs`, three in the CLI) | Growing surface with no owner; every new setting makes the drift worse. Cheap to name an owner, expensive to keep deferring. |
| **A11** | delegated-subagent and compaction usage has a reader and no writer | About money, silent, unbounded — but it is a **design decision** (which model is a summarization billed to?), so it needs an answer before code. |
| **A7** | a killed TUI blocks the workspace for 30 s | The TUI shows nothing because the alternate screen erases the CLI's countdown. A startup-path fix, not a frame fix. |
| A8/A9/A10 | `epsilon-tools` → `epsilon-lsp` dep, no `.gitattributes`, 8 identical `#[allow]`s | Mechanical. A9 needs its own commit: `* text=auto` marks every file for renormalization and the next `git add` would rewrite line endings across the tree. |
| A14/A15 | secret scrubbing in the durable store, and the `read`/`@` asymmetry | **Decisions, not patches.** Both are answered by one paragraph in `docs/user/permissions.md`; neither needs code to be honest. |

Beyond the ledger, the next capability slice is `/goal` — a persisted, auto-continuing session
objective, which the DSH reference documents in full. Notes: [`08-goal-command.md`](08-goal-command.md).

## M202 was not a row, and it was the most valuable find of the two rounds

Driving the **installed binary** through `scripts/console-probe.py` — the only check in this
repository that runs what a user runs — found that `epsilon` would not start in this
repository's own directory: `Usage::cache_reported` was added without a serde default, and
`Usage` is inside every assistant message, so every session file older than M194 was
unreadable. No test could see it, because every fixture is built with the current shape.

**The tool to reach for next time is the probe, not another test.** It answers a question no
in-tree test can: *what does the shipped artifact do in a real console, in a real workspace?*
It costs one command and about ten seconds:

```sh
python scripts/console-probe.py --late --size 80x24 "C:\Users\Hp\.cargo\bin\epsilon.exe" ""
```

Two shapes of finding it can produce that nothing else can: a refusal at startup (this one), and
a frame that differs from what `frame_dump` renders (the rows come back as JSON). Run it after
every reinstall.

## The method fact worth keeping

A scratch probe (`wrap_line` over the exact fixture string, widths 56–61) proved
`wrap_line` is **content-intact at every width** — so the character loss in A1 was in the
*draw* path, and reading `wrap.rs` could not have found it, because the wrapper was correct
about content and wrong about *width*. The repo already had a property test asserting no
content is lost; it passed, and the bug was downstream of the thing it tested.
M201 added the assertion that was missing (`no_row_is_wider_than_the_width_it_was_given`,
widths 1..=len) rather than changing the one that passed.

**A test that proves a component is correct is not a test that the component is used
correctly.** That is the M198 lesson in a third costume.

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
  unchecked.
- **Port fidelity was not re-measured.** `02-port-fidelity.md`'s 38/22/24 split is
  inherited, not re-derived, and the goal is quality-first — so it stays inherited
  and is labelled as such.

