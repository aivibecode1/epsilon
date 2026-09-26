# F11 unconfined-path negative control — a held pipe used to hold the call

**Date:** 2026-09-26 · **Milestone:** M148 · **Defect:** F11 (second half, `shell.rs`)

The bwrap half of F11 is in
[`m147-one-deadline-for-the-whole-run.md`](../../../docs/milestones/M147-one-deadline-for-the-whole-run.md)
and has no control, because this host cannot run bwrap. This half *can* be controlled, and the
control is unambiguous.

## What was wrong

`crates/epsilon-tools/src/shell.rs` bounded the child with the caller's `timeout_secs`, then
awaited the two pipe drains **outside that bound**:

```rust
let wait = tokio::time::timeout(Duration::from_secs(timeout), child.wait());
let status = tokio::select! { /* … */ };
let stdout_bytes = out_task.await.unwrap_or_default();   // ← no bound
let stderr_bytes = err_task.await.unwrap_or_default();   // ← no bound
```

A descendant that outlives the direct child while holding stdout makes `wait()` return promptly
— and then the drain blocks on a pipe nobody will ever close. The call hangs long past the
timeout it promised.

## The control

The fix was reverted to the pre-M148 lines **verbatim**, not reimplemented: the unbounded
`out_task.await.unwrap_or_default()` pair restored, and the `output_complete` flag removed from
the details envelope so the old code compiles as it did before.

The probe spawns a grandchild that inherits the output pipe and holds it for 10 seconds
(`Start-Process -FilePath "ping" -ArgumentList "-n","11","127.0.0.1" -NoNewWindow`; `-NoNewWindow`
is what makes the grandchild inherit the handle), then the direct child exits 0 immediately.

```
test a_descendant_holding_the_pipe_does_not_hang_the_call ... FAILED
settled in 10.4615637s; a held pipe should not hold the call
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.47s
```

**10.46 seconds is the descendant's lifetime.** The call did not return early; it waited for the
pipe to close on its own. That is the defect stated as a measurement, and it is the same number
the pre-`join!` version of the fix produced — see the gotcha below.

With the fix in place the same test settles in under 8 seconds (the 5-second grace), and
`a_normal_command_reports_complete_output_immediately` returns in **0.34s**, so the ordinary path
never pays the grace.

## A real flaw the test caught in the fix itself

The first working version of the fix awaited the two drains **one after the other**:

```rust
let (stdout_bytes, out_done) = collect_within(out_task, DRAIN_GRACE).await;
let (stderr_bytes, err_done) = collect_within(err_task, DRAIN_GRACE).await;
```

Each reader gets a 5-second grace, so a call with both pipes held took **10.46 seconds** — the
same number as the control. The bound test was written as "< 12s" at first, which the sequential
version passed, and tightening it to "< 8s" is what exposed the doubling. `tokio::join!` makes
the two deadlines the same instant, which is the only shape the constant implies.

This is the second time in two milestones that a test written to discriminate caught the fix as
well as the defect (M145's separate-accounting writer is the first). A control proves the old
code was wrong; a test tight enough to fail on a *nearly* right fix is what catches the fix.

## Restoration

`shell.rs` was restored from its own backup. SHA-256 prefix `c8fbcfaed036ae39`; no
`.ctrl-backup` files remain; `collect_within(out_task, DRAIN_GRACE)`, `tokio::join!` and
`"output_complete": output_complete` are all present in the restored file.

## What this control does not cover

- The confined path. `BwrapSandbox` is `cfg(unix)` and is untouched here; M147 has no control for
  that reason and says so.
- Whether the grandchild is killed. It is **not** — on the unconfined path nothing owns it,
  which is the standing difference from the confined path where a job object takes the whole
  tree. The test leaves it to finish and the OS to reap it. The cost is that the test binary
  stays alive until the grandchild does, because it inherited the harness's output pipe; the
  probe is 10s rather than 30s for that reason.
- Whether `output_complete: false` is read by anything. Nothing consumes the flag yet; it is
  recorded for a caller to use, and the TUI does not yet surface it.
