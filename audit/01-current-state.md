# 01 — Current state

What exists, how big it is, and how much of it I trust. Written on 2026-09-28 from the
code, from `git log`, and from measurements taken on this machine.

**Sources of truth, in order:** the code; a command run in this audit; `git log`; the
reference harnesses. Documentation is a *claim about* the code, never evidence for it.

**Evidence labels used throughout:**

- **Measured** — observed from a command named in this audit.
- **Read** — the control flow and its callers were inspected. Establishes what the code
  does, not what a user sees.
- **Inferred** — a conditional consequence. The trigger and the test that would settle it
  are both stated.
- **Unverified** — no adequate evidence yet. Do not treat as a capability.

## Shape of the repository

**Corrected at M233**, which added two crates and moved a third's contents. The line count
below is the 2026-09-28 measurement and is kept for comparison; the crate list is current.

14 crates. The dependency graph is a clean DAG —
`contracts` → {llm, session, ext, lsp} → `core` → {safety, mcp} → `tools` →
{`runtime`, tui, server} → {cli, desktop}. `epsilon-runtime` holds **composition**
(the `runtime.rs` helpers and `build_runtime`, both moved out of the `epsilon` binary at
M233); `epsilon-desktop` is the native IDE. The apparent `tools`↔`safety` cycle is a
dev-dependency only.

| Crate | Lines | What it owns |
|---|---|---|
| `epsilon-tui` | 28,119 | The terminal frontend: state, panels, keys, themes, render cache |
| `epsilon-llm` | 21,615 | Providers, wires, streaming, the model catalog, OAuth |
| `epsilon-core` | 19,233 | The agent loop, scheduling, compaction, the prompt |
| `epsilon-tools` | 10,613 | read/write/edit/bash/grep/glob/find/ls/web_fetch/todo/task, LSP, sandbox |
| `epsilon-session` | 9,402 | The durable JSONL session store, fork, branch |
| `epsilon-cli` | 7,166 | Argument parsing, mode dispatch, the binary |
| `epsilon-safety` | 3,530 | The permission engine, escalation, bubblewrap + Job Objects |
| `epsilon-contracts` | 2,759 | Types and traits with no internal dependencies |
| `epsilon-ext` | 1,939 | The extension host (subprocess tools, declared events) |
| `epsilon-lsp` | 1,560 | Language-server integration |
| `epsilon-server` | 1,229 | The axum server mode |
| `epsilon-mcp` | 672 | MCP stdio transport |
| `epsilon-runtime` | *new at M233* | Composition: workspace + configuration → a running agent |
| `epsilon-desktop` | *new at M233* | The native desktop IDE: window, layout, palette, frame |

**Read:** the crate boundaries are correct and ADR-014's "brick earns existence" rule is
holding. The problem is not the layout — it is that `crates/epsilon-core/src/lib.rs` is
3,415 lines / 155 KB while the same crate's other 16 modules are 1–46 KB.

## Maturity

This is a real codebase with real depth, not a prototype. It has 1,300+ test functions, a
five-gate verification battery, a real-console integration test, a documentation culture
that is genuinely better than average, and 178 milestones of recorded decisions.

The prose quality deserves a specific note, because it is unusual. From `docs/milestones/M106`:

> The header divided the session-total cache reads by the latest prompt size, so the
> figure inflated every turn and pinned at `cache 100%`. The test named for the right
> property passed because it modelled the gauge wrongly.

That is a project recording its own mistakes precisely, which is the behaviour you want
from a team and which most codebases never achieve. **The writing is the strongest asset
here and should be protected through any restructure.**

## What the verification actually covers

The battery is `pwsh -File scripts/gate.ps1 <label>` on Windows (CI uses
`scripts/gate.sh`): `cargo fmt --check`, `cargo clippy --all-targets -D warnings`,
`cargo test --workspace`, the demo, and a real-provider compile check.

One important gap, which the previous session partly fixed in uncommitted work: the
terminal integration test used to require `EPSILON_BIN` to be set by hand, so
`cargo test --workspace` skipped it — and a test the battery skips is a test that rots.
`the_screen_reports_what_the_harness_is_doing` sat broken from M151 to M178 for exactly
that reason. The pending change falls back to `CARGO_BIN_EXE_epsilon`, which Cargo sets
automatically. That change is correct and should be kept.

**But the deeper gap remains, and it is the most important finding in this audit:** the
faux provider emits an entire turn's events instantly
(`crates/epsilon-llm/src/faux.rs:145-148`, `pending.pop_front()` with no delay). No test
or probe has ever run the TUI while a turn is genuinely in flight over seconds. See
[04](04-tui.md).

## Documentation

- `docs/qa/` — 10 design-convention notes. The best writing in the repository.
- `docs/milestones/` — 171 dev logs. Valuable, and roughly 90% is bookkeeping rather than
  decision-record.
- `docs/design/epsilon-decisions.md` — 19 ADRs in one 95 KB file.
- `docs/history.md`, `CHANGELOG.md` (65 KB) — the same facts in a third and fourth place.
- `docs/user/` — 7 maintained guides. These are what a user should actually read.
- **~155 broken markdown links** across 43 files, mostly one unpropagated move from
  `audit/NN-*.md` to `audit/history/NN-*.md`. Two are load-bearing: the README's own
  "read this before trusting any claim" disclaimer points at a file that moved.
- `docs/README.md` opens with 18 orphaned table rows above its heading.

## Skills

`.agents/skills/` holds **8** skills. Two earn their place: `improve` (a read-only
codebase survey that produces a prioritised plan) and `ratatui-tui` (a general ratatui
reference; its `references/ai-harness.md` is genuinely useful).

The other six are `ponytail`, `ponytail-audit`, `ponytail-debt`, `ponytail-gain`,
`ponytail-help`, `ponytail-review`, and they do not belong to this project. `ponytail-gain`
renders a benchmark scoreboard citing `benchmarks/` and the README — **there is no
`benchmarks/` directory in this repository.** `ponytail-debt` greps for `ponytail:`
comments in a structured format; the 19 that exist are prose asides. All six ship
slash-command mode-switching UX that belongs to a chat plugin. They are the shape of
rigour without the substance, and the user's instinct to distrust them is correct.

## Housekeeping

- **173 `.log` files, ~11 MB, tracked in git** under `audit/evidence/`. Rust convention:
  build and CI output is disposable. Keep the `.md` / `.patch` / `.txt` narrative
  evidence; let the logs go.
- Four untracked `.tmp-*.txt` scratch files at the root — draft CHANGELOG rows from the
  M106–M108 work. `.gitignore` is correctly keeping them out of git; they should be
  deleted.
- `references/` is correctly excluded from git (0 tracked files).
- `audit/` was **incomplete**: its README promised four documents that did not exist.
  This pass writes them.

## What I did not verify

- No live provider call. Every claim about provider behaviour is a reading of wire code.
- No Linux or macOS execution. Windows only, and mostly `cmd` rather than PowerShell.
- No credential store was read and no network call was made.
- The full test suite was **not** run in this audit. Phase 0 of [07](07-plan.md) makes no
  product change, so the suite state is unchanged from HEAD, but that is an inference
  about an unrun suite and should be stated as such rather than assumed.
- Real session transcript sizes were not measured, so the per-frame cost projection in
  [04](04-tui.md) rests on a measured microbenchmark and an estimated session size.
