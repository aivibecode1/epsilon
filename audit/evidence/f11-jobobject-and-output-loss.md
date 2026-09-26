# F11, third path and the second-order defect — Windows Job Objects, and a fix that ate the output

**Date:** 2026-09-26 · **Milestone:** M150

## The third path

F11 was closed after M148 on the strength of two fixes. Reading the *third* execution path showed
the ledger entry had been narrower than the defect:

| Path | Where | Fixed by |
|---|---|---|
| bubblewrap (Unix, confined) | `sandbox.rs` | M147 |
| unconfined shell | `shell.rs` | M148 |
| **Job Objects (Windows, confined)** | `sandbox/jobobject.rs` | **M150** |

`jobobject.rs` bounded `child.wait()` with `tokio::time::timeout` and then awaited both pipe
readers *outside* that bound — the identical unbounded drain. The job object kills the processes
assigned to it, and anything holding those pipes that is not in the job keeps them open while the
call waits on a command that has already exited.

## The second-order defect: a fix that ate the output

M148 replaced an unbounded wait with a bounded one. It was still wrong, and the new test that
asserted *the real output is still there* is what found it.

`collect_within` returned `Vec::new()` on expiry. `drain_pipe` accumulated into a local and
returned it only at EOF, so a timed-out drain handed back **nothing** — including everything the
command had legitimately written before a stray descendant kept the pipe open. A `cargo test` that
printed its results and exited 0 came back with zero output. Bounding the wait and discarding the
data is not a fix; it is a second, quieter defect wearing the first one's clothes.

The fix makes the reader publish each chunk into a shared buffer as it arrives, so a snapshot is
meaningful at any instant.

## Control

Reverted only the snapshot, leaving the bound in place — the pre-M150 data flow exactly:

```
test a_descendant_holding_the_pipe_does_not_hang_the_call ... FAILED
the real output is still there — the note is added, not substituted:
"\n\n[epsilon: this output is INCOMPLETE. ...]\n"
test result: FAILED. 0 passed; 1 failed
```

The body is the note and nothing else. The command printed `child-exited`, exited 0, and the
harness reported an empty result. Restored from backup; verified byte-identical.

## A test that passed for the wrong reason

The first version of this test used a latency bound and failed at 8.6s on a loaded machine while
passing at 0.3s alone. Replacing it with an ordering assertion (the descendant writes a marker
file on its way out; the call must return before it appears) removed the timing threshold
entirely — both directions are now load-insensitive.

Then the marker assertion **passed vacuously**: a quoting mistake — `''{path}''` inside a
PowerShell double-quoted string is two literal quotes, so `-LiteralPath` got an empty path — killed
the descendant immediately, and a pipe nobody holds looks exactly like a drain that gave up on
time. The test now waits for the marker *after* the call and fails if it never arrives. A test
that cannot fail is worse than no test, because it is a claim.

## What is now proven, and on which host

- The unconfined path: 223 tests across `epsilon-safety` and `epsilon-tools` pass on Windows.
- The **confined** path: `a_confined_run_reports_whether_its_output_is_complete` runs a real
  command through the job-object backend on this host and asserts the flag's *value*, not its
  presence — a missing key reads as `Null` and would satisfy a truthiness check.
- The bwrap path is unchanged in behaviour and still has **no executed coverage on this host**;
  it is `cfg(unix)` and reports 0 tests here. CI covers it on `ubuntu-latest`. The new
  `output_complete: true` there is honest by construction — the arm is only reached once both
  readers hit EOF — but honest-by-reading is not the same as executed, and the distinction is kept.

**What nothing exercises:** the descendant is still not killed on the unconfined path (nothing owns
it), and the shared-buffer change to `drain_pipe` alters the signature every backend calls, so the
Unix call sites compile-checked here but were not run.
