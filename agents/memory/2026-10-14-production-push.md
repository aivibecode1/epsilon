# Working note — 2026-10-14 session: production-readiness push

**State at session start:** commit `a3875fe` + ~6.4k lines of uncommitted, individually
battery-verified work (22 slices, 2026-10-13/14). Owner instruction this session: take the
project to production-ready — audit everything, prioritize by user value, implement, verify
with the full battery plus real terminal-interface verification, preserve logs.

## What landed (this session)

- **`52b246d`** — the whole verified working stack settled into the commit record (one
  commit; the hunks interleave across keys.rs/panels.rs/commands.rs/state.rs/layout.rs/
  wrap.rs so per-feature commits would need a build per intermediate tree). The message
  names all 22 slices with their GATE PASS logs. All ten non-build gate legs re-run green
  on the tree before the commit (check-links, check-agents-lock, check-lint-tables,
  check-parity-citations, check-citation-placement, check-reference-citations,
  check-verb-counts, check-parity-counts, check-line-endings, check-control-bytes).
- **In flight:** mechanical split of `crates/epsilon-core/src/lib.rs` (4,709 lines) into
  child modules — Phase 5.1 of `audit/07-plan.md`, zero behaviour change, re-exports keep
  every original path compiling.

## Audits dispatched (read-only, parallel)

Five investigators writing full reports to `C:/Users/Hp/AppData/Local/hermes/cache/scratch/`:

| Report | Subject |
|---|---|
| `audit-defects.md` | Re-verify the 13 recorded open defect/roadmap findings (LIVE/STALE/PARTIAL) |
| `audit-pi-port.md` | Verify every missing/partial row of `docs/parity/pi-tui.md` + uncovered pi capabilities |
| `audit-references.md` | 5-10 high-value capabilities from DSH/opencode/Hermes/OpenTUI we lack |
| `audit-tui-polish.md` | TUI polish defects from the captured frames + draw code |
| `audit-quality-docs.md` | Oversized files, duplication, dead pub code, docs-vs-code contradictions |

## Machine rules in force

`CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=%LOCALAPPDATA%/Temp/epsilon-target`, debug info off,
TMP/TEMP/TMPDIR to `%LOCALAPPDATA%/Temp`. One build owner: implementation children
serialized against audits and each other by file ownership. Installed `epsilon.exe` is the
Oct-1 build — real terminal verification (`scripts/console-probe.py`) requires a reinstall
at the end of the round.

## Next

1. Integrate the five audits into one ranked backlog (user value first).
2. Implement slices: tests-first, frame witnesses for anything drawn, negative controls.
3. Final full battery under a fresh label + copy the log to `audit/evidence/`.
4. `cargo install` the proved bytes, run `scripts/console-probe.py` against the installed
   binary, keep the JSON.
5. Update STATUS.md, this note, and the ledger; commit per slice.
