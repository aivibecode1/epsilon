# M79 — language-server diagnostics as edit feedback

**Scope:** roadmap Phase 5 item 1 ("the largest remaining correctness lever for a coding agent"),
port-plan-adjacent; opencode `packages/opencode/src/lsp/*`, `tool/edit.ts:196-211`.

## The problem

The model wrote code and the *compiler* disagreed — a turn later, when the user ran the build, or
never. Every edit was made blind. opencode's answer is to ask the language server that already knows
the answer and ride the result on the edit's own output, and the mining file for this repository
called it the biggest gap in Epsilon's correctness loop.

## What landed

A new crate (`epsilon-lsp`, the twelfth — ADR-014 amendment) and a seam in `epsilon-contracts`:

| Part | What it does |
|---|---|
| `framing` | JSON-RPC over stdio with `Content-Length` headers. Thirty lines; the body length is bounded before anything is allocated, because the header is written by a third-party process. |
| `server` | Extension → server, `PATH` lookup, project root by markers, `file://` URIs with the characters that would break them encoded. **User-installed servers only** — opencode's per-server install commands are deliberately not ported. |
| `client` | Spawn, `initialize`/`initialized`, `didOpen`/`didChange` (full sync), `publishDiagnostics`; waits for the publish rather than guessing. Ignores what it does not understand, so nothing hangs on a message it did not expect. |
| `service` | One client per *(project root, server)*, the map lock held across a spawn (an in-flight dedupe that is guaranteed, not hoped for), and a `broken` set so a server that fails is not respawned on every edit. |
| `report` | The bounded block: **errors only**, at most twenty per file, file order, `ERROR [line:col] message`, messages flattened to one line so the tool output's shape survives. |
| `DiagnosticsProvider` (contracts) | The seam tools consume, so every tool test runs against a fake and no test needs a server installed. |

Consumed by `write` and `edit`: a successful mutation appends whatever the server has to say.

## Key decisions

- **Silent degradation, everywhere.** No server for the extension, no binary on `PATH`, a server that
  dies or refuses to handshake, a publish that never arrives, a probe that has no `DiagnosticsProvider`
  — all of them are `None`, and `None` changes nothing about an edit that already happened. A harness
  that fails an edit for want of a linter is worse than one that cannot lint.
- **Errors only, capped at twenty.** Warnings are the language server's opinions and the model cannot
  act on them; the cap is what keeps a diagnostics dump from pushing the file's contents out of view.
- **The brick earns existence.** A client of an external protocol with its own process lifecycle is
  the standing `epsilon-mcp` already occupies; that is why this is a crate and not a module inside
  `epsilon-tools`, and why the ADR says so (ADR-014 amendment, M79).
- **No new dependency.** The framing is thirty lines over `tokio::io`; `rmcp` owns MCP's framing, so
  mirroring was cheaper than reaching across a crate boundary.
- **The subprocess rule travels.** `epsilon-safety`'s credential scrubber is *injected* into the LSP
  service (`ServiceConfig.scrub_env`) rather than re-derived in the new crate — one policy, one place
  — and a conservative name-based floor applies even when nothing is injected.
- **Child environment is per-spawn.** Two tests running at once must not fight over one environment
  variable (`std::env::set_var` is `unsafe` in edition 2024 anyway, and this workspace forbids it).
- **The library default is inert.** `register_builtins` registers no provider; the product path opts
  in (`builtin_registry_with(_, Lsp::Auto)`). A library that silently spawns `rust-analyzer` because a
  test wrote a file would be a surprise.
- **The doctor discovers, it does not spawn.** Its `lsp servers` line is a `PATH` walk, so the
  diagnostic still runs before the runtime exists.

## Verification

Six mutations, six discriminations:

| Mutation | Result |
|---|---|
| warnings are reported too | 2 report tests fail |
| the report is never capped | the cap test fails |
| the tool drops what the server said | both carry tests fail |
| a broken server is retried every time | the refusal test fails (3 starts observed, not 1) |
| every change spawns a fresh server | the reuse test fails (2 starts, not 1) |
| an implausible `Content-Length` is accepted | the test process **aborts** allocating the advertised length — the exact accident the bound prevents, which is the strongest form of the answer |

The end-to-end tests drive a real child process (`lsp_fixture_server`, the same pattern as
`mcp_fixture_server`): a file with `TODO_BAD` comes back as a bounded block, a clean file is silent, a
second change reuses the running server, a silent server yields silence rather than a hang, and a
refusing server is recorded as broken exactly once.

Two test-caught bugs, both mine: the framing test asserted a hardcoded body length that serde_json's
key ordering made wrong (now computed from the frame itself, so it cannot agree with a wrong
constant), and two tool tests tripped the M65 stale-read guard by overwriting a seeded file the
model had never read — the guard was right and the premises were wrong.

Gate: `scripts/gate.sh m79b` — fmt clean, clippy zero warnings, **765 tests, 0 failed**, demo
completes, `real`-feature build compiles (26 new tests over M78's 739). The first run failed on
clippy alone with every test passing (a collapsible `if`, then a `Default::default()` field
assignment), which is the third milestone in a row where the `-D warnings` hardening caught something
the compiler did not.
