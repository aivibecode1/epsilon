# Epsilon audit

**Current assessment:** [09 — source revalidation and port-first plan](09-revalidation.md),
2026-09-17, working tree over `80b32eb`: five gates passed, 795 tests passed,
0 failed, 1 ignored. Known correctness and port gaps remain. The owner now
authorizes autonomous audit, restructure and implementation commits; no push
or paid provider calls are implied.

## Historical Phase 1 assessment

The remaining sections describe the 2026-09-16 review and its then-current
authorization. Its red baseline and mandatory review stops are historical, not
today's status or instructions.

**Audience:** the owner reviewing scope, and engineers implementing the approved roadmap.
**Date:** 2026-09-16. **Baseline:** working tree over `e200815`, not the committed tree alone.

## Verdict

Epsilon is a substantial Rust coding harness, but it is neither production-ready nor a complete port of pi. The primary blockers are **data preservation, real transport behavior, execution boundaries, and disagreements between integrated components**, not a lack of tool names or another framework.

Fresh Windows verification passed formatting, the offline demo, and real-provider compilation. Strict Clippy failed, and the workspace test run stopped at a checkpoint eviction test failure. These results replace earlier blanket claims of green gates. Source inspection additionally found defects that the existing tests do not exercise.

## Independent revalidation

The Hermes revalidation is complete at the Phase 1 review checkpoint. Read
[00 — Scope and corrections](00-revalidation-scope.md) and
[07 — Internal inconsistencies](07-inconsistencies.md) alongside the reports below.
Fresh execution independently reproduced the two Clippy errors and checkpoint test
failure; selected portable evidence and cleanup results are in 05. Source review
confirmed the main defect mechanisms and narrowed overbroad claims. It did not
execute every proposed regression or certify every reference capability.

## Read in this order

| Document | Purpose |
|---|---|
| [01 — Current state](01-usability-review.md) | Architecture, implemented surfaces, tested behavior, operational limits |
| [02 — Gap analysis and defect ledger](02-defect-ledger.md) | Prioritized findings, traces, evidence, confidence and acceptance tests |
| [03 — Pi port fidelity](03-port-plan.md) | Concrete parity matrix and intentional versus unexplained divergence |
| [04 — Sequenced roadmap](04-roadmap.md) | Dependencies, engineering estimates, Phase 2 scope and Phase 3 acceptance criteria |
| [05 — Verification record](05-verification-log.md) | This run's measured outcomes; prior unverified log replaced |
| [06 — Reference comparison](06-reference-comparison.md) | What to retain/adopt from pi, DSH, Hermes and opencode, and what not to import |

## Evidence rules

- **Executed:** observed from a command in this audit. Only the verification record establishes fresh test results.
- **Source-verified:** control flow and its callers were inspected. This establishes the implementation, not an executed reproduction of every consequence.
- **Inferred:** a conditional consequence or a delivery estimate. The trigger and remaining verification are stated.
- **Unverified:** no adequate evidence collected; do not promote it to a working capability.
- Citations are repository-relative `path:line` anchors in the inspected working tree. Reference citations are to the bundled snapshots, not current upstream HEAD.
- Parallel read-only reviewers examined safety/tools/LSP, core/session/providers, pi fidelity, and the other references. High-impact defect paths were checked again by the lead reviewer. Reference coverage is targeted, not a claim that every source line or every product was tested.
- No live provider calls, credential-store inspection by the auditor, dependency advisory scan, benchmark, interactive terminal session, Linux execution, or macOS execution was performed. The demo's normal startup did inspect local configuration and reported credential availability; no credential value is copied here.

## What changed in this phase

Only audit documentation was authored. No implementation fix, refactor, install, release, or repository reorganization was performed. The independent revalidation committed audit documents incrementally as the user requested; pre-existing source changes remain untouched. Verification created ordinary build artifacts; their cleanup is recorded in 05. Root README and `docs/` corrections are deferred to Phase 2.

The repository contains `.agents/skills`, not `.agent`. Relevant local skills were read: `improve`, `ponytail`, and `ratatui-tui`; the audit playbook was applied. The user's explicit output directory and review checkpoints take precedence over the advisor skill's default `plans/` workflow.

## Disposition of the previous audit

The top-level 01–05 reports are replaced, not adopted as a baseline. Earlier claimed execution logs were not rerun retroactively or retained as current proof. Reports under the local, untracked `audit/references/` directory are historical leads, not current evidence; [06](06-reference-comparison.md) is the committed comparison.

Corrections include:

- **12 workspace crates**, not 11 (`Cargo.toml:3–15`); LSP is already present.
- M79 is documented and checkpoint source marked M80 is also present (`crates/epsilon-tools/src/files.rs:479–482`). Do not describe this working tree as only M33–M62.
- The previous statement that the loop has “nothing missing” is rejected; see F03–F08 and the pi matrix.
- “Verification records exit status” and “global read stamps protect parent/child writes” are contradicted by integration paths: F05 and F02.
- “Every tool is OS-sandboxed” is false: native filesystem tools execute inside the harness process. Windows Job Objects do not restrict filesystem access: F09.
- Checkpoints are no longer wholly absent, but their restore behavior needs hardening: F01/F15.
- No claim that Epsilon's sessions surpass every reference, or that every reference has live API tests in CI, is retained without evidence.
- A historical failed real-provider call is not evidence of a successful coding workflow.

## Review checkpoint

**Stop after Phase 1.** Review the defect priorities, supported-platform target, and the definition of pi completion in 03 before authorizing Phase 2. Phase 3 requires a separate review after restructuring. The current user request authorizes granular audit-only commits, not implementation, pushes or paid API calls.
