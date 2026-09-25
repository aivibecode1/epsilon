# M137 — several SSE frames in one HTTP chunk no longer lose all but the first

**Status:** implemented; verification below
**Closes:** F03 (`audit/02-defect-ledger.md`)
**Evidence:** `audit/evidence/gate-m137.log`, `audit/evidence/m137-negative-control.txt`

## The defect

A provider is free to deliver a whole short response in a single TCP chunk. When it
did, Epsilon returned the first frame's worth of text and silently dropped the rest
— then reported the truncated answer to the user as a **success**, with a terminal
`Done`, no error, and no warning.

Two lines conspired:

1. `WireStream::ingest` drained complete lines out of `line_buf` but `return`ed from
   *inside* the loop as soon as one line produced an event. Every later complete line
   in that same chunk stayed sitting in `line_buf`.
2. `next()` checked `queued` and then awaited the next network chunk. It never
   re-examined `line_buf`.

So the stranded frames were not delayed — they were unreachable. When the body then
ended, the end-of-body path found no queued events, synthesised a `Done` from whatever
text it had, and the turn completed successfully with a two-thirds answer.

The control in `audit/evidence/m137-negative-control.txt` pins the harm in one line:

    left: "one "
   right: "one two three"

## Why it survived so long

`WireStream` held a concrete `reqwest::Response`, so no test could put more than one
SSE frame in one chunk without a real socket. Every wire test in the tree drove the
*codecs* directly — `SseDecoder` and the per-provider `on_event` — and those were
never wrong. The bug lived in the one layer that decides how bytes are grouped, and
that layer had no test at all. A test suite can be large and still have a hole shaped
exactly like the thing nobody imagined testing.

## The fix

Two parts, and the second is the part that matters for the future.

**`ingest` drains the whole chunk.** Queue every event, then hand back one. `next()`
already drains `queued` before it awaits, so the rest come out without touching the
network. This is a four-line change and the whole behavioural fix.

**`inner` became a boxed byte stream.** `Response::bytes_stream()` is exactly
`Stream<Item = Result<Bytes, Error>>`, so production pays one `.boxed()` and gains a
seam. The pre-fix `chunk()` contract — `Option<Result<..>>`, where `None` means "body
finished" — is reproduced exactly by mapping each chunk to `Some(..)` and chaining a
single trailing `None`. Every match arm below the call site is therefore byte-for-byte
what it was; no arm was re-keyed, and the end-of-body path (the unterminated-frame
flush, the `decoder.feed("")` fallback, the honest failure) is untouched.

**Key decisions**

- *Keep the `Option` wrapper rather than re-keying the arms.* `bytes_stream()` stops
  instead of yielding a `None`. Chaining one trailing `None` is two lines and leaves
  120 lines of subtle end-of-body handling provably unchanged. Re-keying would have
  meant touching the DeepSeek unterminated-frame path by hand, for no behavioural gain.
- *Add `bytes` as a declared dependency.* It was already compiled into the binary via
  reqwest — the same `1.12.1` in `Cargo.lock`, so this adds no new compilation. Rust
  will not let a crate name a transitive dependency, and the alternative was either an
  ADR for a fresh crate or a pile of inference tricks. This is the "declare, do not
  smuggle" case, recorded as an ADR-017 amendment.
- *Did not touch `next()`'s ordering.* The fix belongs in `ingest`, where the knowledge
  is. `next()` already drains `queued` first; that behaviour was correct and became
  sufficient once `ingest` filled the queue.

## Tests

Three, all in `providers::http_stream_tests` — the module that could not exist before
this change.

- `every_frame_in_one_chunk_is_delivered_not_just_the_first` — the defect. Three
  frames, one chunk, order preserved, terminal frame still arrives.
- `one_frame_per_chunk_gives_the_same_answer_as_one_big_chunk` — the pair. Same
  bytes, different framing, identical result. Without this, a change that only broke
  the single-chunk path would pass.
- `a_frame_split_mid_line_still_reassembles` — the opposite failure. A line split
  across two chunks must still decode, so a "fix" that dropped reassembly fails here.

`idle_limit` is `Duration::ZERO` in the test harness on purpose: these tests are about
framing, and a watchdog could only convert a framing bug into a flaky one.

## Collateral finding — 38 provider tests were never collected

`cargo test -p epsilon-llm` collects **71** tests. With `--features real` it collects
**109**. The 38 missing ones are the entire provider-stream surface, because
`lib.rs` gates `pub mod providers` on `#[cfg(feature = "real")]`.

`real` is a default feature, so the workspace battery does run them. But a per-crate
invocation without the flag silently drops a third of the crate's tests and still
prints `test result: ok`. The three new tests would have been dead code under a
`cargo test -p epsilon-llm` habit. Every command in this milestone's evidence passes
`--features real` explicitly, and the trap is recorded in
`agents/memory/next-hardening.md`.

Two pre-existing tests (`deepseek_text_chunk_produces_done_with_nonempty_content`,
`deepseek_wire_synthesizes_start_before_text`) were in that never-collected set. Both
pass.

## Verification

Battery **`m137`**, log at `audit/evidence/gate-m137.log`; negative control at
`audit/evidence/m137-negative-control.txt`.

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | 1191 passed, 0 failed, 1 ignored (64 binaries) |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0 |
| `cargo check -p epsilon-cli --features real` | exit 0 |

The suite delta from M136's 1188 is **+3**: exactly the three new tests. The 38
previously-uncollected provider tests were already counted in the workspace total,
because the workspace build enables `real`.

## Limits — what this slice does not do

- **Not tested against a real provider.** Every case is a local `stream::iter`. A
  genuine HTTP response with real chunk boundaries, TLS framing, and proxy
  behaviour is unexercised; a second defect that only appears with a real socket
  would not be caught here.
- **Only the OpenAI-compatible wire is driven.** Anthropic, Bedrock, Google, Mistral
  and the Responses API all share `WireStream` and so benefit from the `ingest` fix,
  but each codec's own `on_event` is still only tested through its own unit tests.
- **`bytes_stream` error timing is unchanged.** A mid-body transport error still takes
  the `Some(Err(e))` arm that synthesises a `Done` from partial content. Correct
  enough, and deliberately left alone.
- **The `idle_limit` watchdog is untouched** and still operates per-chunk, not
  per-frame: a chunk containing a slow frame is still bounded by the same limit.

## What nothing exercises

- **Real chunk boundaries.** The tests choose their own framing from a `Vec`; the
  actual TCP segmentation that triggered this defect in the field is not reproduced.
  A loopback HTTP server would close that gap and is the obvious next test.
- **Tool-call frames split across chunks.** Only text deltas are exercised. A
  `tool_calls` delta is assembled across several frames by the wire codecs, which is
  precisely the multi-frame-per-chunk case — and it is untested end-to-end.
- **The interaction with `retry_stream`.** A stream that died mid-answer is retried
  transparently; that path sees a `WireStream` and is not exercised with a
  multi-frame chunk.
- **`finish_reason` in the final frame of a single chunk.** The synthesised-`Done`
  path in the end-of-body arm is exercised by the pre-existing DeepSeek tests, but
  not specifically with several frames in the last chunk.
