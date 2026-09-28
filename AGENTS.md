# AGENTS.md — Developing Epsilon

This file governs anyone (human or agent) writing code in THIS repository. It loads
automatically into coding-agent context. For the behavior contract of the Epsilon *product*,
see `crates/epsilon-core/assets/agent-contract.md`.

## Identity

- You are working on **Epsilon**, a Rust coding-agent harness. The full picture lives in
  `README.md`; the reasoning in `docs/qa/` and `docs/design/`. The docs index is
  `docs/README.md` — start there when you need to find something, and add new files to it.
  `audit/` holds the review (usability, defect ledger, port plan, verification record,
  reference comparison); `audit/09-revalidation.md` re-validates the earlier allegations
  against the code and names the verified baseline, so treat pre-revalidation claims as
  history until that file confirms them. Read `audit/04-roadmap.md` before starting new work so
  you build the thing that is actually next.
- Answer questions before acting; act before narrating when asked to implement.
- Repo-local skills live in `.agents/skills/` — read the one matching your task before working;
  each carries the proven procedure for that kind of work.
- Keep responses short and technical.

## Verification commands (run after every code change)

```sh
cargo fmt --all -- --check              # formatting gate — no diffs tolerated
cargo clippy --workspace --all-targets -- -D warnings  # enforce zero warnings
cargo test --workspace                  # report actual outcomes; the executed record of record is
                                         # audit/evidence/gate-<label>.log, cited by the milestone that
                                         # ran it and by docs/history.md. audit/05-verification-log.md
                                         # is the *2026-09-16 baseline* and stays history — its tail is
                                         # M86–M88, so do not read it as current.
cargo run -q -p epsilon-cli -- --demo  # demo must complete when loop/session/llm touched
cargo check -p epsilon-cli --features real   # real-provider path must compile (M13 lesson)
# CI (post-push): .github/workflows/ci.yml runs all gates on Windows, the bwrap
# Linux-confinement suite on ubuntu-latest, and an MSRV 1.88 check.
```

All five in one shot, with a disposable build log at `target/gate-<label>.log`
(copy relevant evidence outside `target/` before cleanup):

```sh
pwsh -File scripts/gate.ps1 <label>   # Windows — the one that works on this machine
bash scripts/gate.sh <label>          # POSIX and CI
```

> **Run the PowerShell one here.** `bash` on this box is WSL: a different `$HOME`,
> a different PATH, and no Windows `cargo`. `bash scripts/gate.sh` therefore exits
> 127 in zero seconds and writes a log ending in `GATE FAIL`, which reads like five
> failed gates instead of one missing PATH entry. Both scripts now name the missing
> binary instead of failing anonymously, but only `gate.ps1` can find it.

### The terminal integration test

**As of M179 it runs inside the battery, with no setup.** It used to need a built
binary named by hand, which meant `cargo test --workspace` skipped it — and a test the
battery skips is a test that rots. `the_screen_reports_what_the_harness_is_doing` sat
broken from M151 to M178 for exactly that reason: twenty-seven milestones of TUI work
and nothing ran it. The fix is Cargo's own:

```sh
cargo build -p epsilon-cli                          # the binary itself
EPSILON_BIN=target/debug/epsilon cargo test -p epsilon-cli --test terminal -- --test-threads=1
```

Run it directly when you want it on its own — the `--test-threads=1` matters, because
each test opens its own console and serialising them keeps a failure attributable to one
session. `EPSILON_BIN` is still honoured, because pointing the suite at a specific build
is occasionally the point, but it is no longer required: the test falls back to
`CARGO_BIN_EXE_epsilon`, which Cargo sets for an integration test whose package has a
binary of that name. It skips only when neither is available — a compiled test binary
run on its own — and it says so.

It is the only test that can fail because the binary never reached a terminal at all, and
it adds about 70 seconds to the battery. Two things learned building it, both of which
cost real time:

- **Answer the pseudo-console's `ESC [ 6 n`.** ConPTY asks where the cursor is and
  emits nothing else until it hears back, so a harness with no responder looks
  exactly like a program that renders nothing. The responder is in the test, with
  a comment explaining why.
- **Resize the pty and the emulator together.** Moving only the pty leaves the test
  reading a screen the program is not drawing into, which turns a correct resize
  into a spurious "row wider than the window" failure.

### The conhost probe (the other half of the terminal picture)

That test drives the binary through a **ConPTY**, where the console is emulated
inside the terminal application — so nothing it asserts can observe the console
*window* changing size. When the question is "what does the TUI do to a real CMD or
PowerShell window", use the probe:

```sh
python scripts/console-probe.py --late --size 100x30 target/debug/epsilon.exe "text"
python scripts/console-probe.py --late --maximize target/debug/epsilon.exe "text"
python scripts/console-probe.py --early target/debug/epsilon.exe "text"
```

It opens a genuine conhost window, attaches to it, records the buffer and window
geometry before and after the TUI starts, types real `KEY_EVENT` records into the
console input buffer, and reads the screen back as JSON. Launch the TUI *from the
shell in that console* — spawning it as a child of the probe hands it the probe's
stdout, a pipe, and it renders the frame into the pipe instead of the screen. Results
so far: [`audit/evidence/console-probe-2026-09-21.md`](audit/evidence/console-probe-2026-09-21.md).

> `real` is a **default** feature since M63 (the offline-only binary was a trap), so the
> `--features real` flag above is now redundant but harmless. Build offline-only with
> `--no-default-features`.

Fix or honestly report failures; never weaken a gate to make it pass.

## Verification rounds on this machine (hard-won rules)

One build owner at a time: a gate round, a delegated build and a `cargo install` must never
share the machine. Run long work with `CARGO_BUILD_JOBS=1` and `CARGO_TARGET_DIR` on an external
path — every gate failure at parallel jobs on this box has been `0xc0000142` compiler-startup
contention, and single-job builds with an external target have passed every round since.

- **Every battery attempt gets its own label.** `scripts/gate.sh` truncates
  `target/gate-<label>.log` on start, so re-using a label destroys the failed attempt's evidence.
- **Evidence must cover the shipped bytes.** Freeze a candidate (HEAD archive + exactly the
  changed files, byte-verified by hash), run the gates against that, and stop an in-flight
  battery when a fix round changes the code — a log for bytes that are no longer in the commit
  proves nothing.
- **When a slice cannot have a pre-fix RED** (its API did not exist before it), run a *negative
  control* instead: revert one fix inside a disposable copy of the candidate and show that the
  test naming that property fails for the reason it names. Restore, verify byte-identity, then
  relaunch. Record the probe output as evidence.
- **After restoring a file, touch it — or confirm `Compiling` appears.** `Copy-Item` (and every
  copy that preserves timestamps) restores the *backup's* mtime, so cargo can decide the source
  is unchanged, skip the rebuild, and silently run the **control** binary. The control then
  appears to pass (or a real fix appears to fail) for a reason that has nothing to do with the
  code. Cost two rounds here — 2026-09-21 and 2026-09-22 — before being written down.
- **Shared files carrying another work item's uncommitted changes are staged by line, not by
  file:** `git hash-object -w <your built version>` then `git update-index --cacheinfo`, and diff
  the staged blob against HEAD to prove only your lines are in the commit.
- **Stopping a battery:** find the MSYS wrapper's WINPID with `ps -W`, then
  `MSYS_NO_PATHCONV=1 taskkill /F /T /PID <WINPID>`. The process tool's kill can time out and
  leave cargo running, and `kill -0` is unreliable on MSYS — verify by the gate log no longer
  growing.
- **Avira can pin one test-artifact path** (`LNK1104` or `Access is denied` on a file that does
  not exist, surviving `rm`). Never change security configuration from an agent session; rebuild
  the test profile under fresh metadata instead (`CARGO_PROFILE_TEST_DEBUG=1` gives every test
  executable a new name) and disclose the knob in the verification log.
- Milestone docs carry a `## Verification` section that reads **PENDING** until a battery has
  actually run; fill it from the log, reconcile per-binary test-count deltas, and keep the
  limits honest about what was never executed.
- **Milestone docs also carry a one-line "what nothing exercises" section.** An independent review
  of the TUI slices found five blockers, and **two of them were exactly this**: a field that every
  card advertised but no test asserted (`0ms` durations), and a code path no test had ever drawn (the
  split diff at ≥120 columns — which turned out to hide a real bug where the two halves rendered on
  top of each other). Both would have been visible from a single line saying "no test exercises X" or
  "nothing asserts Y", written by the author who already knew. `Limits` records what a slice does not
  *do*; this section records what it does not *prove*, and the two are different lists.
- **A claim about what a user sees is verified by measuring, not by reading.** The same review said a
  footer badge vanished at 60 columns; the author probed the real frame and found the badge whole with
  the protection disarmed, and the actual defect one band lower (a *partial* badge at widths 26–31,
  which is worse than a missing one because a wrong number is not noticed). Where a reader of the code
  and a measurement of the screen disagree, **the measurement wins** — and a commit message that
  repeats the unmeasured version puts a false claim in the permanent record.

## Disk hygiene (this machine has a small, chronically-full C: drive)

Rust build artifacts are enormous — this repo's `target/` alone grows past 35 GB and
has pushed C: to 96% full. **Clear the cache after every test/build round**, not just
at the end of a session (owner's standing rule, 2026-09-14):

```sh
cargo install --path crates/epsilon-cli --features real --force  # ONLY when CLI/TUI behavior changed: refresh the global exe FIRST
cargo clean && rm -rf target                                     # then wipe build artifacts (~35-40 GB here)
df -h /c                                                         # confirm free space recovered
```

Budget a cold round at **~30–45 min with one job** (the C dependency build dominates; a release
`cargo install` measured 28 min with deps warm). Recent rounds build into an external
`CARGO_TARGET_DIR` rather than an in-repo `target/` — the wipe below then applies to that
directory. Run the full gate battery once per round, then wipe. Never clean mid-round, and never
leave a `target/` behind after reporting results.

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
  change; do not mix it into a documentation-only restructure. The one recorded exception:
  stateless, dependency-free *defaults on seam traits* (`ToolHooks`, `TurnHooks`) under
  ADR-014's M89 amendment — pure functions of their arguments, overridable contract, no
  state, no dependency. Everything else still needs its own narrowly defined amendment.
- New crates require an ADR first (see ADR-014's "brick earns existence" rule).
- genai/rmcp/etc. types never leave their adapter module.

## Dependency policy

- Direct deps pinned EXACT (`=x.y.z`) in `[workspace.dependencies]` as introduced.
- Verify versions and feature names against the crate's own manifest/source before pinning —
  cargo's error messages list real candidates; use them rather than guessing twice.
- No new dependency without a stated reason tied to an ADR or milestone need.

## Protocol surfaces (added after D26)

A wire name is part of the contract, and a test that never sends a frame cannot check one.

- **A protocol test must go through the wire.** Deserializing an enum proves the type
  parses. Only dispatching a real frame and reading the reply proves the protocol accepts
  what a client will send. `ops_deserialize_with_snake_tags` compared serde's output against
  the variant list, so it agreed with the code by construction — which is exactly why a
  mis-spelled op name shipped and a pi-shaped client was refused the name pi documents.
- **`#[serde(other)] Unknown` swallows `#[serde(alias = ...)]`.** A catch-all variant
  absorbs an unrecognised name before serde reaches the alias check, so the alias compiles,
  looks correct, and silently does nothing. For a name that must work, use an explicit
  `#[serde(rename = "...")]` and keep the old spelling as the alias.
- **When an op's name is taken from a reference implementation, quote the reference's line.**
  `rpc-types.ts:30`, not a memory of what it said.

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
- Non-trivial slices get an **independent read-only review** (delegate it; the reviewer must not
  build or edit). Record its findings and dispositions in `audit/evidence/<slice>-review.md`,
  fix blockers before the commit, and carry every residual into the milestone's `Limits` — a
  review whose findings vanish is a review that did not happen.
- State in the milestone whether the global `epsilon.exe` needed a reinstall and why: a slice
  nothing in-tree can reach needs none, a CLI-visible one does (the owner tests the installed
  binary).
- Update [`docs/history.md`](docs/history.md) milestone timeline when a milestone lands
  (the root README stays reader-first: quick start and product story only).

## Git discipline

- Stage explicit paths only; never `-A` blind into unrelated worktrees.
- Commit message convention: `feat(mN): …` / `fix(mN): …` / `docs: …`.
- A commit's message names its evidence: the battery label and the `audit/evidence/…` files the
  claim rests on, so any assertion can be traced to a log. The message is the audit trail, not
  a summary of effort.
- Only commit when the task calls for it; milestones are natural commit points.
- Preserve unrelated dirty work. A documentation commit may incorporate pre-existing
  edits in the named document only after reviewing them; disclose that scope. Do not
  manufacture historical milestone commits or stage source to make status look clean.
- Honor the owner's current authorization. The autonomous audit/restructure/port task
  explicitly authorizes successive implementation phases and small commits without
  repeated approval; old audit checkpoint wording is historical. A new unrelated
  task does not inherit permission to push, publish or use paid services.

## Windows notes (learned the hard way — see docs/milestones/M4)

- Append-only handles cannot `set_len` (FILE_APPEND_DATA ≠ FILE_WRITE_DATA); truncation needs a
  read+write handle opened without append.
- A write-only handle cannot be read back through later. Open purpose-built handles instead of
  contorting one handle across conflicting access needs.
- PowerShell here-string edits miss CRLF/LF variants; prefer proper file-edit tooling over bulk
  string surgery.
- **`bash` on this machine is WSL, and it has no Windows `cargo`** (different `$HOME`,
  different PATH; hit 2026-09-16 as "cargo: command not found", explained 2026-09-21 when the
  battery exited 127 in zero seconds and the log still said `GATE FAIL`). Run
  `pwsh -File scripts/gate.ps1 <label>` here; `scripts/gate.sh` is for POSIX and CI and now names
  the missing binary instead of failing anonymously.
- Avira Security on this machine has flagged freshly linked test executables with
  `HEUR/APC.Misc` and blocked the write (LNK1104), binding the block to the exact
  output path (hit 2026-09-17, see audit/05-verification-log.md). Diagnose with
  name probes in the same directory and `CARGO_TARGET_DIR` outside the repo;
  clearing the detection or adding an exclusion is an owner decision requiring
  administrator rights — never change security configuration from an agent session.
  The current recipe (fresh test-profile metadata) is under "Verification rounds on this
  machine".

## Agent memory (`agents/memory/`)

Long agent conversations get compressed; this folder is the deliberate,
intelligent form of that compression — hand-curated working state that survives
context loss, checked into the codebase where the work happens.

- **Read before working**: load `agents/memory/README.md` and any file matching
  your task; trust code, gates, and `git log` over these notes when they disagree.
- **Update in the same turn** a work item's status, decisions, evidence paths,
  or next steps change — never end a turn on stale memory.
- **One file per active work item**, topic-slug named, under ~80 lines.
- **Keep the index in step**: every note appears in `agents/memory/README.md` with a one-line
  status, and retiring a note deletes its row in the same change — an unlisted note is lost
  state.
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

Folder layout (maintained guides under `docs/`, assessment and recovery state separate):

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
scripts/gate.sh    the full verification battery in one command (POSIX and CI)
scripts/gate.ps1   the same five gates, native Windows (the one that works here)
scripts/console-probe.py  drives the built TUI in a real conhost window and measures it
crates/            the 12 workspace crates
references/        the four reference harnesses (not committed)
agents/            agent working memory (open work state; see "Agent memory")
.agents/skills/    repo-local skills (procedures agents and contributors follow)
```

## Conflict resolution

If this file conflicts with direct user instructions, safety-relevant rules still win — explain
the conflict instead of complying silently. Everything else yields to explicit user direction.
