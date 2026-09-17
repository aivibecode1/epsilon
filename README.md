# Epsilon

<p align="center">
  <img src="assets/Logo.png" alt="Epsilon logo" width="720">
</p>

Epsilon is a Rust coding-agent harness with terminal, headless, HTTP/SSE and
stdio RPC frontends. Its goal is to port pi-agent's coding workflows to Rust,
while adopting useful execution, permissions and recovery mechanisms from
DeepSeek Harness, Hermes Agent and opencode. **The port is incomplete.**

> **Development status:** incomplete pi port, not production-ready. The latest
> [Windows audit battery](audit/09-revalidation.md) passed, but source review found
> unresolved persistence, transport and workflow defects. Use backed-up or disposable
> workspaces. Passing regression tests is not proof of complete port fidelity.

## Contents

- [Start here](#start-here)
- [Capabilities and limits](#capabilities-and-limits)
- [Architecture](#architecture)
- [Development and testing](#development-and-testing)
- [Documentation](#documentation)
- [References and credits](#references-and-credits)

## Start here

You need Rust 1.88 or newer and the platform's Rust build prerequisites. Rust 1.97.1
was used for the recorded Windows verification; MSRV and Linux CI lanes are
configured but were not rerun in that audit. Shell commands need a shell executable;
Linux shell confinement additionally needs `bwrap`. Optional LSP, MCP and extension
features need their own installed programs. There is no required Node/Python runtime
for the Rust harness itself, but this is not a dependency-free execution environment.

From a source checkout:

```sh
git clone https://github.com/AIVibecode1/epsilon.git
cd epsilon
cargo run -q -p epsilon-cli -- --demo
```

The demo is offline and scripted; it does not contact a model provider. It exercises
the agent loop and intentional tool failure handling. A fresh network clone was not
tested in the latest audit. Local uncommitted features are not available merely
because this working tree passed its gates; see the audit for the exact baseline.

To install a reviewed build:

```sh
cargo install --path crates/epsilon-cli --force
epsilon --help
```

Real-provider support is enabled by default. Install only a reviewed build, not an
unreviewed local diff. Setup and platform limits are in [Install](docs/user/install.md).

For supervised real-provider use after configuring your own credentials:

```sh
epsilon auth login anthropic
epsilon --list-models
epsilon --model <model-id>
```

Enter credentials through the local login prompt, not as command-line arguments.
OAuth requires provider-specific setup; see [Auth](docs/user/auth.md). Live login
and a live coding workflow are not established by the offline demo.

Common commands:

| Command | Purpose |
|---|---|
| `epsilon` | Interactive terminal UI |
| `epsilon -p "summarize this repo"` | Headless prompt; approval requests default to denial |
| `epsilon doctor` | Inspect configuration and available capabilities without starting a model run |
| `epsilon -c` / `epsilon --resume <file>` | Continue a session; back up important transcripts first |
| `epsilon --serve --port 8787` | HTTP/SSE frontend |
| `epsilon --rpc` | Epsilon's NDJSON protocol, not pi RPC compatibility |
| `epsilon --plan` | Deny native write/edit/bash/shell while retaining project rules; not a side-effect classifier for extensions |
| `epsilon --help` | Implemented command inventory; known help-text overclaims are listed in the audit |

In the TUI, Enter sends a prompt, Esc requests cancellation and `/help` lists
commands. Cancellation and recovery limitations are documented in
[Sessions](docs/user/sessions.md).

## Capabilities and limits

Implemented mechanisms include read/write/edit/search tools, shell execution,
per-agent tool loops, delegation, provider adapters, retry/fallback, session JSONL,
compaction, permission prompts, process extensions, MCP and optional LSP diagnostics.
Their presence does not establish complete pi behavior or production readiness.

- **Permissions:** matching denies take precedence over allows and remembered approval.
  Other matching allow/ask decisions use the later rule. `--yes` automatically approves
  asks; it does not repair broken boundaries.
- **Confinement:** Windows Job Objects constrain resources/processes, not filesystem
  or network access. Linux bubblewrap restricts writes and network access but does not
  hide all host files. Native file tools are not wrapped by those subprocess backends.
  Default shell execution refuses when no backend is available; `--sandbox off`
  explicitly disables confinement. macOS has no native backend here.
- **Persistence and recovery:** append-only storage, leases and compaction exist, but
  ignored append failures, repeated-compaction projection and final-entry repair need
  hardening. Checkpoint capture is not yet safe user undo.
- **Providers and integrations:** protocol fixtures and compilation are useful evidence,
  not proof that every catalog model, OAuth flow or extension works. SSE buffering,
  refresh, run admission and diagnostic freshness have open findings.

See [Permissions](docs/user/permissions.md), [the defect ledger](audit/02-defect-ledger.md)
and [pi fidelity matrix](audit/03-port-plan.md) for concrete paths and acceptance tests.

## Architecture

The conventional Cargo workspace has twelve crates under `crates/`. CLI composes
adapters; core owns agent execution; shared contracts keep implementation-specific
types out of interfaces. Cargo enforces declared dependencies and rejects cycles,
not a separate architectural layering policy.

| Crate | Responsibility |
|---|---|
| `epsilon-contracts` | Shared types/traits; currently also validation and stateful stream folding |
| `epsilon-llm` | Provider protocols, faux provider, routing, catalog, auth, retry/fallback |
| `epsilon-session` | JSONL storage, indexing, branch projection, repair and leases |
| `epsilon-core` | Agent loop, scheduling, context, compaction and task lifecycle |
| `epsilon-tools` | Native tools, shell adapter, observations, mutation locks and checkpoints |
| `epsilon-safety` | Permission rules/approvals, environment filtering and sandbox adapters |
| `epsilon-lsp` | Optional language-server discovery/client and diagnostic reports |
| `epsilon-ext` | Process extension protocol and host |
| `epsilon-mcp` | MCP client and tool adapter |
| `epsilon-tui` | Ratatui frontend and editor |
| `epsilon-server` | HTTP/SSE frontend |
| `epsilon-cli` | Executable, arguments, configuration and runtime composition |

The [architecture guide](docs/design/epsilon-architecture.md) maps the actual crate
edges and execution paths. The contracts crate's executable helpers are a known
departure from the types-only policy, not a newly accepted architectural exception.

## Development and testing

Read [AGENTS.md](AGENTS.md) before changing code. Run from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p epsilon-cli -- --demo
cargo check -p epsilon-cli --features real
```

`bash scripts/gate.sh <label>` runs those gates and writes `target/gate-<label>.log`.
Keep relevant evidence outside `target/` before clearing build artifacts. Reinstall
the global CLI only after verified CLI/TUI changes; do not replace it with a failing
build. Cleanup commands and failed-round handling are in AGENTS.md.

The [current audit](audit/09-revalidation.md) names the tested tree, gate results,
remaining defects and untested surfaces. Tests use faux providers and local fixtures
by default; live-provider checks are opt-in. Windows, Linux and MSRV CI definitions
live in `.github/workflows/ci.yml`; configured lanes are not evidence they ran locally.

## Documentation

- [Documentation map](docs/README.md): user guides, contributor references and historical records.
- [User guides](docs/user/README.md): installation, configuration, auth, permissions and sessions.
- [Audit and roadmap](audit/README.md): verified findings, evidence limits and ordered hardening work.
- [Architecture decisions](docs/design/epsilon-decisions.md): recorded intent, including commitments not yet implemented.
- [Project history](docs/history.md): historical milestones, not current gate certification.

## References and credits

The locally inspected reference snapshots (not included in a clone) are:

| Reference | Areas studied |
|---|---|
| [pi 0.84.2](https://github.com/earendil-works/pi) | Agent loop, queues, coding tools, sessions, resources and embedding |
| [DeepSeek Harness v0.1.1-rc.2](https://github.com/deepseek-ai/deepseek-harness) | Bounded capture, subprocess lifecycle and confinement |
| [Hermes Agent 2026.8.19](https://github.com/NousResearch/hermes-agent) | Observer identity, verification evidence, prompts and recovery |
| [opencode 1.18.21](https://github.com/anomalyco/opencode) | Permissions, change provenance and LSP edit feedback |

[Reference comparison](audit/06-reference-comparison.md) explains which mechanisms
fit and which do not. [models.dev](https://models.dev) supplies catalog research via
`scripts/regen_catalog.py`. These acknowledgements do not certify complete port
fidelity or substitute for a source/license provenance review.
