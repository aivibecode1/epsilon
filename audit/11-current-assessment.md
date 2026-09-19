# 11 — Current assessment and the plan that follows

**Written:** 2026-09-19, working tree over `0548e9f`.
**Replaces as the live assessment:** [09 — source revalidation](09-revalidation.md),
which covered the tree at `80b32eb`. 09's findings are not withdrawn; they are
folded in below with their current status.

This file is the honest answer to three questions: what is actually true about
this codebase right now, what the evidence for that is, and what to do next in
what order. Where an earlier document claims something this file contradicts,
this file wins and says why.

## How this assessment was produced

Four independent passes, each read-only:

| Pass | Method | Artifact |
|---|---|---|
| Test-suite credibility | Read all 884 test bodies; hand-verified every candidate finding | `.tmp-console/test-audit.md` |
| pi port fidelity | Read pi 0.85.1's loop, prompt, skills, compaction, tools, streaming; compared against the Rust counterparts | `.tmp-console/port-fidelity-audit.md` |
| Gap and contradiction review | Reconciled `audit/00`–`10`, `docs/history.md`, milestone logs and `git log` | summarized below |
| Reference design study | Read pi, opencode and hermes terminal UIs plus DSH's token layer | `.tmp-console/tui-design-spec.md` |

Plus execution: a full-workspace test run on a fresh profile, a strict-Clippy
run, and a terminal-integration probe that drives the built binary in a real
Windows console.

Corrections to the brief that this assessment accepts as fact:

- **DSH has no terminal UI.** `references/deepseek-harness-dsh-v0.1.6-alpha.2` is
  a React-DOM web GUI; it contributes a design-token discipline, not a TUI to
  compare against.
- **opencode's TUI is SolidJS on `@opentui/core`**, not Go. Its Windows console
  shim over `kernel32.dll` is still the sharpest Windows workaround of the four
  and is cited as such.

## 1. Where the codebase actually is

**Verified green.** On a fresh test profile, every workspace crate passes:
`cargo test --workspace` reports 0 failures across all 12 crates. `cargo fmt
--all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the
offline demo and `cargo check -p epsilon-cli --features real` all exit 0.

**The test suite is materially better than this genre's norm.** 884 test
functions, one `#[ignore]`d, no `#[should_panic]`, no tautological assertions,
and exactly one body out of 884 with no assertion at all. That is not a claim the
genre supports by default and it should be stated plainly.

**The port is not complete.** `audit/03-port-plan.md` still carries capability
rows as Partial/Missing, and nothing in this assessment supports a complete-port
claim. What is faithful is the *shape*: the loop, the event taxonomy, the
terminal-frame contract, partial-output preservation, and the session and
compaction entry kinds all match pi. What diverges is the compression and
recovery machinery and the instruction-loading seams.

### What changed since 09

- **M95 paged reads landed** (`27cd677`). Reads that exceed the byte cap now
  page instead of refusing, with the write path refusing a wholesale overwrite of
  a file the agent has only partly seen.
- **M96 closed three port gaps** (`94df984`): the overflow classifier missed
  Anthropic's `request_too_large` spelling so the compaction-and-retry branch
  never fired; the compaction system prompt asserted a template it never sent;
  and a dead-code warning was silenced rather than papered over.
- **The TUI presentation layer was rebuilt** (`7a94ccd`, `59fdf2e`, `f3f82e6`,
  `7ae8673`). `crates/epsilon-tui/src/ui/` now owns theme, layout, metrics,
  header, transcript, composer, footer, overlay and shell, with 213 tests.
  Transparency is a data model (`ui/metrics.rs`), not a draw call.

## 2. Gaps that remain, ranked

### P0 — correctness or safety

| # | Gap | Evidence | Why it matters |
|---|---|---|---|
| G1 | The agent contract tells the model to load a skill body through a `skill` tool that does not exist in the registry, and the catalog prints no path | `crates/epsilon-core/assets/agent-contract.md:103`; `crates/epsilon-core/src/skills.rs:118`; `crates/epsilon-tools/src/lib.rs:109-138` | The model learns a skill exists and cannot open it. Fix is a tool, or a contract that names `read` plus a location. |
| G2 | F09 resolved filesystem containment is open | `audit/02-defect-ledger.md:157`; `agents/memory/f09-path-policy.md` | The native filesystem tools run inside the harness process; Windows Job Objects bound resources, not content. |
| G3 | Checkpoint restore conflates "skipped" with "deleted" (F15); a durable-commit failure is discarded (F07) | `audit/02` F07/F15 | Silent data loss in the recovery path, which is the path a user reaches only when something already went wrong. |
| G4 | Unsupported final entries are truncated (F08) | `audit/02` F08 | Same class: the difference between "there was nothing" and "we could not read it" is exactly what a recovery path must preserve. |

### P1 — behaviour a user would notice

| # | Gap | Evidence |
|---|---|---|
| G5 | `.gitignore` is not honoured by grep/find, and both tool descriptions say it is | `.tmp-console/port-fidelity-audit.md` D-9 |
| G6 | `disable-model-invocation` is parsed but never filtered out of the catalog | D-4 |
| G7 | `SYSTEM.md`, `APPEND_SYSTEM.md` and `.epsilon/prompts/` are trust-gate-detected but never loaded | D-7 |
| G8 | `AGENTS.override.md` and ancestor context files above the workspace are never loaded | D-8 |
| G9 | A `MaxTokens`-truncated summary is committed as if complete | D-5 |
| G10 | The TUI stops repainting after a tool approval in at least one reproduced sequence | `.tmp-console` conhost probe, 12 identical screen digests while the process stayed alive |

### P2 — structure and teaching value

| # | Gap |
|---|---|
| G11 | `crates/epsilon-tui/src/app.rs` is ~7,000 lines owning the event loop, key handling and legacy draw code |
| G12 | Five overlays still draw through the deprecated `crate::theme` shim; the shim cannot be deleted until they move |
| G13 | No terminal-integration test runs in CI, though two working harnesses exist locally |
| G14 | `epsilon-ext` has 5 tests for a whole crate — the least covered surface relative to its size |

### Bookkeeping defects (not code)

These are the contradictions a fresh reader would trip over, and they are worth
fixing precisely because they make every other claim harder to trust:

- `docs/history.md` carried two contradictory M95 rows; now reconciled by
  `27cd677`, but the pattern recurred across milestones.
- `audit/05-verification-log.md` is seven batteries stale relative to the
  milestone logs, while `AGENTS.md` calls it the latest baseline.
- The M95 review record lived only in `%LOCALAPPDATA%\Temp`, outside the repo.
- `audit/10-tui-enhancement.md` names evidence logs whose names disagree with
  their contents (`tui-full-green.log` is a RED run).

## 3. The plan

Ordered by what unblocks what, not by what is interesting.

**Phase A — finish the TUI (owner priority, in flight).**
1. Migrate the four remaining list overlays onto `ui::overlay` *(in progress)*.
2. Split `app.rs`: `event.rs` for the loop, `keys/` per mode, leaving the draw
   path as the shell call plus overlays.
3. Delete `crate::theme`.
4. Check the ConPTY harness into the repo as a test target that renders frames
   and asserts on them, so a visual regression is a failing test.

**Phase B — close the port's behavioural gaps.**
5. G1 (skill loading) — smallest fix with the largest effect on the model's
   behaviour.
6. G5–G9 — each is a few lines; each currently makes a tool or a prompt promise
   something the harness does not do.
7. G9 and G10 get a regression test before the fix, because both are
   "sometimes" failures and a fix without a reproduction is a guess.

**Phase C — the P0 correctness work.**
8. G3/G4 checkpoint restore semantics, with an explicit tri-state
   (restored / absent / unreadable) rather than a boolean.
9. G2 resolved containment, which is a design task, not a patch.

**Phase D — teaching value and the long tail.**
10. Rewrite `README.md` and the docs map around the module tree that now exists.
11. `epsilon-ext` test coverage.
12. The remaining parity rows in `audit/03-port-plan.md`.

## 4. What this assessment does not claim

- No Linux or macOS execution has been performed. The Windows evidence is real
  and specific; the cross-platform claim is not tested.
- No live provider call was made during this assessment; cost and cache figures
  are exercised by tests, not by a paid session.
- The reference studies are reads, not ports: a divergence identified here is
  evidence that the code differs, and for two of them (D-2, D-5) the *running*
  consequence is inferred from two code paths rather than observed.
