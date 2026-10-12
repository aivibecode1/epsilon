# epsilon-mcp

The MCP (Model Context Protocol) client: declarative server config, a transport seam, and a
bridge that puts a server's tools into the ordinary tool registry.

## What this crate is

Three layers. `McpServerConfig` describes a server. `Connector` is the transport seam — list
tools, call one, probe health. `bridge` registers everything a connector exposes into
`epsilon_core::ToolRegistry` as `mcp__<server>__<tool>`, reached like any other tool.

It does **not** own tool execution policy, permissions or the loop. Nor does it own the rmcp
types: those stay inside `src/stdio.rs`; the rest of the crate sees only `Connector` and
`McpToolInfo` (`src/stdio.rs:3`).

## How it connects

Workspace dependencies: `epsilon-contracts`, `epsilon-core`. External: serde, serde_json,
async-trait, tokio, rmcp 3.1.4. Dev: rmcp with its `server` and `transport-io` features.
Depended on by `epsilon-cli` (the `--mcp` flag), `epsilon-runtime` (wiring servers into a
composed agent) — `McpServerConfig` is a `RunParams` field, so naming it here is unavoidable.

## Entry points

| Item | Where |
|---|---|
| `McpServerConfig` — name, command, args | `src/lib.rs:25` |
| `trait Connector` and `McpToolInfo` — the transport seam | `src/lib.rs:37`, `src/lib.rs:55` |
| `qualified_name` — `mcp__<server>__<tool>` | `src/lib.rs:65` |
| `bridge` — register a connector's tools | `src/lib.rs:73` |
| `StdioConnector::spawn` / `shutdown` | `src/stdio.rs:41`, `src/stdio.rs:71` |
| `attach_stdio_server` — spawn and bridge in one call | `src/stdio.rs:149` |

## How to run and test it

Servers are attached by the CLI; this crate has no entry point of its own.

```sh
cargo run -q -p epsilon-cli -- --mcp github=gh-mcp    # TUI with one server attached
cargo test -p epsilon-mcp
```

## Things worth knowing

- `tests/stdio_connector.rs` spawns `mcp-fixture-server` (`src/bin/mcp_fixture_server.rs`), a
  real MCP server over real pipes. That binary needs the features `rmcp/server` and
  `rmcp/transport-io`, declared as `required-features` at `Cargo.toml:22` and supplied by
  `[dev-dependencies]`, so it is built by `cargo test -p epsilon-mcp`, not by
  `cargo run --bin mcp-fixture-server`.
- Third-party MCP output is **untrusted data**: it is passed back verbatim as text and no
  instruction embedded in it is ever acted on (`src/lib.rs:158`).
- Health is probed at most once per `HEALTH_TTL_MS` (`src/lib.rs:111`), and again after every
  failure, so a dead server fails fast with a sentence rather than an opaque transport error.
- `ScrubEnv` is injected by the harness, not imported: this crate does not depend on
  `epsilon-safety`. An MCP server is a third-party binary whose command line comes from a
  config file, so it must not inherit the developer's whole environment (`src/stdio.rs:36`).
- `stdin` is explicitly piped (`src/stdio.rs:50`); left unset it would be inherited, and in the
  TUI the server would compete with crossterm for keystrokes.
- Rules: [`AGENTS.md`](../../AGENTS.md),
  [`docs/design/epsilon-decisions.md`](../../docs/design/epsilon-decisions.md).