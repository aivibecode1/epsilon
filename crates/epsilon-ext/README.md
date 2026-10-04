# epsilon-ext

The process-extension protocol and the host that runs it (ADR-009). Extensions are child
processes speaking line-delimited JSON over stdio, so third-party code never runs in-process.

## What this crate is

A manifest loader, a host process, and the adapter that presents one extension's tool to the
agent as an ordinary tool.

It does **not** decide which extensions load, when they run, or what they may do beyond the
isolation the host provides: a per-call timeout, a crash budget, and an event filter declared
in the manifest. That belongs to `epsilon-runtime`.

## How it connects

Workspace dependency: `epsilon-contracts`. External: serde, serde_json, async-trait, tokio.
Dev: tempfile, tokio-util. Depended on by `epsilon-cli` and `epsilon-runtime`, which scans
manifest directories, calls `manifest::load`, spawns each host, registers `ExtTool`s and wires
`ExtensionToolHooks` into the loop (`crates/epsilon-runtime/src/lib.rs:909`).

## Entry points

| Item | Where |
|---|---|
| `ExtHost::spawn` — start the child and complete the handshake | `src/host.rs:191` |
| `ExtHost::invoke` — call one tool | `src/host.rs:338` |
| `ExtHost::interpose` — ask for a decision before a call | `src/host.rs:515` |
| `ExtError` / `ScrubEnv` | `src/host.rs:54`, `src/host.rs:44` |
| `ExtManifest` / `ExtToolSpec` | `src/manifest.rs:20`, `src/manifest.rs:9` |
| `manifest::load` — read and validate `extension.json` | `src/manifest.rs:47` |
| `INVOKE_TIMEOUT_SECS` 10, `INTERPOSE_TIMEOUT_SECS` 1, `CRASH_BUDGET` 2 | `src/lib.rs:27`, `src/lib.rs:45`, `src/lib.rs:47` |

## How to run and test it

Extensions are discovered and started by the composed agent, not by this crate. See
[Extensions](../../docs/user/extensions.md) for the `extension.json` shape.

```sh
cargo test -p epsilon-ext
```

The integration tests spawn `ext-fixture` (`src/bin/ext-fixture.rs`), whose argv selects a
misbehaviour — `--die`, `--sleep`, `--garbage`, `--bad-shape`, `--huge` and more, one per
failure path the host claims to survive.

## Things worth knowing

- `INTERPOSE_TIMEOUT_SECS` is one second, deliberately far below the ten-second invoke
  budget. An interpose happens before *every* tool call, so its cost multiplies across a
  turn, and a decision is a yes or a no, not work (`src/lib.rs:29`).
- The reader task owns the child's stdout; the host keeps only the write side
  (`src/host.rs:82`). One mutex over one pipe could not serve concurrent invokes: a reply
  tagged `i_7` could not be routed to whoever is waiting on `i_8`.
- `scrub_env` is a parameter rather than an import: `epsilon-ext` is a leaf crate and does not
  depend on `epsilon-safety` (`src/host.rs:185`). `epsilon-lsp` and `epsilon-mcp` agree.
- Every error is a value: a crashing extension degrades the extension, never the harness
  (`src/host.rs:46`), and exceeding an interpose timeout means "no opinion", so the call goes
  ahead.
- Rules: [`AGENTS.md`](../../AGENTS.md),
  [`docs/design/epsilon-decisions.md`](../../docs/design/epsilon-decisions.md).