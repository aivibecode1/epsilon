# epsilon-lsp

An optional language-server client. It asks the server the machine already has what the
compiler would say about the file the agent just wrote, and rides the answer back on the
edit's own tool output.

## What this crate is

Five small pieces: JSON-RPC framing over stdio, a table of known servers, a child-process
client, the lifecycle that keeps those children from becoming a process leak, and a bounded
renderer for the diagnostics.

It does **not** own the agent loop, the edit tool, or the tool output format — it returns a
string, or nothing, and the caller decides where it goes. That is why it depends only on
`epsilon-contracts` and never on `epsilon-core`. Degradation is silent by design: with no
`rust-analyzer` installed, behaviour is exactly what it was before this crate existed.

## How it connects

Workspace dependency: `epsilon-contracts`. External: serde, serde_json, async-trait, tokio.
Dev: tempfile, plus tokio's test features. Depended on by `epsilon-cli` (which resolves the
`--lsp auto|off` flag) and `epsilon-runtime` (which builds the service the loop consults).

## Entry points

| Item | Where |
|---|---|
| `DiagnosticsService::new` / `after_change` | `src/service.rs:89`, `src/service.rs:136` |
| `DiagnosticsService::discover` — servers this machine can run | `src/service.rs:101` |
| `ServiceConfig` — where servers are found, timeouts, env scrubber | `src/service.rs:22` |
| `SERVERS` — the built-in table | `src/server.rs:30` |
| `select_server` / `which` / `project_root` | `src/server.rs:88`, `:99`, `:123` |
| `render_report`, `MAX_REPORTED` | `src/report.rs:66`, `src/report.rs:60` |

## How to run and test it

```sh
cargo run -q -p epsilon-cli -- --lsp auto     # the default; the flag is the CLI's
cargo run -q -p epsilon-cli -- --lsp off
cargo test -p epsilon-lsp
```

`tests/end_to_end.rs` spawns a real child, `src/bin/lsp_fixture_server.rs`, a language server
whose rule is "one error per `TODO_BAD`" — so the assertions test behaviour the test controls.

## Things worth knowing

- **User-installed servers only.** The crate never downloads or installs one
  (`src/server.rs:3`): a coding agent that fetches and executes binaries is a supply chain.
- Every method is total. A missing server, a broken server and a server that never answers are
  all `None` (`src/service.rs:69`): the edit succeeded, and failing it for want of a linter
  would be worse than not linting.
- One client per (server id, project root), with a `broken` set beside it
  (`src/service.rs:78`), so a server that cannot handshake is not respawned every edit.
- The report is bounded on purpose: ERROR severity only, at most 20 per file
  (`src/report.rs:60`).
- `scrub_env` is passed in rather than imported: the subprocess credential rule lives in
  `epsilon-safety`, which this crate deliberately does not depend on.
- Rules: [`AGENTS.md`](../../AGENTS.md),
  [`docs/design/epsilon-decisions.md`](../../docs/design/epsilon-decisions.md).