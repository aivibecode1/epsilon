# M148 — the unconfined shell cannot be held open by a descendant (F11, second half)

## What

M147 bounded the confined run. This bounds the other path, which the same ledger entry named
and which M147 deliberately left alone: `crates/epsilon-tools/src/shell.rs` applied the
caller's `timeout_secs` to `child.wait()` and then awaited **both pipe drains outside that
bound**.

That is not a small gap in coverage. A descendant that outlives the direct child while holding
stdout makes `wait()` return promptly — and the drain then blocks on a pipe nobody will ever
close. The tool call hangs indefinitely, past the timeout it promised, on a command that has
already exited successfully.

## Key decisions

**A completed command keeps its verdict.** The fix bounds the drains, and on expiry reports
`output_complete: false` in `details` rather than failing the call. A command that exited 0
exited 0; the only casualty is how much of its output was collected, and turning "this build
succeeded but a daemon it spawned is noisy" into a tool error would misreport the one thing we
actually know. The flag makes the shortfall visible instead of silent — the same rule M146
applied to a missing exit code, applied here to missing output.

**A 5-second grace, not the remaining budget.** The command's budget is already spent by the
time the drains start; using what remained would give a command that consumed its whole
allowance no time at all to report. On a well-behaved run the readers finish in microseconds, so
the grace is slack for the pathological case only. `a_normal_command_reports_complete_output_immediately`
returns in 0.34s, so the ordinary path never pays it.

**The grace is per-run, not per-pipe.** See the gotcha below.

## Gotchas

**The test caught a flaw in the fix, not just in the defect.** The first working version awaited
the two drains one after the other. Each reader gets a 5-second grace, so a call with both pipes
held took **10.46 seconds** — numerically identical to the unfixed control. My first bound was
written as "< 12s", which the sequential version passed. Tightening it to "< 8s" is what exposed
the doubling; `tokio::join!` makes the two deadlines the same instant.

The lesson generalises: **a negative control proves the old code was wrong, but only a test
tight enough to fail on a nearly-right fix catches the fix.** M145's separate-accounting writer
was the first instance of that in two milestones; this is the second.

**A grandchild inherits the test harness's output pipe.** The first version used a 30-second
probe and the test binary took 31 seconds, because the orphan held the pipe the harness reads
output from. The probe is 10 seconds now — still far longer than the 5-second grace, and the
assertion is on the *tool call's* elapsed time, not the test's. The grandchild is deliberately
not killed: on the unconfined path nothing owns it, which is the standing difference from the
confined path where a job object takes the whole tree.

**`run` returns text; these tests need the envelope.** The existing `run` helper flattens the
result, which is right for most assertions and wrong for any question about `details`. A
`run_unconfined` helper was added rather than changing `run` for everyone.

## Verification

- `cargo test -p epsilon-tools` — **166 passed, 0 failed** across 3 binaries.
- `a_descendant_holding_the_pipe_does_not_hang_the_call` — passes; the call settles in under 8s
  against a 10-second probe, and asserts `exit_code == 0` with `output_complete == false`.
- `a_normal_command_reports_complete_output_immediately` — passes in **0.31s**, `output_complete
  == true`, and the output text is intact. This is the positive control: the fix does not turn
  into "always slow" or "always suspicious".
- `cargo clippy -p epsilon-tools --all-targets -- -D warnings` — exit 0. (`epsilon-tools` has no
  `real` feature; the flag is an error there, not a warning.)
- `cargo fmt --all -- --check` — exit 0.
- `cargo test --workspace --features real` — **62 binaries, 1,244 passed, 0 failed**, exit 0
  (from 1,242 at M147: the 2 new scenarios). `cargo run -q -p epsilon-cli -- --demo` — exit 0,
  6 messages, `tokens in/out: 300/150`.

**Two failed attempts, both disclosed, neither a code defect.**

1. **A stale binary.** `cargo test -p epsilon-tools` run *concurrently with* the background
   workspace suite reported `output_complete: Null` and an 11.07s settle — the control's exact
   numbers, from a source file that provably contained the fix (`tokio::join!` at
   `shell.rs:476`, the flag at `:503`, no `out_task.await` anywhere in the file). The background
   job was still writing to the same `CARGO_TARGET_DIR`. `touch`ing `shell.rs` and re-running
   gave 0.31s and a pass; the full crate then gave 166 passed / 0 failed. **The lesson is in
   memory: never run two cargo invocations against one target directory on this host, and when
   a result contradicts the source, re-run before believing either.**

2. **A flaky positive control of my own making.** `a_normal_command_reports_complete_output_immediately`
   bounded at 2s passed in 0.31s alone and failed at 2.39s under the full parallel suite. That is
   PowerShell startup on a loaded machine, not the drain grace. The bound now asks the question
   actually meant — "well under the 5s grace" — at 4s, which still fails a regression that always
   pays the grace.

### Negative control

Reverted to the pre-M148 lines **verbatim** (the unbounded `out_task.await.unwrap_or_default()`
pair, with the `output_complete` flag removed so the old code compiles as it did). Captured in
[`audit/evidence/f11-unconfined-negative-control.md`](../audit/evidence/f11-unconfined-negative-control.md).

```
test a_descendant_holding_the_pipe_does_not_hang_the_call ... FAILED
settled in 10.4615637s; a held pipe should not hold the call
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.47s
```

**10.46 seconds is the descendant's lifetime** — the call did not return early, it waited for
the pipe to close by itself. That is the defect as a measurement, and it is the same number the
sequential version of the fix produced, which is how that flaw was found.

Restored from backup (SHA-256 prefix `c8fbcfaed036ae39`); no `.ctrl-backup` remains.

## What nothing exercises

- **The confined path.** M147's bwrap work has no control on this host and says so; this
  milestone does not change it.
- **Whether the grandchild is killed.** It is not, and nothing asserts that it should be. The
  unconfined path has no process tree to own.
- **Whether anything reads `output_complete`.** Nothing does yet. It is recorded for a caller,
  and the TUI does not surface it — a user whose output is short gets no on-screen hint that
  anything is missing.
- **Cancellation mid-drain.** The `ctx.token.cancelled()` arm returns before the drains, so a
  cancelled call never reaches them; that path is not exercised by these tests.

## Limits

- The 5-second grace is a fixed constant, not configurable and not derived from the caller's
  timeout. A user asking for a 1-second command still waits up to 5 more seconds on a held pipe.
- On expiry the output collected so far is kept but may be cut mid-line; there is no marker in
  the text saying where it stopped. `output_complete` carries the fact, nothing carries it to
  the model's view.
