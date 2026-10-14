# Epsilon

<p align="center">
  <img src="assets/Logo.png" alt="Epsilon logo" width="720">
</p>

Epsilon is a coding-agent **harness** — the program that runs an agent: it talks to a model,
offers it tools, decides what it is allowed to do, and keeps the conversation — written in Rust,
with a full-screen terminal UI (an opencode-style tree + editor + agent IDE), a headless mode, an HTTP/SSE server and a stdio RPC
frontend over one agent core. It began as a port of
[pi-agent](https://github.com/earendil-works/pi) and has since taken execution, permission and
recovery ideas from DeepSeek Harness, Hermes Agent and opencode.

The port is **not complete**, and this file says where it stops rather than
leaving that to be discovered.

## Contents

- [What it does](#what-it-does)
- [Install](#install)
- [Using it](#using-it)
- [Current status](#current-status)
- [How it is built](#how-it-is-built)
- [Working on it](#working-on-it)
- [Documentation](#documentation)
- [References and credits](#references-and-credits)

## What it does

You point it at a **workspace** — the folder of code you want it to work in, which is also the
boundary it may not read or write outside of — and describe a task. It reads files, edits them,
runs commands, searches, delegates to subagents, and keeps a session you can resume.
Every side-effecting action passes a permission gate first, and on Windows the
commands it runs are confined by a Job Object, on Linux by bubblewrap where it is
installed.

It is a single binary with no required Node or Python runtime.

## Install

Rust 1.88 or newer for everything (the removed desktop IDE was the one crate that needed 1.95;
see ADR-039 in [the decisions log](docs/design/epsilon-decisions.md)).

```sh
cargo install --path crates/epsilon-cli --force
epsilon --help
```

From a clone, without installing:

```sh
cargo run -q -p epsilon-cli -- --demo     # offline, scripted, no provider needed
cargo run -q -p epsilon-cli               # the terminal UI
```

Platform notes, shell requirements and what each confinement backend does and
does not do are in [Install](docs/user/install.md) and
[Permissions](docs/user/permissions.md).

## Using it

| Command | What it does |
|---|---|
| `epsilon` | Interactive terminal UI |
| `epsilon -p "summarize this repo"` | One prompt, headless; approvals default to denial |
| `epsilon doctor` | Report configuration and available capabilities without running a model |
| `epsilon -c` / `epsilon --resume <file>` | Continue a session |
| `epsilon --serve --port 8787` | HTTP/SSE frontend |
| `epsilon --rpc` | NDJSON over stdio; accepts pi's `type` frames and replies with pi's envelope |
| `epsilon --plan` | Deny native write/edit/bash/shell, keeping project rules |

In the TUI, Enter sends, Esc cancels, `/help` lists commands, and the header shows
the model, the remaining context window, the cache hit rate and the running cost.

`/goal <objective>` parks one long-running objective in the session and keeps working on
it between turns; the footer shows its phase and rounds. See
[Goals](docs/user/goals.md).

For real providers, after configuring your own credentials:

```sh
epsilon auth login anthropic
epsilon --list-models
epsilon --model <model-id>
```

Credentials go through the local login prompt, never a command-line argument.
OAuth needs provider-specific setup: see [Auth](docs/user/auth.md).

## Current status

**An incomplete port, not production-ready.** What is true right now — what was
verified, by which command, and what each number does and does not prove — is
[docs/STATUS.md](docs/STATUS.md). The 2026-09-16 assessment this file used to point
at is superseded and kept only as history.

What is solid: the agent loop, the tool set, session storage, compaction, the
permission gate, and a test suite that runs offline against scripted providers —
including a terminal integration test that drives the real binary through a real
pseudo-console.

What is not: resolved filesystem containment for the native file tools is still
open; checkpoint restore has known gaps, so it is not yet a safe user-facing undo;
some pi features are unported; and nothing here has been certified on Linux or
macOS beyond CI compiling it.

Use backed-up or disposable workspaces until that list is shorter.

## How it is built

Fourteen crates under `crates/`. Each has one job, and the edges between them are
documented rather than assumed — Cargo rejects cycles but will happily accept an
edge nobody meant to add.

| Crate | Responsibility |
|---|---|
| `epsilon-contracts` | Shared types and traits, and the command registry every mode reads |
| `epsilon-llm` | Provider protocols, the offline provider, routing, catalog, auth, retry |
| `epsilon-session` | Session storage, indexing, branch projection, repair, leases |
| `epsilon-core` | Agent loop, scheduling, context assembly, compaction, task lifecycle |
| `epsilon-tools` | Native tools, the shell adapter, read observations, checkpoints |
| `epsilon-safety` | Permission rules and approvals, environment filtering, sandbox backends |
| `epsilon-lsp` | Optional language-server client and diagnostics |
| `epsilon-ext` | The process-extension protocol and its host |
| `epsilon-mcp` | MCP client and tool adapter |
| `epsilon-runtime` | Composition: the one place a workspace and a configuration become a running agent |
| `epsilon-tui` | Terminal frontend |
| `epsilon-server` | HTTP/SSE frontend |
| `epsilon-cli` | The binary: argument parsing and mode dispatch |

The [architecture guide](docs/design/epsilon-architecture.md) maps the real edges
and the execution paths. Decisions and their reasoning are recorded as ADRs in
[docs/design/epsilon-decisions.md](docs/design/epsilon-decisions.md).

## Working on it

Read [AGENTS.md](AGENTS.md) first — it is the contract for anyone editing this
repository, human or agent.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p epsilon-cli -- --demo
cargo check -p epsilon-cli --features real
```

`bash scripts/gate.sh <label>` (or `pwsh -File scripts/gate.ps1 <label>` on
Windows) runs all ten gates — five repository checks, `fmt`, `clippy`, the test
suite, the offline demo and the real-feature check — and writes
`target/gate-<label>.log`. Keep any evidence you want to cite outside `target/`,
which is disposable, and give every attempt its own label.

Two habits that save time here:

- **Build with one job.** Long builds on a busy machine have failed with compiler
  startup errors under parallelism; `CARGO_BUILD_JOBS=1` and a target directory
  outside the repository have passed every round since.
- **The TUI has two test layers.** In-process golden frames check what a screen
  looks like; `crates/epsilon-cli/tests/terminal.rs` spawns the real binary in a
  pseudo-console and checks that it gets there at all. Run the second with
  `EPSILON_BIN=target/debug/epsilon cargo test -p epsilon-cli --test terminal` —
  the variable is optional, and it is the only test that can fail because
  the program never reached a terminal.

Test counts, gate results and their limits are **not** asserted here. Where to find them, and how
far each source reaches, measured 2026-10-04:

| Source | What it carries | Reaches |
|---|---|---|
| [`docs/history.md`](docs/history.md) | one row per milestone: what shipped, acceptance criteria, and a `GATE PASS`/`FAIL` line on 46 of the 280 rows | **M0–M276** |
| [`docs/README.md`](docs/README.md) | an annotated pointer list of the milestones a reader is most likely to want | **M0–M276** |
| [`docs/STATUS.md`](docs/STATUS.md) | the living record: the current battery, its numbers, the knobs it ran under, and what is still broken | **current** |

The first two stop at M276 because **M277 onward have no dev log at all** — `docs/history.md`
records that decision rather than inventing rows. For anything after that, `STATUS.md` is the
only one of the three that is current, which is why it is named last and why the reading guide
points at it.

## Documentation

- **[Start here](docs/START-HERE.md)** — the reading guide: a ten-minute tour, a reading order,
  a folder map, a glossary and one diagram of how the pieces connect.
- **[Status](docs/STATUS.md)** — what is true right now: progress, decisions in flight, what was
  cut, what is broken.
- [Documentation map](docs/README.md) — everything below, in one index.
- [User guides](docs/user/README.md) — install, configuration, auth, permissions, goals, sessions.
- [Audit and plan](audit/README.md) — what is verified, what is not, and what is next.
- [Architecture decisions](docs/design/epsilon-decisions.md) — recorded intent, including
  commitments not yet implemented.
- [References and credits](docs/references/README.md) · [CREDITS.md](CREDITS.md)
- [Project history](docs/history.md) — the milestone timeline.
- [Contributing](CONTRIBUTING.md)

## References and credits

The reference harnesses studied while building this, none of them vendored into the build.
Every fact below — author, link, licence — was read from each project's own `LICENSE` and
package metadata. **No third-party source code was copied into this repository**; what was
taken is behaviour, structure and design decisions.

| Reference | Author / organisation | Licence | Areas studied |
|---|---|---|---|
| [pi 1.0.0](https://github.com/earendil-works/pi) | Mario Zechner (`earendil-works`) | MIT | Agent loop, queues, coding tools, sessions, resources, prompt assembly |
| [DeepSeek Harness 0.2.1-alpha.1](https://github.com/deepseek-ai/deepseek-harness) | DeepSeek | MIT | Bounded capture, subprocess lifecycle, design tokens, IDE workflow |
| [Hermes Agent 2026.9.24](https://github.com/NousResearch/Hermes-Agent) | Nous Research | MIT | Observer identity, verification evidence, prompts, recovery |
| [opencode 1.18.34](https://github.com/anomalyco/opencode) | opencode (`anomalyco`) | MIT | Permissions, change provenance, language-server edit feedback |
| [OpenTUI 0.5.14](https://github.com/anomalyco/opentui) | opentui (`anomalyco`) | MIT | Terminal UI design — ideas taken, library not adopted |
| [codewhale-ratatui](https://github.com/Hmbown/codewhale-ratatui) | Codewhale Contributors | MIT | Presentation-layer discipline, terminal-profile fallbacks |

Two agent skills shape how work is done here, both MIT:

| Skill | Author | Licence | What it contributes |
|---|---|---|---|
| [Ponytail](https://github.com/DietrichGebert/ponytail) | DietrichGebert | MIT | The minimalism ladder: reuse, standard library, platform, smallest change |
| [improve](https://github.com/shadcn/improve) | shadcn | MIT | The audit produces a self-contained **plan** for a less capable executor, and the advisor never edits source |

Full detail — one page per reference, what we adopted, adapted and skipped, and
[the TUI stack decision](docs/references/TUI-DECISION.md) — is in
[`docs/references/`](docs/references/README.md). Formal acknowledgement is in
[`CREDITS.md`](CREDITS.md). [models.dev](https://models.dev) supplies catalog research via
`scripts/regen_catalog.py`.

These acknowledgements credit the ideas; they are not a claim of complete fidelity.
