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

Local-link destination checks passed across 142 Markdown documents: root README and
AGENTS, the docs tree, and top-level audit reports. No missing local destinations
were found. Heading anchors, remote URLs and committed-checkout completeness were
not certified by this check. `git diff --check` passed. A hash comparison of all files under `crates/` against the
Phase 2 start snapshot found no changes. `target/` remained absent; no rebuild was
needed for documentation-only edits. The Phase 1 gate failures remain open, not
waived. Link existence in the working tree is not proof those targets are committed.

## Remaining Phase 2 work

The architecture guide has now been reread and rewritten against the audited
implementation. Its twelve production dependency rows were checked against Cargo
manifests; unsupported backend/API/parity claims were removed. The old tracked
`design/epsilon-architecture.md` deletion and reviewed replacement are committed
together as one documentation relocation. Remaining docs,
historical move adoption and local-link/anchor checks across the documentation tree
also remain. Source splits are deferred under the approved roadmap's red-baseline
stop rule. No dead-code deletion is authorized by the audit.

## Follow-up: historical moves adopted; verification remains red

The earlier move-map and verification sections describe their original slices.
Subsequently, the reviewed design records were adopted in `8533f23`, historical
plans in `a5f049f`, and research reports in `9427a0d`. Those commits incorporate
pre-existing content and path changes; they do not certify historical parity or
completion claims. Archive/status notices distinguish those claims from the audit.

A later workspace test run failed at the auth concurrency test before reaching
the known checkpoint failure. Ten isolated reruns of the same workspace-built
binary passed; the cause is not established. Evidence and completed build cleanup
are recorded in [05](05-verification-log.md#phase-2-follow-up-intermittent-auth-lock-failure).

The announcements about implementing F10 or an auth-lock fix were not followed by
source edits. No explicit implementation exception was approved. They must not be
treated as authorization to bypass the Phase 3 review gate.

Remaining Phase 2 work is bounded to final documentation reference/anchor review,
review of the referenced untracked support files, and a changed/deferred-path
report. Source splits remain deferred because the baseline is red. Unrelated
source and historical milestone changes remain unstaged.

## Phase 2 review handoff

**Status: documentation/support-file restructure complete; paused for review.**
This is the documentation-first delivery allowed by roadmap section “Source
organization” when the red baseline prevents confident source moves. It is not a
claim that all original Phase 2 acceptance conditions or the overall goal are met.

### Adopted scope

- Root README, AGENTS and the user guides now own current setup, limitations and
  contributor commands; docs/README is the navigation map.
- Architecture, ADRs, plans and research use the recorded `docs/` locations.
  Historical claims remain historical rather than being certified by relocation.
- Final reviewed glossary/QA/handoff changes correct paths, contracts-layer
  claims, README navigation policy and the superseded M16 handoff label. These
  include pre-existing edits; no historical implementation commits are invented.
- `52ec313` adopts the existing LICENSE and scripts/gate.sh unchanged. The MIT
  declaration and Mohamed attribution match both HEAD and working-tree manifests.
  The wrapper passed `bash -n`; its actual five-gate executions and failures are
  already preserved in 05. Adoption does not mean the gates passed.

### Deferred scope and reasons

| Paths/work | Disposition |
|---|---|
| CLI/TUI/core module splits, dead-code removal | Deferred under the red-baseline stop rule; no source moves or confirmed-dead deletions made |
| Existing `crates/`, Cargo manifests/lockfile, CHANGELOG and milestone/history edits | Remain the owner's dirty implementation baseline; require separate preservation/review, not a documentation commit |
| `scripts/rpc_probe.py` | Remains untracked: disables sandbox at line 38, automatically approves at 93–101, and permits real-provider selection without a script; its success check at 148 does not require tool execution as its header claims. Not run or adopted |
| `scripts/regen_catalog.py`, `.agents/skills/ratatui-tui/` | Remain outside this slice; no regeneration or skill installation performed |
| `audit/references/` | Local untracked historical leads; index points to committed report 06 rather than linking an uncommitted directory |

### Final verification and limitations

A local checker scanned **143 Markdown documents** (root README/AGENTS, docs tree,
and top-level audit reports), inspecting **152 local inline-link/reference-definition
destinations** before this handoff was added. No missing destinations or unresolved
heading fragments were found. Fenced examples were excluded. This is a local
Markdown check, not a renderer, remote URL check, or proof of clean-checkout
completeness. Untracked historical milestone targets still exist only in the
working tree. No blanket claim that every Markdown syntax form was validated.

Source hash comparison confirmed all 166 files under crates/ unchanged during
this closing slice. Strict Clippy and checkpoint test
failures remain open, as does the intermittent auth-lock failure. See 05 for
partial test results and cleanup; no green baseline is claimed.

### Decision required

Review this documentation-first delivery and approve or revise the source-work
deferral before Phase 3. Recommended next authorized implementation is roadmap A
(F00 baseline/checkpoint correction and F01 file preservation), with the new
intermittent auth failure included in baseline qualification. Preserve/review the
existing dirty source baseline first. Automated goal continuation is not approval.
