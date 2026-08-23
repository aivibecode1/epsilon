# M9b — ratatui TUI, serve mode (SSE), rmcp stdio transport

Date: 2026-08-23 · Status: completes M9 (with M9a). Remaining M9 criterion "serve mode
streams SSE" closed here; M10 owns Windows confinement + release packaging.

## What was built

- **TUI (`epsilon-tui`)** — `--tui ["task"]`. A pure event reducer
  (`state::UiState`, unit-tested without a pty) plus a thin terminal runtime
  (ratatui 0.30.2 / crossterm 0.29, `event-stream`). Scrolling transcript with live
  streaming block (`MessageUpdate` replaces, `MessageEnd` commits), one-line tool cards,
  thinking lines, status footer with token counters. Typed input submits a prompt when
  idle and **steers** mid-run (pi behavior); ↑/↓ history; PageUp/PageDown scroll.
  Permission asks render as a dialog: y/a/n/e (deny-with-guidance), Esc denies.
- **Serve mode (`epsilon-server`)** — `--serve [--port N] ["task"]`. axum 0.8.9:
  `GET /events` streams every `AgentEvent` as SSE data frames (lag reported as `lagged`
  events instead of silent gaps); `POST /prompt|/steer|/followup`; `GET /approvals` +
  `POST /approvals/{id}` answer permission asks remotely with the same
  `PermissionDecision` payloads the contracts type already defines. Single-run invariant
  surfaces as HTTP `409`.
- **Shared approval channel (`epsilon-safety::ChannelApprovalUi`)** — the TUI dialog and
  the REST endpoint resolve the SAME parked asks, out-of-order, by `request_id`; a
  dropped/unanswered ask fails closed to `Deny`. One mechanism, three consumers
  (TUI, server, tests).
- **rmcp stdio transport (`epsilon-mcp::StdioConnector`)** — closes the M8 staging
  amendment behind the unchanged `Connector` seam: real child process, real MCP
  initialize handshake, `tools/list`, `tools/call` (text + structured-content fallback),
  graceful shutdown. Integration tests spawn a REAL rmcp server binary
  (`mcp-fixture-server`) over real stdio pipes — offline, deterministic.
- **CLI** — mode dispatch now covers demo | `-p` | `--tui` | `--serve`; `--script` drives
  any mode offline; repeatable `--mcp name=cmd[:arg,arg]` attaches stdio servers to any
  run (drive-letter colon handled for Windows paths). Composition lives in ONE place
  (`build_runtime`): every frontend gets real tools + ruleset gate + session store.

## Key decisions

- **Frontends own no runtime composition.** The CLI builds `Arc<Agent>` once and hands it
  to `epsilon_tui::run` / `epsilon_server::router`. This is ADR-011's "one session
  runtime beneath all" made structural, not aspirational.
- **Approvals are channel-based, not event-scraped.** The loop's `PermissionDecided`
  events carry empty request ids (M6-era quirk), so frontends discover asks through the
  approval queue itself; the server exposes it over REST rather than scraping events.
  Fixing exec's event emission is deferred until a consumer actually needs it.
- **Fixture server via `required-features`.** The fixture bin needs rmcp's
  `server`+`transport-io` features that production epsilon-mcp deliberately doesn't ship;
  `[[bin]] required-features = ["rmcp/server", …]` + dev-dependency feature union means
  plain `cargo test --workspace` builds and runs it while `cargo build -p epsilon-mcp`
  stays lean.
- **MSRV 1.85 → 1.88** (ratatui 0.30 and rmcp 3.1 both declare 1.88).

## Test-caught bugs / gotchas

- A first-draft `ChannelApprovalUi` test deadlocked: calling `ui.ask(...).await` inline
  blocks the test main on its own responder. All asks must be spawned; poll
  `pending()` before resolving.
- SSE smoke test false-negative: my termination check compared `buf.windows(5)` against
  the 9-byte needle `"agent_end"` — windows size must equal needle length. The stream
  itself was correct the whole time (diagnostic dump showed every frame).
- Ordering race in the same test: posting `/prompt` before awaiting the SSE handshake let
  the instant faux run finish BEFORE the events handler subscribed (broadcast drops with
  no receiver). Waiting for handshake headers first proves subscription.
- axum 0.8 path params use `{id}` syntax (not `:id`).
- rmcp's `ClientInfo`/`Implementation`/`ServerInfo` are `#[non_exhaustive]`:
  default-construct then mutate fields.
- New toolchain clippy fires `collapsible_if` on pre-existing contracts/session/core/safety
  code — fixed at the source (let-chains), no gate weakened.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 98 passed (was 83; +15)
cargo run -q -p epsilon-cli             # demo unchanged
cargo run -q -p epsilon-cli -- -p "smoke task"    # exit 0
--serve smoke: healthz/approvals answered over a real socket from a spawned binary
--tui: enters interactive loop (manual-key verification by design; reducer unit-tested)
```

## Known gaps remaining

- rpc(stdio) mode → editor-integration milestone (route types already single-source).
- Windows confinement (Job Objects + ACL runner), release packaging → M10 acceptance
  criteria (README roadmap).
