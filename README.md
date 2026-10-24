# Epsilon

<p align="center">
  <img src="assets/Logo.png" alt="Epsilon logo" width="720">
</p>

Epsilon is a coding-agent harness written in Rust: it talks to a model, gives it tools,
decides what those tools may do, and keeps a resumable session. The same agent core is
driven by a full-screen terminal IDE, a headless prompt mode, an HTTP/SSE server and a
stdio-RPC frontend.

It began as a port of [pi](https://github.com/earendil-works/pi) and borrows execution,
permission and recovery ideas from DeepSeek Harness, Hermes Agent, opencode and
codewhale-ratatui. It can attach [REA](https://github.com/morluto/rea) for binary and
app reverse engineering. It is a release candidate, not a final certified release;
the current limitations are in [RELEASE.md](RELEASE.md) and
[docs/04-STATUS.md](docs/04-STATUS.md).

## Contents

- [Quick start](#quick-start)
- [What it does](#what-it-does)
- [The terminal IDE](#the-terminal-ide)
- [Modes](#modes)
- [Models and credentials](#models-and-credentials)
- [Documentation](#documentation)
- [How it is built](#how-it-is-built)
- [Working on Epsilon](#working-on-epsilon)
- [Current status](#current-status)
- [Release status](#release-status)
- [Credits and licence](#credits-and-licence)

## Quick start

Prerequisites: **Rust 1.88 or newer**. Windows and Linux are the development targets.
Windows has a Job Object confinement backend; Linux uses bubblewrap where installed.
macOS builds but has no native shell-confinement backend, so side-effecting shell calls
refuse by default.

```sh
git clone https://github.com/AIVibecode1/epsilon.git
cd epsilon

# Windows
powershell -ExecutionPolicy Bypass -File .\install.ps1
# macOS / Linux
bash install.sh

epsilon --help                                     # all modes, flags, env vars and files
epsilon --demo                                     # offline scripted run: no key, no network
```

The installer uses `cargo install --locked`, verifies the binary and prints the path.
`--prefix`/`-Prefix` chooses a location; `--no-verify` skips the smoke check.

Then open the IDE in a project directory:

```sh
cd /path/to/your/project
epsilon
```

Without installing:

```sh
cargo run -q -p epsilon-cli -- --demo
cargo run -q -p epsilon-cli
```

If `epsilon` is not found after install, add the installer's printed directory
(usually `~/.cargo/bin`) to `PATH`.

## What it does

Point Epsilon at a **workspace** — the folder it may read and change — and describe a task
in plain language. It reads files, searches, edits, runs commands, delegates work to
subagents and keeps a session you can resume.

Every side-effecting action passes a permission gate first. The gate is layered: defaults,
then project rules, then an optional preset, then plan mode. Deny wins. On Windows shell
commands run inside a Job Object; on Linux they use bubblewrap when available.

The interesting parts:

- **One agent core, several frontends** — TUI, `-p`, HTTP/SSE and stdio-RPC all use the
  same loop, tools and permission engine.
- **A real terminal IDE** — right-hand project explorer, file viewer/editor, agent
  transcript, tool cards, command palette and settings panels. User and assistant
  messages are independent cards; the explorer is live, with `git` status markers.
- **Sessions that survive** — stored as JSONL with leases and a repair pass on resume.
- **Tooling** — file read/write/edit, shell, search, glob, web fetch, LSP diagnostics, MCP
  servers, process extensions, skills and subagent tasks.
- **A prompt-cache-aware wire** — stable system/tool prefixes and a moving tail
  breakpoint on the last user-role turn, including tool results. `/stats` reports what
  the provider actually charged.
- **Skills that are opt-in** — twelve built-in procedures ship in the binary; `/skills`
  shows what each does, why it matters and its upstream credit, and `/skill <name> on`
  enables one.
- **Subagents that are opt-in** — `explore`, `review`, `verify` and `implement` are
  bundled subagent roles. `/subagent` opens the browser; `/subagent <name> on` installs
  one as an ordinary `task` agent definition. They stay absent until you ask. The naming
  follows DeepSeek Harness: a *subagent* is the delegated role, a *harness* is the
  program.
- **Optional reverse engineering** — `epsilon --rea` attaches REA's MCP server for
  binaries, Electron apps, .NET assemblies and sites.
- **No Node or Python runtime** — one Rust binary for the harness itself. REA is the one
  optional exception, and only when you ask for it.

## The terminal IDE

The explorer is docked on the **right** by default. It does not take the keyboard until you
focus it, so the composer always works first.

| Key | Action |
|---|---|
| `Enter` | send the prompt |
| `Esc` | cancel the run, close an overlay, or return focus to the composer |
| `F6` | show and focus the explorer; focus a resting one; hide a focused one |
| `Left` / `Right` | collapse / expand a directory in the explorer |
| `Enter` in the explorer | open a file in the viewer, or toggle a directory |
| `@` | insert a workspace file reference |
| `/` | open the command palette |
| `/help` | every slash command and key hint |
| `Ctrl+G` | refresh the live model listing (bound by default; the hint names the key) |

The explorer remembers whether it is open and which directories are collapsed, per
workspace. Empty directories are visible. While it is visible it polls the workspace
every 900 ms and `git status` every 3 s, so files the agent adds or edits appear
without pressing `r`; `A`/`M`/`?`/`D`/`R`/`!` mark the working tree, and a changed
directory gets a roll-up dot. The header shows the model, context use, cache rate and
running cost.

## Modes

| Command | What it does |
|---|---|
| `epsilon` | interactive terminal IDE |
| `epsilon -p "summarize this repo"` | one prompt, headless; reply on stdout, diagnostics and usage on stderr |
| `epsilon doctor` | report configuration and capabilities without running a model |
| `epsilon -c` / `epsilon --resume <file>` | continue a session |
| `epsilon --serve --port 8787` | HTTP/SSE frontend |
| `epsilon --rpc` | NDJSON over stdio; accepts pi-style frames |
| `epsilon --plan` | deny mutating capabilities, keep project rules |
| `epsilon --rea` | attach REA's reverse-engineering MCP server (Node/npx required) |
| `epsilon --demo` | offline scripted turn |
| `epsilon --list-models [provider]` | catalog snapshot and credential availability |

## Models and credentials

The bundled catalog works offline for browsing:

```sh
epsilon --list-models
```

To use a real provider, log in once through the local prompt:

```sh
epsilon auth login deepseek          # or anthropic, openai, openrouter, …
epsilon auth login anthropic --oauth # Claude Pro/Max
epsilon --model deepseek-flash
```

Stored credentials live in `~/.config/epsilon/auth.json` and win over environment
variables such as `DEEPSEEK_API_KEY`, `ANTHROPIC_API_KEY` and `OPENAI_API_KEY`.
**The file is plaintext**, matching Pi 1.3.0's own `auth.json`; the OS-keyring
backend is optional hardening. It is owner-only on Unix, and login/list warns.
Environment variables are the better choice in CI or on a shared machine. See
[docs/user/auth.md](docs/user/auth.md).

Custom OpenAI-compatible endpoints are declared in `config.toml`:

```toml
[provider_custom.lmstudio]
base_url = "http://localhost:1234/v1"
# env_key = "LMSTUDIO_API_KEY"   # omit for keyless local servers
```

Then select `lmstudio/<model>`. See [docs/user/models.md](docs/user/models.md) and
[docs/user/configuration.md](docs/user/configuration.md).

## Documentation

Start with the [documentation map](docs/README.md).

| Document | What it answers |
|---|---|
| [01 — Overview](docs/01-OVERVIEW.md) | a ten-minute tour, reading order and folder map |
| [02 — Architecture](docs/02-ARCHITECTURE.md) | crates, dependency edges and execution paths |
| [03 — Development](docs/03-DEVELOPMENT.md) | build, test, battery, machine rules and recovery |
| [04 — Status](docs/04-STATUS.md) | what is true now; the latest gate and limitations |
| [05 — User guide](docs/05-USER-GUIDE.md) | install, configuration, auth, models, permissions, sessions, goals, skills, subagents, reverse engineering |
| [06 — Glossary](docs/06-GLOSSARY.md) | harness, turn, tool call, compaction, lease and the rest |
| [07 — Commands](docs/07-COMMANDS.md) | every slash command, alias, argument and confirmation |
| [09 — TUI reference](docs/09-TUI-REFERENCE.md) | the frame, regions, focus rules, keys, themes and states |
| [10 — Extension points](docs/10-EXTENSION-POINTS.md) | tools, MCP, extensions, skills, agents and frontends |
| [ADR log](docs/design/epsilon-decisions.md) | why the product is shaped this way |
| [Pi parity matrix](docs/parity/pi-tui.md) | the live terminal-IDE comparison |
| [CONTRIBUTING.md](CONTRIBUTING.md) | the short contributor path |
| [CHANGELOG.md](CHANGELOG.md) | user-visible changes, newest first |

There is no documentation archive. A page is current or it is corrected; the decision
record and the living limitations are in [docs/04-STATUS.md](docs/04-STATUS.md).

## How it is built

Thirteen crates under `crates/`. Each has one job; `epsilon-runtime` is the only place a
workspace plus a configuration becomes a running agent.

```text
epsilon-contracts   shared types, traits, command and keybinding registries
epsilon-llm         provider protocols, faux provider, routing, catalog, auth, retry
epsilon-session     JSONL storage, branching, leases and repair
epsilon-core        the agent loop, scheduling, context assembly, compaction, tasks
epsilon-tools       native tools, shell adapter, checkpoints, goals
epsilon-safety      permission engine, approvals, sandbox backends
epsilon-lsp         optional language-server client and diagnostics
epsilon-ext         process-extension protocol and host
epsilon-mcp         MCP client and tool adapter
epsilon-runtime     composition: the one place configuration becomes an agent
epsilon-tui         terminal IDE
epsilon-server      HTTP/SSE frontend
epsilon-cli         the binary: argument parsing and mode dispatch
```

The dependency edges and execution paths are in
[docs/02-ARCHITECTURE.md](docs/02-ARCHITECTURE.md). Behavioural
decisions are recorded as ADRs in
[docs/design/epsilon-decisions.md](docs/design/epsilon-decisions.md).

## Working on Epsilon

Read [AGENTS.md](AGENTS.md) before changing code. The short version:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p epsilon-cli -- --demo
cargo check -p epsilon-cli --features real
```

The full battery has **fourteen legs**:

```sh
pwsh -File scripts/gate.ps1 <label>    # Windows
bash scripts/gate.sh <label>           # POSIX / CI
```

It runs the repository checks, `fmt`, `clippy`, the workspace tests, the offline demo and
the real-feature build. The log lands in `target/gate-<label>.log`; copy anything you want
to cite somewhere durable before clearing `target/`. On this machine, build with one job and
outside the tree:

```sh
$env:CARGO_TARGET_DIR = "$env:TEMP\epsilon-target"
$env:CARGO_BUILD_JOBS = "1"
$env:CARGO_PROFILE_TEST_DEBUG = "0"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
```

Tests use the deterministic faux provider by default; nothing hits a paid API unless a
test is explicitly enabled.

## Current status

**Verified (2026-10-24):** the full battery is **14/14 legs green** — repository
checks, `fmt`, `clippy -D warnings`, the workspace suite, the offline demo and the
real-feature build. The workspace suite is **77 test binaries, 2,658 passed / 0
failed / 21 ignored**; the terminal-IDE suite is **918 passed / 0 failed / 20
ignored**. Evidence: `gate-final-audit-7.log` (debug info off,
`CARGO_BUILD_JOBS=1`, isolated target). The installer result is in
[docs/04-STATUS.md](docs/04-STATUS.md).

This is a release candidate. The honest limitation list is in [docs/04-STATUS.md](docs/04-STATUS.md)
and the release classification is in [RELEASE.md](RELEASE.md). Two that matter up front:

- **Credentials are plaintext** in `auth.json`, matching Pi 1.3.0; the OS-keyring
  backend is optional hardening, not a parity blocker. Keep the file owner-only and
  prefer environment variables in CI.
- **Live-only model ids** listed by a provider are shown as `[unavailable]` rather than
  routed on guessed context, prices or capabilities. Add a catalog row or a
  `[provider_custom]` declaration to use one deliberately.

Use disposable or backed-up workspaces while the defect ledger is non-empty.

## Release status

**0.1.0 is a release candidate.** The credential model follows Pi 1.3.0, which also
stores `auth.json` in plaintext at mode 0600; Epsilon adds atomic replacement and a
0700 directory. OS-keyring storage is optional hardening, not a parity requirement.
[RELEASE.md](RELEASE.md) records the verified evidence, the pushed commit and the
remaining caveat: the macOS CI job still needs to run before tagging. Live
cache-hit rates are not claimed.

## Credits and licence

Epsilon stands on other people's work. This section names it so a reader can check.

Every author, link and licence below was read from that project's own `LICENSE`, `README`
and package metadata in the local vendored copy, not from memory. Where a fact could not be
verified, it is not claimed.

**`references/` is local-only.** The vendored reference sources are read-only research input:
they are ignored by Git (`/references/` in `.gitignore`), are not tracked, and are never
uploaded to the cloud repository. Only the written studies under
[`docs/references/`](docs/references/README.md) are committed.

### Reference harnesses and components

Epsilon began as a port of **pi** and has since been shaped by five more harness
projects plus one reverse-engineering component. All seven are MIT-licensed, and
Epsilon is MIT, so code could legally have been adapted with attribution. In the
event, **no third-party source code was copied into this repository**: what was
taken is behaviour, structure and design decisions. Two MIT-licensed **skill
texts** are adapted and bundled with credit — Ponytail and improve — and their
attribution travels with the skill in `/skills`. REA is invoked as an external
npm package over MCP and is not vendored.

| Project | Author / organisation | Repository | Licence |
|---|---|---|---|
| **pi** | Mario Zechner (`earendil-works`) | https://github.com/earendil-works/pi | MIT — "Copyright (c) 2025 Mario Zechner" |
| **DeepSeek Harness** | DeepSeek | https://github.com/deepseek-ai/deepseek-harness | MIT — "Copyright (c) 2026 DeepSeek" |
| **Hermes Agent** | Nous Research | https://github.com/NousResearch/Hermes-Agent | MIT — "Copyright (c) 2025 Nous Research" |
| **opencode** | opencode (`anomalyco`) | https://github.com/anomalyco/opencode | MIT — "Copyright (c) 2025 opencode" |
| **OpenTUI** | opentui (`anomalyco`) | https://github.com/anomalyco/opentui | MIT — "Copyright (c) 2025 opentui" |
| **codewhale-ratatui** | Codewhale Contributors | https://github.com/Hmbown/codewhale-ratatui | MIT — "Copyright (c) 2026 Codewhale Contributors", and "Portions adapted from Codewhale: Copyright (c) 2024-2025 DeepSeek-TUI Contributors" |
| **REA (Reverse Engineer Anything)** | morluto | https://github.com/morluto/rea | MIT — "Copyright (c) 2026 morluto" |

### What each one taught us

**pi** — the agent harness this project is a port of, and the reason its loop is shaped the
way it is: two levels of provider turns and tool batches, with steering and follow-up queues
that let a person interrupt a run without cancelling it. `epsilon-core` exists because of
pi's design.

**DeepSeek Harness** — bounded capture of subprocess output, subprocess lifecycle discipline,
design tokens, and the shape of an IDE workflow: sidebar, editor, agent panel, status. We
took the workflow, not the Electron carrier. See ADR-020 in
[docs/design/epsilon-decisions.md](docs/design/epsilon-decisions.md).

**Hermes Agent** — an observer that knows what it has actually seen: read observations
scoped to a lifetime token, verification evidence attached to a claim, and recovery paths
that assume something went wrong.

**opencode** — permission rules worth reading closely, change provenance, and editing guided
by real language-server feedback rather than a model guessing. Its file-oriented
`apply_patch` envelope (`*** Begin Patch` … `*** End Patch`, with `*** Add/Update/Delete
File` headers and `@@` hunks) is the patch format Epsilon's `apply_patch` tool speaks
(ADR-059). It is also the project that put OpenTUI into production, which is how OpenTUI
came to our attention.

**OpenTUI** — three ideas, taken without the library: layout declared as relationships and
resolved in one place; key binding separated from command routing; and an agent-facing
`AGENTS.md` that briefs delegates with owned files and required checks. The reasoning for
not adopting the library is in
[docs/references/TUI-DECISION.md](docs/references/TUI-DECISION.md).

**codewhale-ratatui** — the presentation-layer discipline of a component kit that owns paint
and nothing else, the terminal-profile fallbacks Epsilon has (`NO_COLOR`, ASCII-only
glyphs), and the practice of pinning a profile per test. Its whale-and-ocean identity
belongs to its authors and is not ours. Epsilon has no workbar and no reduced-motion path;
those are gaps, recorded in
[docs/references/codewhale-ratatui.md](docs/references/codewhale-ratatui.md), not parity.

**REA** — the evidence-first idea applied to software nobody has the source for: an
observation names its provider and its limits, and "the provider is not installed" is a
finding rather than a guess. Epsilon integrates it as an optional MCP server
(`epsilon --rea`), pins the package version, wraps the Windows launch itself, and carries
the workflow as an opt-in skill. REA is not vendored. See
[docs/references/rea.md](docs/references/rea.md) and
[docs/user/reverse-engineering.md](docs/user/reverse-engineering.md).

### Tools

| Project | Author / organisation | Repository | Licence |
|---|---|---|---|
| **Ponytail** | DietrichGebert | https://github.com/DietrichGebert/ponytail | MIT — "Copyright (c) 2026 DietrichGebert" |
| **improve** | shadcn | https://github.com/shadcn/improve | MIT — "Copyright (c) 2026 shadcn" |

(The Ponytail repository's commit history records the author identity as
`Peter <8146306+p-clements@users.noreply.github.com>`, consistent with that handle.)

**Ponytail** is the minimalism discipline this project works by, installed as an agent skill
at `.agents/skills/ponytail*` and bundled opt-in as the `ponytail` built-in. Its ladder —
does this need to exist, is it already here, does
the standard library do it, does the platform do it, is an installed dependency enough —
runs *after* reading the problem, never instead of it. It explicitly must not be used to
skip input validation, error handling that prevents data loss, security or accessibility;
`AGENTS.md` repeats that limit.

**improve** is installed at `.agents/skills/improve`, from `skills/improve` in that
repository; the local copy is MIT and it is also bundled opt-in as the `improve`
built-in with its three reference pages. It surveys a codebase and writes
**plans for a different, less capable executor**, and its first hard rule is that it never
edits source itself. Epsilon's findings are written to be acted on by whoever picks them up
next, not re-derived from a conversation.

### Data and licence

**[models.dev](https://models.dev)** supplies model-catalogue research via
`scripts/regen_catalog.py`. It is a data source, not a code dependency.

Epsilon is licensed under the [MIT Licence](LICENSE). Copyright © 2026 the Epsilon authors.
