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
| [`00-session-takeover.md`](00-session-takeover.md) | What this session established, with the measurements | done |
| [`01-plan-execution.md`](01-plan-execution.md) | **Where I am in the 0-7 phases; inherited dirty work to triage** | live |
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

`docs/history.md` and `git log` are the record of what actually landed.

## Where the truth lives

| Question | File |
|---|---|
| What is true of the code right now | [`audit/01-current-state.md`](../../audit/01-current-state.md) |
| What is broken, ranked | [`audit/03-defects.md`](../../audit/03-defects.md) |
| What happens next, in what order | [`audit/07-plan.md`](../../audit/07-plan.md) |
| What landed, in order | `git log`, plus `docs/history.md` |
| How to verify on this machine | `AGENTS.md`, `scripts/gate.ps1`, [`05-machine.md`](05-machine.md) |
