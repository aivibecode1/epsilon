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

Four independent passes, each read-only. The reports are checked in beside this
file rather than left in a scratch directory, because a finding that only exists
in a temporary folder is a finding the next reader has to redo:

| Pass | Method | Artifact |
|---|---|---|
| Test-suite credibility | Read all 884 test bodies; hand-verified every candidate finding | [`evidence/test-suite-audit.md`](evidence/test-suite-audit.md) |
| pi port fidelity | Read pi 0.85.1's loop, prompt, skills, compaction, tools, streaming; compared against the Rust counterparts | [`evidence/port-fidelity-audit.md`](evidence/port-fidelity-audit.md) |
| Reference design study | Read pi, opencode and hermes terminal UIs plus DSH's token layer | [`evidence/reference-tui-design-spec.md`](evidence/reference-tui-design-spec.md), [`evidence/reference-harness-ideas.md`](evidence/reference-harness-ideas.md) |
| Gap and contradiction review | Reconciled `audit/00`–`10`, `docs/history.md`, milestone logs and `git log` | summarized below |

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
| ~~G1~~ | **Closed** in M97 (`2d50813`): the `skill` tool now exists and is registered from the discovered list, and the catalog no longer advertises skills the model may not load (`87c5628`). | new tests in `crates/epsilon-tools/src/skill.rs` | — |
| G2 | F09 resolved filesystem containment is open | `audit/02-defect-ledger.md:157`; `agents/memory/f09-path-policy.md` | The native filesystem tools run inside the harness process; Windows Job Objects bound resources, not content. |
| G3 | Checkpoint restore conflates "skipped" with "deleted" (F15); a durable-commit failure is discarded (F07) | `audit/02` F07/F15 | Silent data loss in the recovery path, which is the path a user reaches only when something already went wrong. |
| G4 | Unsupported final entries are truncated (F08) | `audit/02` F08 | Same class: the difference between "there was nothing" and "we could not read it" is exactly what a recovery path must preserve. |

### P1 — behaviour a user would notice

| # | Gap | Evidence |
|---|---|---|
| ~~G5~~ | **Closed** in M99 (`542fa92`): `.gitignore` is honoured, with the matcher in `epsilon-core::ignore` so the tools and the TUI's `@`-file completion share one answer. | [`evidence/port-fidelity-audit.md`](evidence/port-fidelity-audit.md) D-9 | — |
| ~~G6~~ | **Closed** in `87c5628`: both the catalog and the tool filter it. | D-4 | — |
| ~~G7~~ | **Closed** in M101 (`df338b7`) for `SYSTEM.md` + `APPEND_SYSTEM.md` (both load; the project pair shadows the global one). **`.epsilon/prompts/` stays detected and unloaded**: pi's `prompts/*.md` are user-invoked slash templates, not system-prompt text, and the template feature is unported — the gate is kept so they cannot load untrusted when it lands. | D-7 | prompts templates still open |
| ~~G8~~ | **Closed** in M101 (`df338b7`): `AGENTS.override.md` is in the candidate list, ancestors load root-to-leaf, and `doctor` now reads the loader's own list instead of a second copy. | D-8 | — |
| ~~G9~~ | **Closed** in M102 (`989bd0a`): a length-stopped summary is refused and the caller falls back to context-free pruning. | D-5 | — |
| ~~G10~~ | **Wrong, and closed by diagnosis**: the TUI was repainting (the spinner advanced); the *tool call* was stalling six seconds on a PowerShell profile. See G15. | M98 `3e1d01b` | — |

| G15 | Every Windows `bash` call loaded the user's PowerShell profile — 6.2 s for a one-word `echo`, measured against 1.1 s with `-NoProfile` | M98 `3e1d01b`, probe `.tmp-console/bashtest` | **Closed.** This is what "the TUI freezes" was. |

### P2 — structure and teaching value

| # | Gap |
|---|---|
| ~~G11~~ | **Closed**: `app.rs` is **3,594 lines** from 6,700. The keyboard model, the event loop and all three test modules now live beside it (`app/keys.rs`, `app/event.rs`, `app/{tests,golden,composer_viewport_tests,tests_support}.rs`). |
| ~~G12~~ | **Closed** in `278d2ec`: every surface takes a passed-in `Theme` and the shim is deleted. |
| ~~G13~~ | **Closed in substance**: `app/golden.rs` drives the production `draw()` at 44x10 through 160x48 and asserts on the assembled frame, so a rendering regression is a failing test rather than a bug report. What is still not in CI is a *process*-level test (spawn the binary under a pseudo console); the two local harnesses for that remain the last gap. |
| G14 | `epsilon-ext` has 5 tests for a whole crate — the least covered surface relative to its size, and still open. |

### Known and deliberately not claimed

- A `.gitignore` negation re-includes a hidden directory in the matcher, but the
  walker still does not descend into one. `crates/epsilon-tools/src/util.rs` says
  so in the test rather than asserting behaviour that is not there.
- `epsilon-core::ignore` does not implement character classes (`[a-z]`), escaped
  metacharacters, or `core.excludesFile`. The module docs name all three.
- The header shows `0%%` in a **captured screen buffer** read through
  `ReadConsoleOutputCharacterW`; the rendered row is `0%`. The duplication is in
  the capture path, not the frame, and it is recorded here because it has not
  been proven to my own satisfaction on a real terminal — only explained.

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
