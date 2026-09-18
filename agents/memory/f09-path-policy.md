# F09 — resolved path policy

## Starting point (historical)
- Shared resolver `epsilon-core/src/paths.rs` is lexical; its comment delegated native
  symlink safety to the sandbox, but native filesystem calls are not surrounded by it.
- F09 open: read/write/edit/search/attachments/checkpoint identity must agree.
  Canonicalize-then-open is not race-resistant containment.

## Closed: the find scheduling slice (M84, commit d0ac16b)
- RED: expected directory key, actual `None` (isolated HEAD + test export).
- Fix: `FindTool::resource_key` delegates to the existing search-directory helper
  (now crate-visible); default and explicit roots join the planner's directory-prefix
  conflict checks against writes. Only find.rs/search.rs source changed.
- GREEN: regression + 78/78 tools library tests; unchanged repeat battery GATE PASS
  (741). Failed rounds preserved and explained (auth-lock timing test; compiler-startup
  0xc0000142 at jobs=2). Independent review approved the bounded change.
- Final acceptance 2026-09-18: fresh candidate `d9c1810` + only find.rs/search.rs
  (sha256 c7fc7957…/87800abf…), `CARGO_BUILD_JOBS=1`, external target →
  **all five gates exit 0, 743 passed / 0 failed / 0 ignored** (55 summaries),
  `audit/evidence/gate-f09-r2.log` (sha256 6c333c23…); candidate + target removed
  (57G free), `f09-r2-cleanup.log`. No CLI/TUI source changed → no reinstall.

## Still OPEN: resolved containment (the real F09)
- Alias spellings (symlink/reparse), replacement races, and attachment/checkpoint
  identity remain unresolved; the sandbox backends do not wrap native tool calls.
- Seam map: `audit/evidence/f09-path-policy-map.md` (consumers still to convert).
- Next defensive design task: alias-consistent identity for permission subjects,
  resource locks, attachments and checkpoint paths (including existing-parent
  resolution for new files). No exploit reproductions; Windows/Linux differences
  stay explicit. Ledger: `audit/02-defect-ledger.md` F09.

## Operating notes (this machine)
- One build owner; batteries run with `CARGO_BUILD_JOBS=1` and an external
  `CARGO_TARGET_DIR` (every failure at parallel jobs was 0xc0000142 compiler startup;
  single-job + external has passed every round since).
- `kill -0` is unreliable on MSYS for liveness — use the process tool or log growth.
- Evidence out of `target/` before cleanup; clean after each round, never mid-round.
