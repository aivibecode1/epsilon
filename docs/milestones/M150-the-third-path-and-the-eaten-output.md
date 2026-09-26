# M150 — the third path, and the fix that was eating the output (F11)

## What

Two defects, one of them inside the fix I shipped two milestones ago.

**The third path.** F11 was closed after M148 on the strength of two bounded drains: bubblewrap
(M147) and the unconfined shell (M148). Reading the third execution path showed the ledger entry
had been narrower than the defect. `sandbox/jobobject.rs` — the Windows confined backend, the one
this project actually runs on — bounded `child.wait()` and then awaited both pipe readers *outside*
that bound, exactly as the unconfined path did. The job object kills the processes assigned to it;
anything holding those pipes that is not in the job keeps them open while the call waits on a
command that has already exited.

**The second-order defect.** M148's bound was still wrong. `collect_within` returned
`Vec::new()` on expiry, and `drain_pipe` accumulated into a local it returned only at EOF — so a
timed-out drain handed back *nothing*, including everything the command had legitimately written.
A `cargo test` that printed its results and exited 0 came back with zero output. Bounding the wait
and throwing the data away is not a fix; it is a second, quieter defect wearing the first one's
clothes, and it was invisible to every test written before it because none of them checked whether
the output survived.

## What changed

- `drain_pipe` publishes each chunk into a `SharedBuf` **as it arrives** rather than accumulating
  locally, so `snapshot` is meaningful at any instant. Signature change; every call site updated.
- `collect_within` (both the `shell.rs` and `jobobject.rs` copies) awaits `&mut task` and returns
  `snapshot(&buf)` on both paths — on expiry the handle is dropped, which *detaches* the reader so
  it keeps draining in the background instead of being cancelled mid-read.
- `jobobject.rs` gains the M147/M148 drain grace, with `output_complete` on `ConfinedOutcome`.
- The confined details carry `output_complete` too, so a caller does not have to know which backend
  ran to know whether its output is whole. bwrap reports `true` honestly: that arm is only reached
  once both readers hit EOF.
- An in-band note in the **model-visible text** when output is short. M149 warned the human; the
  model was still handed short output as though it were whole, which is a correctness bug rather
  than a display one. The note is deliberately not the truncation marker: truncation keeps a spill
  file and names the locator, whereas here the rest was never written and there is nothing to point
  at.

## Negative control

Reverted only the snapshot, leaving the bound in place — the pre-M150 data flow exactly:

```
test a_descendant_holding_the_pipe_does_not_hang_the_call ... FAILED
the real output is still there — the note is added, not substituted:
"\n\n[epsilon: this output is INCOMPLETE. ...]\n"
test result: FAILED. 0 passed; 1 failed
```

The body is the note and nothing else: the command printed `child-exited`, exited 0, and the
harness reported an empty result. Captured in `audit/evidence/f11-jobobject-and-output-loss.md`;
restored from backup and verified byte-identical.

## A test that passed for the wrong reason

The first version of this test bounded elapsed time and failed at 8.6s on a loaded machine while
passing at 0.3s alone — PowerShell startup, not the drain. It is now an **ordering** assertion: the
descendant writes a marker file on its way out, and the call must return before that marker exists.
No timing threshold, and both directions load-insensitive.

Then the marker assertion **passed vacuously**. A quoting mistake — `''{path}''` inside a
PowerShell double-quoted string is two literal quotes, so `-LiteralPath` received an empty path —
killed the descendant at once, and a pipe nobody holds looks exactly like a drain that gave up on
time. The test now waits for the marker *after* the call and fails if it never arrives.

Both of these are the same lesson from different ends: **a test that cannot fail is worse than no
test, because it is a claim.** The first could not fail on a slow machine; the second could not
fail on a broken fixture.

## Verification

- `cargo test -p epsilon-safety -p epsilon-tools` — **223 passed, 0 failed** across 9 binaries.
- `a_confined_run_reports_whether_its_output_is_complete` — a real command through the job-object
  backend on this host, asserting the flag's *value* rather than its presence. A missing key reads
  as `Null` and would satisfy a truthiness check, which is the mistake this assertion exists to
  catch.
- `cargo test --workspace --features real` — **62 binaries, 1,251 passed, 0 failed**, exit 0 (from 1,249).
  `cargo clippy --workspace --all-targets --features real -- -D warnings` — exit 0 (**workspace**, not one package).
  `cargo fmt --all -- --check` and `cargo run -q -p epsilon-cli -- --demo` — exit 0, 6 messages, `tokens in/out: 300/150`.

**What nothing exercises:** the bubblewrap path still has **no executed coverage on this host** —
`cfg(unix)`, reporting 0 tests here, covered by CI on `ubuntu-latest`. Its `output_complete: true`
is honest by construction, but honest-by-reading is not executed, and the distinction is kept. The
`drain_pipe` signature change means the Unix call sites compiled on this host but were not run. The
descendant is still not killed on the unconfined path, because nothing owns it. And the note tells
the model its output is short without offering a way to get the rest — re-running is the only
option, which is honest but not helpful.
