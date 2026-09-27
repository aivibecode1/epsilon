# Plan execution — phases 0-7

**Status: Phase 0 starting. Audit committed as `b8b9b18`.**

Source of truth for *what* to do: `audit/07-plan.md`. This file tracks *where I am* and
*what I verified on the way*, so a context loss does not lose the position.

## Done

| Commit | What | Evidence |
|---|---|---|
| `47ca904` | Per-frame context-mirror measurement (test only, no product change) | 16.9µs@20 msgs, 216.7µs@200 — 12.9× for 10× transcript |
| `b8b9b18` | The audit: 01–07 written, 2 retractions, 5 new defects (D17–D21) | every claim names its command |

## Inherited uncommitted work — NOT mine, decide before touching

- `crates/epsilon-cli/tests/terminal.rs` — M179, falls back to `CARGO_BIN_EXE_epsilon` so
  `cargo test --workspace` stops skipping the suite. **Judged correct; adopt it.**
- `AGENTS.md` — M179 prose about the same. Adopt with it.
- `.agents/skills/ratatui-tui/` (6 files) — **not reviewed.** The user distrusted the
  skills; this one I judged genuinely useful but have not verified the diff.
- `docs/design/epsilon-architecture.draft.md` (untracked) — the layout subagent says this
  is the real architecture doc and the live 172-line one is a stale disclaimer. **Verify
  before promoting.**

## Phase 0 — honesty (no product code; suite state must stay unchanged)

- [ ] 0.1 fix the 2 dead README links
- [ ] 0.2 repair `docs/README.md` (18 orphan rows above the heading)
- [ ] 0.3 mark or delete `docs/qa/03-partial-vs-full-port.md` (stale by 5 milestones)
- [ ] 0.4 `git rm --cached audit/evidence/*.log` (173 files, ~11MB) + ignore rule
- [ ] 0.5 delete 6 `ponytail-*` skills + 4 `.tmp-*` files
- [ ] 0.6 promote the architecture draft
- [ ] 0.7 fix ~155 broken links

**Verification for all of Phase 0:** docs only, so `cargo test` state must be identical.
Run the link checker; do not run a full battery for a docs commit (it costs 30–45 min and
buys nothing here).

## Then

Phase 1 (harness) → Phase 2 (TUI) → Phase 3 (numbers) → Phase 4 (port) → Phase 5 (structure).

**The single most important next deliverable is Phase 1**: a provider that streams over a
controllable duration, reachable from the TUI, plus a test that types during a live turn.
Nothing in Phase 2 is verifiable without it, and the whole point of this audit is that
the previous pass claimed a root cause it could not reproduce.

## Traps on this machine

- Build owner is one at a time. `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`.
- Wipe `D:/eps-target` after each round; C: is at 93%.
- `pwsh -File scripts/gate.ps1 <label>` is the battery that works here.
- Avira can pin a test-exe path (`LNK1104`). Never change AV config from a session.
- `rm .epsilon/sessions/tui.lock` before EVERY probe run (D20).
