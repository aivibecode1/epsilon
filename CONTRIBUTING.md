# Contributing to Epsilon

Thank you for looking. This file tells you how to get oriented and how to check your work.
It deliberately does **not** duplicate the rules — those live in one place.

## Before you change anything

Read **[`AGENTS.md`](AGENTS.md)**. It is the contract for anyone editing this repository,
human or agent, and it is short on purpose. It covers:

- what the project is, and the eight architecture invariants that do not change quietly;
- the build, test, lint and run commands, and the machine-specific traps on Windows;
- the security and data-loss rules;
- the accessibility standard the desktop IDE is held to;
- how to be a good subagent, if you are one.

**`AGENTS.md` is locked.** It is maintainer-owned, its SHA-256 is in `AGENTS.md.sha256`, and
`scripts/check-agents-lock.py` fails the build when the bytes move. If you think it is wrong,
write a proposal in [`docs/AGENTS-PROPOSALS.md`](docs/AGENTS-PROPOSALS.md) — do not edit it.

## Get oriented

- New to the project? Start at **[`docs/START-HERE.md`](docs/START-HERE.md)**. There is an
  Arabic version at [`docs/START-HERE.ar.md`](docs/START-HERE.ar.md).
- Want to know what is true right now? **[`docs/STATUS.md`](docs/STATUS.md)**.
- Curious where an idea came from? **[`CREDITS.md`](CREDITS.md)** and
  [`docs/references/`](docs/references/README.md).

## Build and run it

You need Rust **1.88 or newer** for everything.

```sh
cargo build --workspace
cargo run -q -p epsilon-cli -- --demo      # offline demo: no API key, no network
cargo run -q -p epsilon-cli                 # the terminal UI
```

To use a real model you need credentials for a provider — see
[`docs/user/install.md`](docs/user/install.md) and [`docs/user/auth.md`](docs/user/auth.md).
Never put a key on a command line; it would land in your shell history.

## Check your work

Run the whole battery. It is one command and it is the same command CI runs.

```sh
pwsh -File scripts/gate.ps1 mylabel    # Windows
bash scripts/gate.sh mylabel           # macOS, Linux
```

It runs, in order: documentation links, lint-table consistency, line endings, control bytes,
**the `AGENTS.md` lock**, formatting, clippy, the test suite, the offline demo, and a check
that the real-provider path still compiles. The log lands at `target/gate-<label>.log` and
`target/` is disposable.

Two rules that save everyone time:

- **Give the battery a generous timeout.** The test leg alone runs for minutes; a short timeout
  kills it part-way and the log records no verdict.
- **Never re-use a label, and never edit source while a battery is running.** The log then
  describes a tree that no longer exists, which proves nothing.

### Running just the test suite

```sh
cargo test --workspace
cargo test -p epsilon-tui                  # one crate
```

Tests run offline against a scripted provider by default. Nothing in the default suite calls a
paid API. Real-provider tests are marked `#[ignore]` and stay that way.

## Writing code

- **Smallest correct change.** Before adding a dependency or an abstraction, check whether the
  codebase, the standard library or the platform already does the job. See the Ponytail
  discipline in `AGENTS.md`.
- **Rust rules**: `unsafe_code = "forbid"`; every public item is documented; direct dependencies
  are pinned exactly in `[workspace.dependencies]`; a new crate needs an ADR first.
- **Comments explain why**, not what.
- **A behaviour-level change needs an ADR entry** in
  [`docs/design/epsilon-decisions.md`](docs/design/epsilon-decisions.md).

### Two habits this project is strict about

**Anything a user can see must be rendered and looked at.** Asserting that a piece of state
changed is not the same as showing that a person would see it. Features have shipped here that
passed every test and were never drawn.

**A definition plus a test is not a feature.** When you add a primitive, grep for its callers
and check that at least one is production code.

## Writing documentation

- Document intent, structure and decisions — not what the code already says.
- Plain and concrete. Short sentences. Define a term at first use.
- **Every command in the docs must have been run.** Every link must resolve.
- Add new files to [`docs/README.md`](docs/README.md), the map.

## If you are an agent working here

Same rules. Additionally:

- **Every subagent brief starts: "read and obey `AGENTS.md`."**
- Give each subagent owned files. Never two writers on the same file.
- Whoever integrates re-runs the checks. "Done" on a subagent's word is not verification.

## Licence

Epsilon is MIT — see [`LICENSE`](LICENSE). Contributions are accepted under the same licence.

Because the project credits six reference harnesses and two agent skills, a note on what "no
third-party code was copied" means in practice: if you adapt code from another project, check
its licence, keep the original copyright and licence notices, mark the adapted file with a
header naming the source, and add it to the credits. When in doubt, reimplement from the idea
and say where the idea came from.
