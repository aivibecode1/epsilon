# 08 — Restructure record

For reviewers distinguishing Phase 2 documentation changes from implementation.
Phase 1 approval was explicit: begin Phase 2, stop again before Phase 3.

## Completed slice: entry point and contributor/user guidance

README now distinguishes implemented mechanisms from verified workflows, includes
setup and test commands, names the actual twelve crates, and warns that a fresh
clone is not the heavily modified audited working tree. The source demo was tested
in Phase 1; a fresh install is not claimed tested.

AGENTS now gives strict Clippy flags, distinguishes Cargo dependency checks from
architectural policy, acknowledges existing contracts-layer drift without accepting
it, and resolves the conflicting cleanup instructions. Failed rounds preserve
evidence then clean; only verified CLI/TUI changes require reinstalling the global
binary. This prevents both stale installs and installation of a failing baseline.

The reviewed `docs/user/` guides are incorporated as the product-reference location.
Corrections cover monotonic denies, ineffective plan-mode shell restrictions,
platform limits, credential setup, refresh limitations and unsafe recovery claims.
These documents existed untracked before Phase 2; committing their reviewed current
content does not imply they were newly authored in this run.

## Instruction corrections and rationale

| Original guidance | Correction | Reason |
|---|---|---|
| Workspace lints deny warnings | Clippy explicitly uses `-- -D warnings` | Manifest lints are warnings; the command enforces the gate |
| Always reinstall before cleaning; clean only after green gates | Reinstall after verified CLI/TUI changes; preserve failures then clean failed rounds | Do not ship failing code or keep gigabytes merely because a test failed |
| Compiler enforces architecture layering | Cargo enforces declared edges/acyclicity, policy requires review | No independent allowed-edge policy check was established |
| Contracts contain no logic | Types-only remains intent; existing folding/validation is recorded drift | Documentation must not disguise implementation debt as compliance |
| Milestone log for every change | This phase record owns documentation-only restructuring | No runtime milestone or new behavior is being claimed |

No source fix, module move, dependency update, formatter write, install or reference
build is included. No unrelated implementation baseline is staged. Documentation
commits include the reviewed pre-existing edits in the named documents; they do not
manufacture earlier milestone commits.

## Existing moves: map, not newly executed migrations

Git reports deleted old paths and matching current destinations. These moves were
already present before this phase; this slice does not stage those deletions or
claim byte-identical renames:

| Old location | Existing working-tree location |
|---|---|
| `design/*.md` | `docs/design/*.md` |
| `plans/*.md` | `docs/plans/*.md` |
| `reports/*.md` | `docs/research/*.md` |
| Product reference formerly mixed into root README | `docs/user/` |

The corresponding source/history baseline remains dirty. A reliable clean-checkout
rollback point requires a separately reviewed baseline commit, not blindly staging
hundreds of files as part of this documentation slice.

## Verification

Local-link existence checks passed for README, AGENTS and all seven product guides;
`git diff --check` passed. A hash comparison of all files under `crates/` against the
Phase 2 start snapshot found no changes. `target/` remained absent; no rebuild was
needed for documentation-only edits. The Phase 1 gate failures remain open, not
waived. Link existence in the working tree is not proof those targets are committed.

## Remaining Phase 2 work

The architecture guide still mixes design proposals with implemented behavior; its
attempted full rewrite was refused by the edit tool and did not modify that file.
It must be reread and reconciled before claiming Phase 2 complete. Remaining docs,
historical move adoption and local-link/anchor checks across the documentation tree
also remain. Source splits are deferred under the approved roadmap's red-baseline
stop rule. No dead-code deletion is authorized by the audit.

**Status: Phase 2 in progress.** This is a completed documentation slice, not the
Phase 2 review checkpoint and not authorization to begin implementation.
