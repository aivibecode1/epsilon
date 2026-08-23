# M9a — Extension event forwarding + print mode

Date: 2026-08-23 · Status: partial M9 delivery (TUI/serve/rmcp remain for M9b)

## What was built

- **Event forwarding (closes known-gap #1)**: `ExtHost::send_event` pushes
  `{"type":"event","event":{...AgentEvent}}` frames to child processes, filtered by the
  manifest's `events` list (`["*"]` = all). Fire-and-forget by contract — event delivery can
  never break a run. Children report receipts via `{"type":"log"}` frames, collected by the
  host (`logs()`) for diagnostics and tests.
- **Print mode**: `epsilon -p "task"` — headless single-prompt run over the current directory
  with REAL tools, default ruleset, auto-approved asks (TUI dialog arrives with M9b), durable
  session under `.epsilon/sessions/cli.jsonl`, final assistant text to stdout. `--script`
  accepts faux-script JSON so `-p` stays offline/reproducible by default.
- Protocol addition documented in module docs; fixture bin extended to count/log events.

## Test-caught notes

- Event-counter numbering is per-child global (frames, not per-type) — test expectation fixed,
  no code change.
- tokio 1.53: `ChildStdout` needs async traits (`AsyncBufReadExt`) in tests too; sync
  `std::io::Write` imports don't cover tokio stdin.

## Known gaps remaining (unchanged)

- Windows confinement (Job Objects + ACL runner) → M10 acceptance criteria.
- rmcp stdio transport → M9b.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 83 passed (2 new: forwarding + filter)
cargo run -q -p epsilon-cli             # demo unchanged
cargo run -q -p epsilon-cli -- -p "smoke task"   # print mode completes, exit 0
```
