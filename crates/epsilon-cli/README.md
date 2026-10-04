# epsilon-cli

The binary, installed as `epsilon`. It parses arguments and dispatches to a mode.

## What this crate is

It owns the command line: flag parsing, mode selection, and the few process-level concerns
(auth subcommands, `doctor`, session import) that must work without an agent.

It does **not** own composition. Turning a workspace plus a configuration into a running
agent belongs to `epsilon-runtime` (ADR-021); this crate consumes `build_runtime` and
`resolve_launch`. That is why the desktop IDE can build the same agent without depending on
this binary.

## How it connects

Workspace dependencies: `epsilon-contracts`, `epsilon-llm`, `epsilon-core`,
`epsilon-runtime`, `epsilon-tools`, `epsilon-lsp`, `epsilon-safety`, `epsilon-session`,
`epsilon-tui`, `epsilon-server`, `epsilon-mcp`, `epsilon-ext`. Dev: `portable-pty`, `vt100`.
Depended on by nothing in the workspace. It has a library target so its own tests can reach
`doctor` and `rpc_mode`, and that target re-exports the composition crate as
`epsilon_cli::runtime` (`src/lib.rs:12`) to keep older paths working.

## Entry points

| Item | Where |
|---|---|
| `main` — parse, handle the pre-runtime modes, build tokio, dispatch | `src/main.rs:601` |
| `Mode` — one variant per mode | `src/main.rs:218` |
| `Args` / `parse_args` | `src/main.rs:172`, `src/main.rs:260` |
| `dispatch` — mode dispatch once the runtime exists | `src/main.rs:1152` |
| `doctor::gather` / `doctor::run` — the diagnostic report | `src/doctor.rs:264`, `src/doctor.rs:578` |
| `rpc_mode::run` — NDJSON over stdio | `src/rpc_mode.rs:1327` |
| `pub use epsilon_runtime as runtime` | `src/lib.rs:12` |

## How to run and test it

```sh
cargo run -q -p epsilon-cli                  # interactive TUI
cargo run -q -p epsilon-cli -- --demo        # offline, scripted, no provider needed
cargo run -q -p epsilon-cli -- --help        # every flag, env var and file
cargo test -p epsilon-cli
```

## Things worth knowing

- `real` is a **default** feature and forwards to four crates (`Cargo.toml:30`). Miss a
  forwarding and `Agent::try_hot_swap_to_real` compiles to a `false` stub, so the TUI's
  "now live on X" hot-swap silently degrades to "restart epsilon".
- `die()` (`src/main.rs:596`) exits with code 2. That is acceptable *here*, in a binary, and
  nowhere else: a function shared with a window returns an error.
- `auth`, `doctor` and `--import` run before the tokio runtime is built (`src/main.rs:605`).
  A diagnostic that first constructs the machinery cannot diagnose the machinery, and the
  startup banner would print over the report.
- `tests/terminal.rs` drives the **built binary** through a real pseudo-terminal. In-process
  `TestBackend` tests show what a frame looks like and cannot tell you the binary puts it on
  a console: terminal init, raw mode and the alternate screen are all outside that boundary.
- The mode list is `HELP` at `src/main.rs:56` — the shipped binary is its own documentation.
- Rules: [`AGENTS.md`](../../AGENTS.md),
  [`docs/design/epsilon-decisions.md`](../../docs/design/epsilon-decisions.md).