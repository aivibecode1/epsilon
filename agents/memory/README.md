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
| [`02-tui-truth-first.md`](02-tui-truth-first.md) | **Session 2: "shrinks" ruled out, D30 landed, K1 + C2 metrics fixed** | live |
| [`05-machine.md`](05-machine.md) | This machine: build/disk/verification procedure and its traps | reference |

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
