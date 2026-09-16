# 00 — Independent revalidation scope

**Audience:** owner reviewing Phase 1 and engineers interpreting this audit.
**Run:** 2026-09-16, Hermes; Windows, working tree over `e200815`.
**Status:** Phase 1 review checkpoint. Independent source review and gate evidence are consolidated; Phase 2 has not started.

## Baseline, not a clean checkout

The run began with 218 normal short-status entries: 87 modified, 23 deleted,
and 108 untracked entries. Untracked directories are collapsed by this Git mode;
these are not file counts. The branch is `main`; the index was empty. Existing
source changes, document moves and `docs/user/` predate this revalidation.
Do not attribute those changes to this run or reconstruct milestone commits from them.

`Cargo.toml:3–15` lists twelve workspace crates. A Python filesystem inventory
counted 118 Rust files under crate `src/` directories, with 60,804 physical lines,
and 24 Rust files under crate `tests/` directories. Physical lines include comments,
inline tests and generated data; these are not implementation LOC or test counts.
`crates/epsilon-llm/src/catalog/catalog_data.rs` accounts for 6,701 lines of generated
catalog data. The 6,404-line TUI `app.rs` is a different maintenance concern.

No `target/` existed initially; C: had about 63 GiB available. This run started one
full gate battery, not one build per finding. Exact outcomes belong in
[05](05-verification-log.md), not a remembered milestone total.

## Corrections already established

- `.agent/` is absent, but development skills exist under `.agents/skills/`.
  Absence of the singular spelling does not imply missing project guidance.
- The old F19 statement that the current README lists eleven crates and claims a
  green baseline is stale. `README.md:17–21,155–168,185–193` already names the current
  defects and lists twelve crates. The runtime-dependency, plan-mode and architectural
  enforcement claims still require correction (`README.md:8–10,107,148–157`).
- `docs/README.md:62–63` refers to a restructure move map in 05, but the inspected 05
  has no move map. Existing documentation reorganization is not proof of Phase 2
  approval or completion in this run.
- The Bash wrapper resolves Cargo in this session. Its earlier PATH failure may be
  genuine historical evidence; it is not the current environment diagnosis.

## Source checks completed directly

F01 is confirmed in both edit paths: `util.rs:171–194` in `epsilon-tools` returns
lossy text up to the byte cap and discards the capped flag. `files.rs:482–499` and
`:620–634` write a transformation of that prefix as the whole file. This is a
source trace, not an executed destructive reproduction.

F02 is confirmed: `read_log.rs:90–92,120–139,153–179` shares observations by path,
not agent; `epsilon-core/src/exec.rs:661–669` supplies the constant session ID
`inline`. Global mutation serialization should remain, but knowledge of the old
file must belong to the observer. The module's stated rationale at
`read_log.rs:16–18` conflates those different responsibilities.

## Instruction precedence for this phase

The user's explicit Phase 1 scope permits only audit document changes and asks for
small commits as documents finish. Therefore:

- The local `improve` skill's `plans/` output and no-commit defaults yield to
  `audit/` output and audit-only commits. Its read-only source discipline stays.
- Source fixes, restructuring, new tests in source, global installation, and edits
  to `AGENTS.md` or project skills are deferred. These would violate this phase.
- The general requirement to add new documents to `docs/README.md` is deferred;
  `audit/README.md` owns this phase's navigation. No undocumented source move is made.
- Failed-gate evidence is retained before cleaning build artifacts. Failure does
  not justify keeping a large cache indefinitely or weakening a gate.

These are task-specific precedence decisions, not permanent replacement rules.
The source-fixing stages of the debugging/review skills are not authorized here.
No credential files, paid provider calls, global cache purges, or reference builds
are needed to validate these claims.
