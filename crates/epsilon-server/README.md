# epsilon-server

The HTTP/SSE frontend. Axum routes over an agent it did not build.

## What this crate is

It exposes one session runtime over HTTP: a small REST surface for prompts, steering and
permission decisions, plus an SSE stream of agent events. The `Arc<Agent>` it serves is handed
to it by the CLI.

It does **not** compose the agent, and it is deliberately agnostic about which provider,
tools or ruleset are in play — those choices are settled before this crate is called. It also
does not own approval *policy*: asks park in `epsilon_safety::ChannelApprovalUi`, the same
channel the terminal dialog drives.

## How it connects

Workspace dependencies: `epsilon-contracts`, `epsilon-core`, `epsilon-llm`,
`epsilon-safety`, `epsilon-tools`. External: axum, futures-util, serde, serde_json, sha2,
tokio. Dev: `http-body-util`, `tower`. One crate depends on it, `epsilon-cli`; the edge does
not run the other way.

## Entry points

| Item | Where |
|---|---|
| `ServeState` — the handle every route operates on | `src/lib.rs:33` |
| `ServeState::new` — agent plus approvals, no auth token | `src/lib.rs:47` |
| `ServeState::with_boot_token` — the production constructor | `src/lib.rs:58` |
| `router(state) -> Router` — the only place routes are registered | `src/lib.rs:186` |
| `PromptBody` / `AbortBody` / `TextBody` — request bodies | `src/lib.rs:94`, `:105`, `:141` |
| `openapi::spec()` — the document served at `/openapi.json` | `src/openapi.rs:12` |

The route list itself lives in the private `route_table()` (`src/lib.rs:166`), because
`router()` and the OpenAPI drift test both read it.

## How to run and test it

This crate ships no binary. The CLI starts it:

```sh
cargo run -q -p epsilon-cli -- --serve              # default port 8787
cargo run -q -p epsilon-cli -- --serve --port 9000
cargo test -p epsilon-server                        # tests are inline in src/lib.rs
```

## Things worth knowing

- Serve mode is a trust boundary (`src/lib.rs:199`). Every route except `/healthz` and
  `/openapi.json` requires a per-boot bearer token *and* a loopback `Host` header. The CLI
  prints the token once on stderr.
- `ServeState::new` leaves `auth_token` as `None`, which disables the check. That path is for
  tests; production goes through `with_boot_token`.
- A route added to the table but not to the OpenAPI document — or the reverse — fails a test,
  so the two cannot drift.
- `epsilon-server` cannot depend on `epsilon-cli`, so the shared session payload is built
  through `Agent::session_state` in `epsilon-contracts` (`src/lib.rs:134`).
- Rules: [`AGENTS.md`](../../AGENTS.md),
  [`docs/design/epsilon-decisions.md`](../../docs/design/epsilon-decisions.md).