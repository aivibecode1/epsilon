# AGENTS.md — Developing Epsilon

This file governs anyone (human or agent) writing code in THIS repository. It loads
automatically into coding-agent context. For the behavior contract of the Epsilon *product*,
see `crates/epsilon-core/assets/agent-contract.md`.

## Identity

- You are working on **Epsilon**, a Rust coding-agent harness. The full picture lives in
  `README.md`; the reasoning in `docs/qa/` and `docs/design/`. The docs index is
  `docs/README.md` — start there when you need to find something, and add new files to it.
  `audit/` holds the honest review (usability, defect ledger, port plan, verification
  record, reference comparison); read `audit/04-roadmap.md` before starting new work so
  you build the thing that is actually next.
- Answer questions before acting; act before narrating when asked to implement.
- Keep responses short and technical.

## Verification commands (run after every code change)

```sh
cargo fmt --all -- --check              # formatting gate — no diffs tolerated
cargo clippy --workspace --all-targets -- -D warnings  # enforce zero warnings
cargo test --workspace                  # report actual outcomes; latest baseline: audit/05-verification-log.md
cargo run -q -p epsilon-cli -- --demo  # demo must complete when loop/session/llm touched
cargo check -p epsilon-cli --features real   # real-provider path must compile (M13 lesson)
# CI (post-push): .github/workflows/ci.yml runs all gates on Windows, the bwrap
# Linux-confinement suite on ubuntu-latest, and an MSRV 1.88 check.
```

All five in one shot, with a disposable build log at `target/gate-<label>.log`
(copy relevant evidence outside `target/` before cleanup):

```sh
bash scripts/gate.sh post            # any label; exits non-zero if a gate fails
```

> `real` is a **default** feature since M63 (the offline-only binary was a trap), so the
> `--features real` flag above is now redundant but harmless. Build offline-only with
> `--no-default-features`.

Fix or honestly report failures; never weaken a gate to make it pass.

## Disk hygiene (this machine has a small, chronically-full C: drive)

Rust build artifacts are enormous — this repo's `target/` alone grows past 35 GB and
has pushed C: to 96% full. **Clear the cache after every test/build round**, not just
at the end of a session (owner's standing rule, 2026-09-14):

```sh
cargo install --path crates/epsilon-cli --features real --force  # ONLY when CLI/TUI behavior changed: refresh the global exe FIRST
cargo clean && rm -rf target                                     # then wipe build artifacts (~35-40 GB here)
df -h /c                                                         # confirm free space recovered
```

Budget for a cold rebuild (~10-15 min) before the next verification round: run the
full gate battery once per round, then wipe. Never clean mid-round, and never leave a
`target/` behind after reporting results.

- When CLI/TUI behavior changed and gates pass, reinstall the global `epsilon.exe`
  before cleaning: the user tests the installed binary. No reinstall for docs-only work.
- If gates fail, preserve the failure evidence, do not install the failing baseline,
  then clean after the verification round. A failed round does not justify retaining
  a large cache indefinitely. Never clean while a build/test process is still running.
- Do not indiscriminately purge shared dependency caches or user runtime data.
  Clean this repository's disposable artifacts and confirm recovered disk space.

## Architecture law

Cargo enforces declared dependencies and rejects cycles; it does not enforce our
allowed-edge policy. The actual direct dependency graph is documented in
`docs/design/epsilon-architecture.md`. CLI composes sibling crates; core depends on
contracts, llm and session; LSP communicates through contracts. No library may depend
on CLI. New edges require review rather than relying on an illustrative layer diagram.

- `epsilon-contracts` is intended for types and traits, with zero internal dependencies.
  Existing validation and stateful stream folding violate the literal types-only rule
  (`audit/07-inconsistencies.md`). Do not add behavior there or silently endorse that
  drift. Extraction or a narrowly defined policy amendment needs its own reviewed
  change; do not mix it into a documentation-only restructure.
- New crates require an ADR first (see ADR-014's "brick earns existence" rule).
- genai/rmcp/etc. types never leave their adapter module.

## Dependency policy

- Direct deps pinned EXACT (`=x.y.z`) in `[workspace.dependencies]` as introduced.
- Verify versions and feature names against the crate's own manifest/source before pinning —
  cargo's error messages list real candidates; use them rather than guessing twice.
- No new dependency without a stated reason tied to an ADR or milestone need.

## Code quality

- `unsafe_code = "forbid"` workspace-wide; exceptions need a per-crate override + justification.
- Rustdoc on every public item (`missing_docs` warn is on). Write docs you would want to inherit.
- Prefer compiler-guided fixes over cleverness; the M4/M5 logs show the borrow checker walking
  us to better designs — let it.
- Match the existing porting discipline: semantics cite their pi/DSH/Hermes/opencode origin in
  doc comments where non-obvious.

## Testing philosophy

- Faux provider first: full-loop scenarios run offline against scripted steps. Never add tests
  that hit paid APIs by default (real-provider checks go behind env-var-gated `#[ignore]`).
- Deterministic time: use `SlowProvider` for timing-sensitive tests, never bare sleeps-and-hope.
- Each brick owns integration tests covering its failure paths, not just its happy path.
- When a test reveals a design bug, fix the design AND record the story in that milestone's
  dev log — those stories are the project's teaching material.

## Milestone workflow

- One milestone = one commit = one dev log at `docs/milestones/M<N>-<name>.md`
  (what / key decisions / gotchas / verification output).
- Behavior-level changes require an ADR entry or amendment in `docs/design/epsilon-decisions.md`.
- Update [`docs/history.md`](docs/history.md) milestone timeline when a milestone lands
  (the root README stays reader-first: quick start and product story only).

## Git discipline

- Stage explicit paths only; never `-A` blind into unrelated worktrees.
- Commit message convention: `feat(mN): …` / `fix(mN): …` / `docs: …`.
- Only commit when the task calls for it; milestones are natural commit points.
- Preserve unrelated dirty work. A documentation commit may incorporate pre-existing
  edits in the named document only after reviewing them; disclose that scope. Do not
  manufacture historical milestone commits or stage source to make status look clean.
- Explicit phase-review gates require user approval. Automated `/goal` continuation
  is not new approval; once approval is given, do not ask for the same gate again.

## Windows notes (learned the hard way — see docs/milestones/M4)

- Append-only handles cannot `set_len` (FILE_APPEND_DATA ≠ FILE_WRITE_DATA); truncation needs a
  read+write handle opened without append.
- A write-only handle cannot be read back through later. Open purpose-built handles instead of
  contorting one handle across conflicting access needs.
- PowerShell here-string edits miss CRLF/LF variants; prefer proper file-edit tooling over bulk
  string surgery.
- Bash-launched tools may not see `cargo` even though PowerShell does (PATH differs per shell;
  hit 2026-09-16 during the audit). If `scripts/gate.sh` fails with `cargo: command not found`,
  point the script at the absolute path (`~/.cargo/bin/cargo.exe`) or run the five gates directly.
- Avira Security on this machine has flagged freshly linked test executables with
  `HEUR/APC.Misc` and blocked the write (LNK1104), binding the block to the exact
  output path (hit 2026-09-17, see audit/05-verification-log.md). Diagnose with
  name probes in the same directory and `CARGO_TARGET_DIR` outside the repo;
  clearing the detection or adding an exclusion is an owner decision requiring
  administrator rights — never change security configuration from an agent session.

## Agent memory (`agents/memory/`)

Long agent conversations get compressed; this folder is the deliberate,
intelligent form of that compression — hand-curated working state that survives
context loss, checked into the codebase where the work happens.

- **Read before working**: load `agents/memory/README.md` and any file matching
  your task; trust code, gates, and `git log` over these notes when they disagree.
- **Update in the same turn** a work item's status, decisions, evidence paths,
  or next steps change — never end a turn on stale memory.
- **One file per active work item**, topic-slug named, under ~80 lines.
- **Checkpoint before compaction**: record verified facts, assumptions, decisions,
  evidence paths, running process IDs, dirty-work boundaries and the next safe action.
- **Retire after closeout**: remove the working note only after documentation,
  verification, required installation and cleanup finish. Committing code alone
  is not closeout. No secrets or transcript dumps.

## Documentation while building

Documentation is part of "done" — if a change lands without its docs, the task is not finished:

- **Milestone dev log** (`docs/milestones/M<N>-*.md`) updated/created with: what, key
  decisions, test-caught bugs, verification output.
- **History timeline**: milestone row + suite totals move the same commit as the
  work ([`docs/history.md`](docs/history.md); README carries no milestone tables).
- **Design conversations become qa notes**: when a question produces a convention decision
  (naming, file placement, counts, policies), capture it as `docs/qa/<NN>-<topic>.md` —
  decisions that live only in chat history are lost to the next contributor and the next agent.
- **ADR amendments** for behavior-level changes (`docs/design/epsilon-decisions.md`).
- **New documentation files** must be added to [`docs/README.md`](docs/README.md) (the docs map)
  so nothing is orphaned; `docs/qa/` numbering stays sequential.
- Rationale and precedent: see `docs/qa/05-documentation-organization.md`.

Folder layout (reorganized 2026-09-15 so everything a reader needs is under `docs/`):

```
README.md          entry point (product story, install, contents)
AGENTS.md          this file — rules for writing code here
CHANGELOG.md       user-visible changes, newest first
audit/             review + defect ledger + port plan + roadmap
docs/
  README.md        the map of everything below
  design/          architecture, ADR log, skills design
  history.md       milestone timeline
  milestones/      one dev log per milestone
  qa/              design conventions and primers
  plans/           implementation plans in execution order
  research/        reference-harness studies and gap analyses
  glossary.md      vocabulary
  handoff/         task handoffs
  user/            maintained product guides
scripts/gate.sh    the full verification battery in one command
crates/            the 12 workspace crates
references/        the four reference harnesses (not committed)
agents/            agent working memory (open work state) + local agent skills
```

## Conflict resolution

If this file conflicts with direct user instructions, safety-relevant rules still win — explain
the conflict instead of complying silently. Everything else yields to explicit user direction.
