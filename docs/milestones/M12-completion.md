# M12 — Completion pass: config layer, reliability stack, all four frontends

Date: 2026-08-23 · Status: shipped. Closes the deferred ledger except items with
permanent rationale (ACL runner) or external dependencies (Linux runtime verification).

## What was built

### Reliability (ADR-010 amendment)
- **`Retrying` StreamFn executor**: wraps any provider; retries *pre-stream* failures
  only (peek-first-event discipline: deltas never buffered or duplicated), classified
  via the existing `retry::classify`, decorrelated-jitter backoff. `on_retry` hook
  exposes `RetryNotice{model, kind, attempt, delay, detail}` for UI notices / durable
  entries. Tests cover retry-then-succeed, exhaustion surfacing verbatim,
  auth-never-retry, and passthrough fidelity of a delta stream.
  Honest scope note: full `(turn, step)` ledger keying stays open — the adapter layer
  cannot see loop positions; recorded in ADR-010 rather than faked.
- **Run-scoped cancellation**: `Agent` holds one token per run;
  `abort_current_run()` cancels it; `exec.rs` distributes it through every
  `ToolContext`; both bash paths honor it (confined: dropping the run future drops the
  job guard → whole tree terminated; unconfined: explicit cancel branch). The loop
  checks at each turn boundary and ends sticky-`Aborted`. TUI Esc during a run aborts;
  idle Esc still quits.
- **Cost display**: footer renders session cost from catalog prices (documented
  placeholders) in integer micro-USD → formatted dollars; custom/unknown ids omit cost.

### Sessions
- **`--resume <path>` / `-c/--continue`** (`--continue` = newest `*.jsonl` under
  `.epsilon/sessions/`): restores the store's SHADOWED model view into runtime history
  pre-run, so resumed sessions continue post-compaction without re-persisting restored
  messages. Verified live (40-entry transcript).

### Project config layer (ADR-012 amendment)
- `.epsilon/config.toml`: `[model] id`, `[[rules]]` (stacked as a `"project"` engine
  layer over defaults — last-match-wins with any-deny-wins preserved),
  `[[extensions]] dir`, `[[mcp_servers]] name/command/args`. Malformed entries are
  reported and skipped, never fatal.
- **Trust hash**: first use records SHA-256 at `.epsilon/trust.json`; later content
  changes print a loud review banner. Verified live including the change case.
- Model precedence extended: flag > project > global saved > default.

### Frontends complete (ADR-011 amendment)
- **rpc(stdio)** (`--rpc`): NDJSON requests (`prompt/steer/followup/approvals/approve/
  ping`) in; every AgentEvent streamed out as frames. Same runtime + approval channel
  as serve — transport is the only difference.
- **OpenAPI 3.1** at `GET /openapi.json`: hand-maintained spec with a drift-guard test
  asserting spec paths == registered routes (7/7). Codegen deliberately skipped while
  the surface is this small.

## Test-caught notes

- PowerShell single-quoted strings don't expand backtick-n: two rounds of corrupted
  Rust from bulk string surgery before switching entirely to proper edit tooling. The
  AGENTS.md Windows note earns its place every milestone.
- async-trait methods return futures — calling `.stream(..)` without `.await` produces
  `Pin<Box<Future>>` type errors that look baffling until you squint; three call sites
  fixed mechanically.
- Duplicate resolution blocks after inserting the project-aware one: the stale block
  shadowed the new binding (compiler flagged "unused variable" on the LIVE code path —
  an inverted signal worth remembering).
- toml 1.x `Entry::or_insert_with` takes a closure; `or_insert(Value::Table(...))` is
  the value-shaped form.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 129 passed (was 120; +9)
cargo run -q -p epsilon-cli             # demo unchanged
cargo run -q -p epsilon-cli -- -p smoke # exit 0
--continue                              # resumed 40 entries (live)
project .epsilon/config.toml            # trust hash + change WARNING (live)
--rpc                                   # ping/prompt over stdio (live)
GET /openapi.json                       # 200, served (live)
```

## Remaining ledger (final)

- Linux confinement runtime verification — needs a Unix host/CI; code ships cfg-gated.
- Windows ACL restricted-token runner — permanently deferred with rationale (ADR-005).
- Live per-provider catalog fetch, favorites/recents, credential store — future work
  beyond the planned scope (catalog is data; additive when wanted).
- Interactive trust re-prompt (currently banner-only) — TUI polish candidate.
