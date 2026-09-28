# M181 — Extensions intercept tool calls (ADR-019, phases 4.3 and 4.5a/4.5b)

## What

An extension can now **veto a tool call**. Until this, an Epsilon extension could contribute
a tool and watch the transcript, and nothing else: `ExtHost::send_event` wrote a line to the
child's stdin and returned `()`, the protocol had exactly two host-to-child frame types
(`event` and `invoke`), and there was no way for a child to answer anything.

Two commits, because the two halves are separable and only the second is the point:

| | |
|---|---|
| `e9d5b94` | ADR-019 — the decision, and the constraint that forced its shape |
| `ece3af4` | the transport: `interpose` / `interpose_result` and a demultiplexing reader |
| this | the bridge: `ExtensionToolHooks` in `epsilon-cli`, installed on the agent |

`ToolHooks` and `TurnHooks` are the correctly-shaped seam for pi's mutating extension events
and the agent loop already called both correctly (`exec.rs:654,782`). But `set_tool_hooks` and
`set_turn_hooks` appeared **ten times in the repository and every one was in
`tests/loop_scenarios.rs`**. The seam was built, tested, and unreachable. This is the first
production `set_tool_hooks` call in the project's history.

## Key decisions

**The transport had to be structural, not additive.** One mutex over the pipe, held for a
whole `invoke`, cannot carry a request/response channel. A single long-lived reader task now
owns stdout and routes frames by `id` to per-request oneshots; the mutex shrank to writes,
and the child's `log` channel moved off `ChildIo` with it, because the reader must not take
the write lock to append a diagnostic.

**The bridge lives in `epsilon-cli`.** `epsilon-ext` depends only on `epsilon-contracts` and
must keep doing so, or the process boundary stops being testable in isolation;
`epsilon-core` does not depend on `epsilon-ext` at all. `epsilon-cli` is the one crate that
sees both, and it already owned `spawn_event_forwarder`.

**Any veto wins; there is no majority and no first-wins.** An extension declaring a veto is
asserting something the harness cannot check for itself. A harness that let a second opinion
overturn the first would make the safety property depend on extension load order — so
*adding* an extension can only *add* a restriction.

**Silence is neither consent nor a veto.** A malformed decision is ignored rather than read
in either direction. Reading the string `"block"` as a block would let one buggy extension
deny every tool call in the harness; that is the most damaging misreading available at this
seam and it has its own test.

## Test-caught bugs

**Five of the 22 pre-existing protocol tests failed on the first pass**, all correctly, all
from one mistake: I collapsed every structural failure into a single `Null` marker. That
turned three protocol violations into "the child died" *and charged them to the crash
budget*, where one stray line can now disable an extension for the session. The fix was to
make the channel carry a typed `Result` and have the reader distinguish malformed JSON (a
violation) from a dead stream (a crash) from an oversized or over-budget frame (a violation
that desynchronizes the stream, so reading on would splice the rest of the line into the
next frame).

**I removed a filter that was load-bearing.** While restoring the type-then-id order I
briefly deleted `validate_result`'s `type == "result"` check as redundant, because the reader
now filters on type. It is not redundant for a *stale* reply, and the reasoning was wrong
in a second way: I then hardcoded `result` in the reader, which silently dropped every
`interpose_result` frame. The filter belongs in **both** places, and `--shape-noise` exists
precisely because a child will stamp a real request id on a frame of any other type.

**A test found a real performance defect I had shipped in the ADR.** ADR-019's failure
policy says a silent extension must not *block* a run. It does not — but `interpose` reused
`INVOKE_TIMEOUT_SECS` (10 s), so a silent extension held every tool call open for ten
seconds. An interpose runs before *every* tool call, so a 30-call turn would add five minutes
of pure waiting. "Must not block" and "must not delay" are different requirements and only
the first was written down. `INTERPOSE_TIMEOUT_SECS = 1` now separates them; the suite went
from 3.06 s to 1.06 s on the same machine.

**ADR-019 was wrong about the failure mode and is corrected in place.** It claimed the old
single-lock host would *deadlock* on an interposed event. It would not: the old code read the
pipe itself, so a waiting request *was* the reader and its own reply reached it. The real
failure is head-of-line serialisation — measured at **9.74 s** to write one event line during
a slow tool call. The decision is unchanged; the reason is corrected, because an ADR naming a
nonexistent failure mode gets used later to justify something that does not help.

## Gotchas

- `ToolCallId` is `type ToolCallId = String` — there is no `::new`. `docs/milestones/M42`
  recorded this already; I re-learned it.
- `ToolResultMessage` has no constructor either. Build it field-by-field, matching
  `epsilon-core/tests/sequence_repair.rs:155`.
- `after_tool_call` returns `Option<AfterToolCallResult>`, not the struct.
- `Agent::new` already returns `Arc<Agent>`. Wrapping it produces `Arc<Arc<Agent>>`, and the
  error surfaces ~150 lines later at the function's return.
- `CARGO_BIN_EXE_ext-fixture` is only set for `epsilon-ext`'s *own* integration tests, so the
  CLI tests derive the fixture path from `current_exe()` instead. They **skip** rather than
  fail when it is absent — the same rule as the terminal suite, and the reason to build the
  fixture explicitly and check the skip count was zero.
- `async-trait` was absent from `epsilon-cli` because it had never implemented a contracts
  trait. Every other crate that does, declares it; the missing line was not an oversight but
  an untaken path.

## The test that was worthless twice

The concurrency claim — "an interpose is answered while other work is in flight" — is the
whole justification for the reader task, and it took **three** attempts:

1. A child that merely answers `invoke` frames: **passed against the control**. The control
   serialised requests, so the answer arrived; it just arrived late.
2. Two interposes at once: **also passed against the control**, because the old lock made
   them serial rather than concurrent.
3. Measuring the *wait*: pre-ADR-019 design **9.74 s**, new design milliseconds.

Both early versions asserted an **outcome** that the broken design also produces. Only the
third asserts a **measured quantity** with a bound an order of magnitude from the control's
value. This is the second vacuous test of the session, after `TerminalGuard`'s; the rule is
now written down in `agents/memory/01-plan-execution.md`.

For the bridge, the same discipline: the eight reducer tests (pure JSON in, verdict out) all
passed against a control that mangled the event tag, and only the end-to-end test caught it —
"the extension said no and the call went ahead anyway." That is the whole justification for
having it: the reducer tests check the arithmetic, the e2e test checks the wiring, and only
one of them can tell whether the two halves are connected.

## Verification

- `cargo test -p epsilon-cli --lib`: **51 passed, 0 failed** (12 new: 8 reducer, 4 end-to-end
  against a live child process). Verified to run 0 skips.
- `cargo test -p epsilon-ext`: **39 passed**, 0 failed. The 22-test protocol suite is
  unmodified.
- `cargo fmt --all -- --check` clean; `cargo clippy -p epsilon-cli -p epsilon-ext
  --all-targets -- -D warnings` clean.
- Full battery `m181`: see [`../../audit/evidence/gate-m181.md`](../../audit/evidence/gate-m181.md).

## What nothing exercises

- ~~**`after_tool_call` is not wired.**~~ **Superseded** — the result-rewriting half shipped
  in the same milestone's follow-on commit, after reading pi's `emitToolResult` directly
  rather than inferring it. See `audit/02-port-fidelity.md` for the two divergences that
  reading surfaced.
- **The interpose event payload is unvalidated on the way in.** An extension receives
  `tool_name` and `args` and the host trusts it to return well-formed JSON; there is no
  schema for `decision`, so an extension author has no contract to code against beyond this
  document.
- **Multi-extension veto is tested on the reducer only**, never with two live children. The
  `hosts` loop is sequential by design, and that choice is not exercised end to end.
- **No real extension exists.** Everything here is proven against `ext-fixture`, a test
  binary. Whether a third-party author can find the protocol documented well enough to use
  is untested, because no third party has tried.
