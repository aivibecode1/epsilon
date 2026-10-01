# Agent Memory

Durable working memory for coding agents operating in this repository. Long conversations get
compressed; these files are the deliberate form of that compression: hand-curated state that
survives context loss, written in the codebase where the work happens.

**This file is an index, not a log.** It says what is open, where the detail lives, and what to do
next.

## Rules

1. **Read before working.** Load the index and any file matching your task. Trust the code, the
   gates, and `git log` over these notes when they disagree.
2. **Update after every slice.** When a work item's status, decisions, evidence locations, or next
   step changes, update its file in the same turn.
3. **One file per active work item**, named by topic slug. Keep each file short.
4. **Working state, not product docs.** Product truth lives in `docs/` and `audit/`.
5. **Retire only after closeout.** Delete the note and its index row in the same change.

## Index

| File | Work item | Status |
|------|-----------|--------|
| [`00-session-takeover.md`](00-session-takeover.md) | What session 1 established, with the measurements | superseded — read for history only |
| [`01-plan-execution.md`](01-plan-execution.md) | Session 1's phase tracker, its traps, and three classes of self-inflicted error | live — the *lessons* still apply |
| [`02-tui-truth-first.md`](02-tui-truth-first.md) | **Session 2: "shrinks" ruled out, M194 + M195 landed** | live |
| [`03-port-gaps.md`](03-port-gaps.md) | **Session 2: the 24 missing pi capabilities; `branch_with_summary` wiring first** | live |
| [`04-session3-env-and-structure.md`](04-session3-env-and-structure.md) | **Session 3: the MSVC-env build blocker, the link-debt repair, G7 markdown** | live |
| [`build-blockers-resolved.md`](build-blockers-resolved.md) | **Five stacked environment faults that made this repo unbuildable — now fixed** | live |
| [`05-machine.md`](05-machine.md) | This machine: build/disk/verification procedure and its traps | reference |
| [`06-m197-baseline-and-ext-hang.md`](06-m197-baseline-and-ext-hang.md) | **M197: the tree was left RED, and a dead extension waited the full invoke budget** | live |
| [`07-m198-frame-path.md`](07-m198-frame-path.md) | **M198: render a frame and look at it — the transcript printed every marker twice** | live |
| [`frame-reading-pass.md`](frame-reading-pass.md) | **CLOSED by M201 — the findings, and the method that found them** | closed — the recipe still applies |
| [`m199-containment.md`](m199-containment.md) | **M199 landed: symlink containment, the bypass it nearly shipped, and the open items it left** | live |
| [`ranked-sweep-2026-09-29.md`](ranked-sweep-2026-09-29.md) | **The t2 ranked sweep, what M201 closed, and the order the rest goes in** | live — heads the implementation queue |
| [`08-goal-command.md`](08-goal-command.md) | **`/goal` is complete (state at M203, the round driver at M204, on the wire at M206) — the two follow-ups still open, and the five traps it met** | live — heads the capability queue |
| [`goal-gates.md`](goal-gates.md) | **A goal's completion rule, the sandbox trap in it, and the four things still missing** | live — heads the harness queue |
| [`prompt-cache.md`](prompt-cache.md) | **Where the provider cache marker goes, and the test that guards it** | live — a one-line rule with a regression test |
| [`desktop-ide.md`](desktop-ide.md) | **The native desktop IDE: the toolkit decision, the `epsilon-runtime` extraction, and the wiring slice that comes next** | **live — heads the implementation queue** |
| [`10-handoff-2026-09-30.md`](10-handoff-2026-09-30.md) | **Session handoff (M201-M214)** — state at HEAD, the defects this session added and where each stands, what to do next in order, and the traps it paid for | **read first** if you are picking this project up cold |
| [`f09-path-policy.md`](f09-path-policy.md) | **F09: the resolved-path policy** — the lexical resolver, what it delegated, and the containment work that followed | historical reasoning; the landed result is M199 (`m199-containment.md`) |
| [`port-revalidation.md`](port-revalidation.md) | **The autonomous audit + complete-port arc**: its authority and scope, and the port-fidelity thread it opened | standing context for why the port frames the project; superseded as *status* by `audit/01`-`07` |
| [`tui-redesign.md`](tui-redesign.md) | **TUI redesign: chrome economy** — the overlay-chrome migration across every list panel and prompt | landed and green at the time of writing; the later pass over the same surface is M201 (`frame-reading-pass.md`) |
| [`next-hardening.md`](next-hardening.md) | **This machine, and this repo's open threads** — procedure and traps, explicitly *not* status | reference; the open-item map of record is named inside it |

## Superseded notes (kept for history, not for direction)

The pre-2026-09-28 notes describe the previous line of work. Read them for history; do
not follow them as instructions.

- [`superseded/02-tui-freeze-wrong-root-cause.md`](superseded/02-tui-freeze-wrong-root-cause.md)
  — **its central claim is disproved.** It says the typing-freeze is a permission dialog
  that silently swallows keys. I measured typing working, on both a debug build and the
  installed release binary. See `audit/04-tui.md`.
- `f09-path-policy.md`, `next-hardening.md`, `port-revalidation.md`, `tui-redesign.md` —
  earlier work. `next-hardening.md` had already drifted before this session: its "next
  safe action" named M135 while M178 was committed.

**Three of these also name `D:/eps-target` as the build directory. That is withdrawn.**
The owner ruled on 2026-09-28 that builds go on `C:` in the repo's own `target/`, and that
directory has been deleted. Follow [`05-machine.md`](05-machine.md), not these files: a
note is a *claim about* the machine, and this one was wrong in a way that cost 21 GB and
two rounds of confident wrong answers.

`docs/history.md` and `git log` are the record of what actually landed.

## Where the truth lives

| Question | File |
|---|---|
| What is true of the code right now | [`audit/01-current-state.md`](../../audit/01-current-state.md) |
| What is broken, ranked | [`audit/03-defects.md`](../../audit/03-defects.md) |
| What happens next, in what order | [`audit/07-plan.md`](../../audit/07-plan.md) |
| What landed, in order | `git log`, plus `docs/history.md` |
| How to verify on this machine | `AGENTS.md`, `scripts/gate.ps1`, [`05-machine.md`](05-machine.md) |
