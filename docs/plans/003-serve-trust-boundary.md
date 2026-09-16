# Plan 003: Serve-mode trust boundary — bearer token, Host check, print-mode auto-approve opt-in

> **Drift check**: `git diff --stat 64e927f..HEAD -- crates/epsilon-server/ crates/epsilon-cli/src/main.rs`

## Status

- **Priority**: P1 · **Effort**: M · **Risk**: LOW
- **Depends on**: none · **Category**: security
- **Planned at**: commit `64e927f`, 2026-08-24

## Why this matters

SECURITY-01: the serve-mode HTTP API (prompt/steer/approve endpoints) has no auth.
Any local process — or a webpage via cross-origin POST / DNS rebinding — can start
runs AND resolve permission asks as allow, collapsing the entire ask-first safety
posture. SECURITY-02: print mode (`-p`) wires `AutoApprove`, so "bash asks by
default" silently doesn't hold in headless runs with real tools attached.

## Current state

- `crates/epsilon-server/src/lib.rs:97-111` router: no middleware, no auth layer.
- `crates/epsilon-cli/src/main.rs:859` binds localhost only (mitigates remote, not
  local processes or browsers).
- `crates/epsilon-cli/src/main.rs:728` print mode uses
  `Arc::new(epsilon_safety::AutoApprove)` unconditionally.

## Commands

fmt --check; clippy --workspace --all-targets; cargo test --workspace.

## Scope

**In scope**: epsilon-server router/middleware, cli main.rs approval wiring + flags,
server tests. **Out of scope**: TUI/rpc modes, TLS, multi-user auth concepts.

## Steps

1. Serve mode: generate a random per-boot token (`uuid`-style from std rand is NOT
   available; use a simple OS-random hex via `std::fs::File::open("/dev/urandom")`
   is unix-only — instead reuse an existing dep or implement SHA-256 of
   process-id+time via the in-tree `sha2` crate already used for trust hashes).
   Print it once to stderr at startup; require header `authorization: Bearer <t>`
   on every route except `GET /healthz` via a small axum middleware fn.
2. Add Host-header allow-list middleware: reject when Host is neither
   `127.0.0.1:<port>` nor `localhost:<port>` (defeats DNS rebinding). Reject
   requests carrying an `Origin` header that isn't `null`/localhost.
3. Print mode: default to auto-DENY on asks (new `AutoDeny`-style ApprovalUi if not
   present — check epsilon-safety for existing AutoDeny). Add explicit flag
   `--yes` ("auto-approve asks; unattended") which selects AutoApprove and prints a
   loud stderr banner. Update USAGE text.
4. Tests: server test asserting 401 without token and 200 with it (pattern: existing
   SSE smoke test); Host-rebind rejection test; CLI-level test that `-p` with a
   gated bash script denies by default and `--yes` approves.

## Done criteria

- [ ] No server route mutates state without valid bearer token
- [ ] `-p` run with bash tool no longer executes ask-gated command without `--yes`

## STOP conditions

- Existing tests depend on unauthenticated access in ways more than mechanical.
- sha2-based token generation unavailable in server crate deps (report; do not add
  new dependencies without instruction).

## Maintenance notes

Document serve mode as a TRUST BOUNDARY in README when this lands.
