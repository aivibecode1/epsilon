# M88 — Abort stops starting new calls inside the running batch

## What

`execute_batch` scheduled every prepared call unconditionally in both batch plans. The run
token travelled to tools through `ToolContext`, so a tool that personally honoured it could
stop early — but nothing stopped the *next* call from starting, and local tools (writes,
edits) simply ran. After Esc, the remainder of the active batch still executed: a cancelled
write or shell command could still happen.

pi's agent loop stops scheduling the remainder of an aborted batch and marks each unstarted
call as an error result ("Operation aborted"); in-flight calls are awaited and their results
still emitted, so the transcript never carries a call without an answer
(`references/pi-0.84.2/packages/agent/src/agent-loop.ts:478-480,516-553,629-635`).

The fix is one checkpoint at the top of `run_one` — the single choke point shared by the
barrier plan and the segment (conflict-group) plan — before the call's `ToolExecutionStart`
is emitted:

- cancelled run → the call is refused with `[harness] this call was not executed: the run
  was cancelled before it started.` (`details.aborted = true`, `is_error = true`);
- results still arrive for every call, in call order, so the transcript stays valid and the
  model is told plainly what did not happen;
- in-flight calls are untouched, exactly as pi awaits scheduled calls.

New test fixture `SlowTool` (sequential, no resource key → barrier plan, counts executions)
makes the mid-batch abort deterministic.

## Verification

Round-B battery (`gate-m86-88f`, candidate `epsilon-m86-green`, ten files byte-verified):
fmt, clippy, test, demo and real-check all exit 0 — **751 passed, 0 failed, 0 ignored
across 55 summaries** (`audit/evidence/gate-m86-88f.log`, sha256 `1ccc939e…`), with
`abort_mid_batch_refuses_calls_that_have_not_started` green. RED (executed against the
pre-fix code): the same test failed with `left: 2, right: 1` — both batch calls executed
after the abort, "only the call that was already running may execute"
(`audit/evidence/red-round.log`, sha256 `103b5307…`; attempt history in the verification
log).

## Regression (executed)

`abort_mid_batch_refuses_calls_that_have_not_started`
(`crates/epsilon-core/tests/loop_scenarios.rs`): two calls to `SlowTool` in one assistant
turn; abort lands while the first is in flight; asserts the tool executed exactly once, both
calls have results in call order, and the second is an error result saying it was not
executed.

## Limits

- The segment plan's groups each check the token before every call, but the test exercises
  the barrier plan; a multi-group abort scenario is not separately executed.
- Tools that already honour the token (shell) keep their own early-exit behaviour; this
  change is about calls that have not started.
