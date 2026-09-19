# M62 — A failure never discards what you already watched arrive

Third round of the solidity pass (M60 transport retries, M61 durability leases).
This one asked a narrow question: when a stream dies **after** it has produced
output, does the partial text survive into the transcript, the durable session
file, and the resumed session — or does the failure take it with it?

## What the audit found

Two places threw it away.

1. **The transport.** `WireStream::next`'s read-failure arm built a bare
   `error_message("stream read failed: …")` — a socket that dies after 300 tokens
   produced a transcript entry with the error and **no text**, while the user had
   sat there watching those tokens appear. (M60's watchdog had a salvage rule;
   nothing else did.)
2. **The wires' error frames.** OpenAI-compat, Mistral, Anthropic, and the
   OpenAI-Responses and Bedrock adapters all did the same for provider-sent error
   frames: `error_message(...)`, accumulator ignored.
3. **The loop.** `stream_one_turn`'s "stream ended with no `Done`" arm returned
   `Err(RunError::Internal(…))` and dropped the folder's partial the same way. The
   abort path had always salvaged (`aborted_message`); the failure path never did.
4. **The TUI**, found while verifying the above: the failed-reply marker rendered
   only when the message had *no* text (`… && text_empty && …`), so a salvaged
   partial would have been displayed with no indication that it was cut off —
   reading as a complete answer. The marker now renders whenever a failure carries
   a detail, worded `error: … — the reply above is partial` when text arrived.

The asymmetry is the bug: abort kept your text, a dropped connection did not.

## The rule, now stated once and enforced everywhere

*Whatever the user already watched arrive stays.* Losing visible output is worse
than an ugly transcript, and a resumed session rebuilding context from the store
needs the partial reasoning rather than a hole where it used to be. Two honesty
constraints ride along, so this can never become a way to fake success:

- the turn is flagged **`StopReason::Error`** with an `error_message` that
  explains itself — never presented as a complete answer;
- **tool calls in flight are dropped** (their arguments may be half-written JSON;
  executing a half-written `bash` call is worse than failing the turn), though any
  prose beside them is kept.

Implementation: one salvage decision, `HttpEventStream::failure_message(detail)`
on the wire side (the stall watchdog's `stall_message` is now a caller with its
own wording — M60's rule generalized rather than duplicated), and
`Agent::failure_message(partial, stop, detail)` on the loop side, from which
`aborted_message` is now a one-line wrapper. Every mid-stream failure path goes
through one of them; when nothing has arrived they degrade to a plain error
message, so pre-stream failures are unchanged.

Sites converted: the transport read failure, the OpenAI-compat/Mistral bare-JSON
error frames, the Anthropic `error` event, both OpenAI-Responses error shapes, the
Bedrock stream exceptions, and the loop's no-terminal-frame arm.

## Verification

- `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets`
  zero diagnostics.
- `cargo test --workspace`: **459 passed, 0 failed** (457 before: +1 loop
  scenario, +1 TUI state test).
- `--features real` lane: **133 passed, 0 failed** (+2 wire fixtures: an
  OpenAI-compat error frame after a content delta keeps the text; an Anthropic
  `error` event after a thinking delta keeps the thinking).
- `cargo run -q -p epsilon-cli -- --demo` completes.
- The new loop scenario drives a provider that emits `Start` + one delta and then
  ends with no terminal frame, and asserts all four consequences: the partial is
  the turn's output, it is flagged `Error` with the reason, `TurnEndReason::Error`
  reaches the run outcome, and the text is in `agent.history()` — i.e. in the
  durable mirror a resume reads back.
