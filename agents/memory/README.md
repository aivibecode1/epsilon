# Agent Memory

Durable working memory for coding agents operating in this repository. Long conversations get
compressed; these files are the deliberate, intelligent form of that compression: hand-curated
state that survives context loss, written in the codebase where the work happens.

**This file is an index, not a log.** It says what is open, where the detail lives, and what to do
next. Round-by-round narrative belongs in a topic note or in `docs/milestones/`, and a note that
grows past a screen is a note that has stopped being read. The previous version of this file was
373 lines of per-round state and had drifted far enough from the code that three of its "still
open" items had been fixed for a round; that is the failure this rewrite exists to prevent.

## Rules

1. **Read before working.** Load the index below and any file matching your task. Trust code,
   gates, and `git log` over these notes when they disagree — a stale "still open" line here has
   cost real time, because the note was written by an agent that believed it when it wrote it.
2. **Update after every slice.** When a work item's status, key decisions, evidence locations, or
   next step changes, update its file in the same turn — do not end a turn on stale state.
3. **One file per active work item.** Name files by topic slug (`m135-read-guard.md`, not
   `notes3.md`). Keep each file short: status, decisions, changed files, evidence paths, next step.
4. **Working state, not product docs.** Product truth lives in `docs/` and `audit/`. These files
   record *where work stands*. Never put secrets or credentials here.
5. **Retire only after closeout.** Keep a work item until its documentation, verification,
   installation (when required), and cleanup are complete. Then move durable findings to the
   milestone/ledger and **delete the note and its row in the same change** — an unlisted note is
   lost state, and a listed note whose work is closed is a false one.
6. **Checkpoint before context loss.** Save the goal, verified facts versus assumptions, decisions,
   blockers, exact evidence paths, running process IDs, dirty-work boundaries, and the next safe
   action. Update existing topic files instead of appending a transcript.

## Index

| File | Work item | Status |
|------|-----------|--------|
| [next-hardening.md](next-hardening.md) | **Read before editing anything**: this machine's verification procedure, the traps that cost real time, and the ranked open queue | reference |
| [port-revalidation.md](port-revalidation.md) | The arc: every milestone, battery label, evidence path and limit from the audit to the present | the account |
| [tui-redesign.md](tui-redesign.md) | TUI design rules, the render-path traps, the conhost probe; the owner's stated priority | live |
| [f09-path-policy.md](f09-path-policy.md) | F09 resolved filesystem containment — a design task, not a patch | open; the M84 scheduling slice is closed |

Retired 2026-09-25, their durable content already in `docs/milestones/`: `metrics-and-reader-honesty`
(M106–M111; its one open thread — the header's unexplained `?`, `—` and `+` — moved to the queue in
`next-hardening.md`), `session-store-integrity` (M105; its three open items were closed by M108, M122,
M124 and M133), and `surface05-paged-reads` (M95; the hole it carried in its `Limits` — an `edit`
promoting a partial read into a whole-file licence — was closed by M135).

## Where the truth lives

| Question | File |
|---|---|
| What is true of the code right now, and what to do next | [`audit/11-current-assessment.md`](../../audit/11-current-assessment.md) |
| Which defects are open | [`audit/02-defect-ledger.md`](../../audit/02-defect-ledger.md) — every row carries a status, and a row is not evidence: the milestone it names is |
| What landed, in order | `git log`, plus `docs/history.md` and `docs/milestones/` |
| How to verify | `AGENTS.md` and `scripts/gate.ps1`; the executed record is `audit/05-verification-log.md` and `audit/evidence/gate-*.log` |

## Next safe action

- **M135 in flight: an `edit` no longer hands the model the rest of a partly-read file.** The F01
  guard was defeatable in three ordinary tool calls, because `edit` recorded its success through
  `read_log::record_write`, whose contract ("the bytes it just wrote are known in full") is true of
  `write` and false of a targeted replacement. Fixed with a second recorder, `record_edit`, which
  refreshes the stat and carries `read`/`complete`/`capped` while discarding the line arithmetic.
  Files: `crates/epsilon-tools/src/{read_log.rs,files.rs}`,
  `crates/epsilon-tools/tests/builtin_tools.rs`, `docs/milestones/M135-*.md`, the ADR-005 amendment.
  **Negative control executed** (both `edit` call sites reverted to `record_write`): the behavioural
  test fails with `Ok("overwrote book.txt (19 bytes)")` — the defect reproduced. Restored
  byte-identically, sha256 `9ff4a6ae…`, and touched.
  **Battery `m135a` runs against an external `CARGO_TARGET_DIR`**: the in-repo artifact path is the
  one Avira pins, and an in-repo run produces 15 spurious `Access is denied` failures in tests that
  pass from any other path. Fill the milestone's Verification from the log, then commit.
- **Do not trust a "still open" line in this folder without checking the code.** Blockers B2–B5 from
  `audit/evidence/m126-m127-review.md` and the `auth.lock` sharing-violation retry were both listed
  open for a round and were already fixed (M130; `auth.rs:278-289`). The session-format gaps 4A and
  4B were listed open and had landed in M113 and M119.
- **Unrelated dirty work — never stage it:** `.agents/skills/ratatui-tui/**` (6 files) has been
  modified by something outside this task since 2026-09-23.
- **The next slices are ranked in `next-hardening.md`.** The port is in better shape than the notes
  claimed: the open items were re-derived from the code rather than inherited, so re-read the code
  before picking one.
