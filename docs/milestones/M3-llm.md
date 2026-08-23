# M3 — epsilon-llm (StreamFn contract, faux provider, genai adapter, retry)

Date: 2026-08-23 · Status: complete

## What was built

| Module | Contents |
|---|---|
| `stream_fn.rs` | `ModelRef`, `LlmContext`, object-safe `EventStream` trait, `StreamFn` trait with pi's never-fail contract, **`PanicGuard`** blanket wrapper (panics become `Done{Error}` streams) |
| `faux.rs` | Script-driven deterministic provider: `FauxStep::{Assistant, ToolCalls, Truncated, Error}` from JSON or code; records every request for assertions; emits realistic chunked deltas |
| `retry.rs` | Pure failure classification (`RateLimit/Server/Network/Auth/Other`) + exponential backoff (500ms base, decorrelated jitter, 30s cap) honoring Retry-After |
| `catalog.rs` | Static model table with explicit capability fields — no id-substring sniffing anywhere |
| `genai_adapter.rs` | Real providers behind `--features real`: maps our types to/from genai 0.6.5, assembles the authoritative final message from `StreamEnd.captured_*` |

12 unit tests in the crate; workspace total now 27.

## Key decisions made while coding

1. **Object safety over RPITIT.** `EventStream::next()` uses `#[async_trait]` so frontends can
   hold `Box<dyn EventStream>`; native async-in-trait would have made dynamic dispatch
   impossible for our registry-shaped consumers.
2. **Authoritative-Done semantics** (carried over from the M2 lesson): real adapters enable
   genai capture flags and build the final message from `captured_content/usage/stop_reason`;
   streamed deltas exist only for live UI. This removed an entire class of "partial assembly"
   bugs before they could exist.
3. **Tool-call chunks buffered, not diffed**: providers emit partial-JSON fragments in
   incompatible shapes; rather than diffing fragments per provider, we emit synthetic
   Start/Delta events at stream end. UI sees identical event shapes for every provider.
4. **Auth failures never retried** — retrying 401s leaks attempts and can lock accounts;
   classification defaults unknown errors to no-retry (fail fast beats fail hammering).

## Gotchas hit (learning notes)

- **genai feature names verified before pinning**: guessed `rustls` was wrong; actual feature is
  `rustls-tls`. Reading the crate's own Cargo.toml via GitHub raw took 30 seconds and saved a
  broken lockfile.
- API surface verified against v0.6.5 sources (`ChatRequest{system,messages}`,
  `ToolCall{call_id,fn_name,fn_arguments}`, `ToolResponse::new(call_id, content)`,
  `exec_chat_stream(model: impl Into<ModelSpec>, req, Option<&ChatOptions>)`) instead of memory.
- Two test-caught defects on first run:
  1. jitter test asserted exact cap equality — wrong expectation (jitter lives below cap);
  2. **real bug**: oversized Retry-After fell through to jitter instead of clamping. The pure-
     function design made this a one-line, fully-tested fix.

## Verification

```
cargo fmt --all -- --check                 # clean
cargo clippy --workspace --all-targets     # zero warnings
cargo test -p epsilon-llm --features real  # 12 passed
cargo test --workspace                     # 27 passed
```

## Next

M4 fills `epsilon-session`: JSONL tree store, shadow views, leases, crash repair, replay CLI.
