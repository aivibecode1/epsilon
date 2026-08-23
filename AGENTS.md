# AGENTS.md — Developing Epsilon

This file governs anyone (human or agent) writing code in THIS repository. It loads
automatically into coding-agent context. For the behavior contract of the Epsilon *product*,
see `crates/epsilon-core/assets/agent-contract.md`.

## Identity

- You are working on **Epsilon**, a Rust coding-agent harness. The full picture lives in
  `README.md`; the reasoning in `docs/qa/` and `design/`.
- Answer questions before acting; act before narrating when asked to implement.
- Keep responses short and technical.

## Verification commands (run after every code change)

```sh
cargo fmt --all -- --check              # formatting gate — no diffs tolerated
cargo clippy --workspace --all-targets  # ZERO warnings tolerated (workspace lints deny)
cargo test --workspace                  # full suite must stay green (29+ tests)
cargo run -q -p epsilon-cli             # demo must complete when loop/session/llm touched
```

Fix or honestly report failures; never weaken a gate to make it pass.

## Architecture law

Dependency direction is compile-enforced and may not be violated:

```
contracts <- llm <- core <- {tools, safety, ext, mcp} <- {tui, server} <- cli
contracts <- session <- core
```

- `epsilon-contracts` holds TYPES AND TRAITS ONLY. Zero business logic, zero internal deps.
  If you find logic creeping in, move it out — the vocabulary crate is the stud standard of
  the whole build methodology (`docs/qa/02-lego-brick-methodology.md`).
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
- Behavior-level changes require an ADR entry or amendment in `design/epsilon-decisions.md`.
- Update `README.md` status table when a milestone lands.

## Git discipline

- Stage explicit paths only; never `-A` blind into unrelated worktrees.
- Commit message convention: `feat(mN): …` / `fix(mN): …` / `docs: …`.
- Only commit when the task calls for it; milestones are natural commit points.

## Windows notes (learned the hard way — see docs/milestones/M4)

- Append-only handles cannot `set_len` (FILE_APPEND_DATA ≠ FILE_WRITE_DATA); truncation needs a
  read+write handle opened without append.
- A write-only handle cannot be read back through later. Open purpose-built handles instead of
  contorting one handle across conflicting access needs.
- PowerShell here-string edits miss CRLF/LF variants; prefer proper file-edit tooling over bulk
  string surgery.

## Documentation while building

Documentation is part of "done" — if a change lands without its docs, the task is not finished:

- **Milestone dev log** (`docs/milestones/M<N>-*.md`) updated/created with: what, key
  decisions, test-caught bugs, verification output.
- **README status**: milestone table row + suite totals move the same commit as the work.
- **Design conversations become qa notes**: when a question produces a convention decision
  (naming, file placement, counts, policies), capture it as `docs/qa/<NN>-<topic>.md` —
  decisions that live only in chat history are lost to the next contributor and the next agent.
- **ADR amendments** for behavior-level changes (`design/epsilon-decisions.md`).
- **New documentation files** must be added to the root README's reading guide so nothing is
  orphaned; `docs/qa/` numbering stays sequential.
- Rationale and precedent: see `docs/qa/05-documentation-organization.md`.

## Conflict resolution

If this file conflicts with direct user instructions, safety-relevant rules still win — explain
the conflict instead of complying silently. Everything else yields to explicit user direction.
