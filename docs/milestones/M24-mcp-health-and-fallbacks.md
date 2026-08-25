# M24 — MCP health probes + provider fallback chains

Closes DIRECTION-02 (MCP TTL+grace probes) and DIRECTION-03 (fallback chains),
the last two open items from `reports/research-pass-2-2026-08.md`.

## What shipped

### 1. MCP health probes (DIRECTION-02, from Hermes)

The problem: when an MCP server process dies mid-session, the next tool call
either hangs on a broken pipe or fails with an opaque transport error from deep
inside rmcp. The user sees garbage; the model sees an error it cannot act on.

The fix follows Hermes' availability-probe pattern, adapted to our `Connector`
seam:

- `Connector` gains `async fn is_healthy(&self) -> Result<(), String>` with a
  default "trust by default" implementation (return `Ok(())`), so custom
  connectors only implement it when they can actually check something.
- The stdio connector overrides it: rmcp's `RunningService::is_closed()` is a
  cheap local check for a dead child process.
- `McpBridgeTool::execute` calls the probe before every tool call, cached with
  a 30-second TTL (`HEALTH_TTL_MS`) so healthy servers pay one atomic load per
  call, not one round-trip.
- A failed call resets the cached probe timestamp, so the next call re-probes
  first (the grace half of TTL+grace).

Failure message names the remedy: "mcp server process is not running (exited
or crashed); restart the session or re-attach with --mcp".

Test seam: `InMemoryConnector` grows `mark_dead()`; the bridge test pins both
the healthy path and the fast-fail path (`bridge_health_probe_fails_fast_when_server_dies`).

### 2. Provider fallback chains (DIRECTION-03, from Hermes)

Hermes walks a configured fallback chain when its error classifier marks a
failure retryable, restoring the primary next turn. We ported the semantics
onto our never-fail `StreamFn` seam:

```sh
epsilon --model claude-sonnet-4-5 --fallback gpt-5.4 --fallback deepseek-chat
```

New module `epsilon-llm/src/fallback.rs`, gated behind `--features real` like
the rest of the direct-provider stack.

Semantics:

- Candidates are tried in order within one turn. The requested model leads;
  configured fallbacks follow.
- Only RETRYABLE failure classes walk the chain: rate limit, transient server,
  network trouble — exactly the classes `retry::classify` already marks
  retryable. Auth failures and unknown models stop the walk immediately:
  switching providers cannot fix a bad key or a typo.
- Per-turn primary restoration falls out of statelessness: every call starts at
  the chain head, so once the primary recovers it is used again automatically.
  No sticky state to reset.
- Fallback applies only when the attempt failed BEFORE streaming started
  (pre-flight in `providers::try_stream_direct`). Once events flow, the stream
  is committed; we never switch mid-stream.

CLI wiring: repeatable `--fallback <model>` flag composes
`FallbackProvider` instead of bare `DirectProvider` and logs the chain in the
startup diagnostics ("fallbacks: gpt-5.4 -> deepseek-chat").

## Key decisions

1. **Probe lives behind the existing Connector trait**, not a new service layer.
   One method, default-trusting; stdio overrides where a check is possible.
   Cost on healthy servers: one relaxed atomic load per call.
2. **Reuse `retry::classify` as the chain's error classifier** rather than
   inventing a second taxonomy. One classification drives both the in-turn
   retry executor and cross-candidate walking; they cannot drift apart.
3. **No mid-stream switching.** A half-streamed answer spliced onto another
   provider would corrupt transcripts and usage accounting. Pre-flight failure
   only.
4. **Auth resolution happens at construction** via the standard precedence
   (`resolve_api_key`: stored key wins over env). Candidates without keys are
   skipped honestly; `usable_count()` reports reality.

## Test-caught issues

- First draft used an env-var TTL override; rejected because `env::set_var` is
  unsafe under edition 2024 (and forbidden workspace-wide). Replaced with a
  fresh-registry construction in the test, which models TTL expiry without
  touching process state.
- Clippy caught a `format!` around a literal in the new stdio error path and
  an unused field after simplification; both fixed same commit.
- rustfmt 1.97 (local) formats the multi-line error literal differently than
  CI's pinned tool; CI failed the formatting gate once and the rustfmt output
  was committed as-is (lesson reinforced: run `cargo fmt` right before push).

## Verification

```
cargo fmt --all -- --check          clean
cargo clippy (both configs)         0 warnings
cargo test --workspace              241 passed, 0 failed
cargo test --workspace --features real   255 passed, 0 failed
cargo run -q -p epsilon-cli -- --demo    completes
CI                                  green (windows-full, linux-confinement, msrv)
```

## Attribution

Both features are ports of Hermes Agent patterns
(`chat_completion_helpers.try_activate_fallback` and the check_fn availability
probes), documented in `reports/harness-analysis.md` §3 and reimplemented from
scratch on Epsilon's seams. Full credit table: README "Credits and references".
