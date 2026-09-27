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
| [`01-audit-2026-09-27.md`](01-audit-2026-09-27.md) | The independent audit: state, defects, TUI, metrics, port fidelity, plan | live |
| [`02-tui-freeze.md`](02-tui-freeze.md) | **The typing-freeze: root cause found and reproduced.** Not a render freeze | live |
| [`03-metrics-accuracy.md`](03-metrics-accuracy.md) | Context window, cache hit rate, cost accuracy defects | live |
| [`05-machine.md`](05-machine.md) | This machine: build/disk/verification procedure and its traps | reference |
| [`06-restructure.md`](06-restructure.md) | Repository layout restructuring | queued |

## Superseded notes (kept for history, not for direction)

The pre-2026-09-27 notes (`f09-path-policy.md`, `next-hardening.md`, `port-revalidation.md`,
`tui-redesign.md`) describe the previous line of work. `next-hardening.md` had already drifted —
its "next safe action" named M135 while M178 was committed. Read them for history; do not follow
them as instructions. `docs/history.md` and `git log` are the record of what actually landed.

## Where the truth lives

| Question | File |
|---|---|
| What is true of the code right now | [`agents/memory/01-audit-2026-09-27.md`](01-audit-2026-09-27.md) |
| What is measured, and what each measurement proves | [`agents/memory/00-verified-baseline.md`](00-verified-baseline.md) |
| What landed, in order | `git log`, plus `docs/history.md` |
| How to verify on this machine | `AGENTS.md`, `scripts/gate.ps1`, [`05-machine.md`](05-machine.md) |
