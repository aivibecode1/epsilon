# M60 — Retries that actually run, and a stall watchdog

The goal says *usable solid harness*, so this round audited the failure paths
rather than the feature list. The finding was uncomfortable: **the retry policy
was fully implemented, unit-tested, and never engaged.** `retry.rs`
(classification + decorrelated-jitter backoff) and `retry_stream.rs` (the
`Retrying` executor) had tests; `grep` for `Retrying` across the production
crates returned only the re-export. Meanwhile `crates/epsilon-llm/src/http.rs`
said out loud: *"no overall timeout (streams legitimately run for minutes)"* —
so a provider that accepted the connection and then went quiet left the turn
pending forever, and a transient 429/503 failed it outright.

Two defects, one milestone:

## 1. A stall watchdog on the streaming read

`WireStream::next` awaited `self.inner.chunk()` with no ceiling. It now wraps the
read in an idle limit (`STREAM_IDLE_LIMIT`, 120 s, override
`EPSILON_STREAM_IDLE_SECS`, `0` disables) — **silence** is the signal, not
elapsed time, so a legitimately long stream is never cut off.

Silence produces a terminal message via `HttpEventStream::stall_message`, whose
salvage rules matter:

- **Nothing produced** → a plain error. The message reads *"provider stream timed
  out: no data for 120s"*; the word "timed out" is load-bearing (see gotcha 1).
- **Prose produced** → the prose is kept and the turn is flagged `Error`: the
  model's words are not thrown away, and a cut-off answer is never presented as
  complete.
- **A tool call in flight** → the call is dropped. Its arguments may be
  half-written JSON, and executing a half-written `bash` call is worse than
  failing the turn. Any prose is preserved.

## 2. The retry policy, engaged

`Retrying` now wraps the real stream on **both** frontends:

- `Agent::retrying_real_stream()` — one constructor used by the M32d hot-swap, so
  a login that lands mid-session gets the policy too.
- The CLI's `DirectProvider` **and** `FallbackProvider` paths (the fallback chain
  is retried as a unit).

Scheduled waits surface as a new `AgentEvent::Notice`, because a pause nobody
explains is indistinguishable from a hang:

- TUI: the notice becomes the status line *and* a dim `⟳ …` transcript line.
- CLI: `[retry] …` on **stderr**, keeping stdout exactly the transcript for `-p`.

The retry policy itself was extended to cover the case this milestone is about:
**a live stream that dies before emitting anything is retryable.** The executor
now tracks whether any *visible* output went out (`TextDelta`, `ThinkingDelta`,
tool-call frames — `Start` does not count, it only opens the message). While
nothing is visible, a retryable failure re-runs the call, swallowing the fresh
attempt's duplicate `Start` so the caller sees exactly one. Once a delta has been
forwarded, failures pass through untouched: re-running after the user has watched
half an answer would duplicate or contradict it.

## Gotchas

1. **The message-shaped classifier and the message text disagreed.** `classify()`
   matched `"timeout"`; the watchdog said `"timed out"`. A stalled stream would
   therefore have classified as `Other` → **never retried** — the feature would
   have been dead in exactly the case it exists for. Caught by the first test run
   (the recovery test asserted the classification). Fixed in the class, not the
   message alone: `classify` now also matches `"timed out"` (reqwest says
   "operation timed out", so this was always a hole), and the fixture pins the
   classification end to end.
2. Unused-variant hygiene: `AgentEvent` is matched exhaustively in
   `epsilon-cli/src/runtime.rs`'s test tag mapper and in the TUI's `apply`; adding
   `Notice` required both (the TUI's arm is where the transcript line comes from).
3. `HttpEventStream::new` had to become public so wire fixtures can drive the
   watchdog's salvage rules directly — the alternative was a local HTTP server
   emitting a partial SSE frame and then stalling, which tests tokio's timer
   rather than our decision.
4. Rust test-authoring: a `#[test]` inserted at a fuzzy anchor landed *inside*
   another test body and produced `unnameable_test_items` — read the enclosing
   block before trusting an insertion point.

## Verification

- `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets`
  zero diagnostics.
- `cargo test --workspace`: **448 passed, 0 failed** (441 before: +3 executor
  scenarios in `retry_stream`, +3 end-to-end retry-path core scenarios, +1 TUI
  notice test).
- `--features real` lane: **131 passed, 0 failed** (125 before: +3 stall salvage
  fixtures, +3 executor cases).
- `cargo check -p epsilon-cli --features real` clean.
- `cargo run -q -p epsilon-cli -- --demo` completes.
