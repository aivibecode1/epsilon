# M197 evidence: the extension-host hang, with a working negative control

**Both runs are real command output from this machine, not a description of it.**

## The defect

`ExtHost::request` (`crates/epsilon-ext/src/host.rs`) registers a `oneshot`, writes the request
frame, and waits for the reader task to complete it. When the reader stops it calls `fail_all`,
which wakes every request **already in the pending map**. A request that registers **after** the
reader has exited has nothing left to wake it: the write succeeds into a pipe no one reads, and
the call waits out `INVOKE_TIMEOUT_SECS` (10 s) before returning `ExtError::Timeout`.

The timeout is a false account of the event. Nothing was slow; the reader was already gone.

Why it is a product defect and not a test artifact: a harness holding a dead extension looks
exactly like a harness that has stopped responding, and because the failure is reported as a
timeout rather than a crash it never spends the crash budget, so the host is never disabled
either. Ten seconds of silence per call, forever.

## The fix

An `Arc<AtomicBool>` shared between `ExtHost` and `read_frames`:

- set on the reader's way out, **only** when the reader did not stop on the host's own shutdown
  signal (a clean teardown is not a broken host, and marking it would charge a crash for it);
- checked at the top of `request`, before anything is written;
- set **before** `fail_all`, so a request landing in the gap sees a stopped reader rather than
  racing the wake-up.

The error is `Crashed`, so the existing crash accounting disables a host that keeps dying.

## Negative control — the part that took three attempts

**Attempt 1 (`--die` fixture): the control PASSED.** The test was green with the fix and green
without it. It could not see the defect, which was live in the tree. Reason: with a dead child
the *write* to its stdin fails immediately, so the call returns fast regardless of whether the
host knows the reader is gone.

**Attempt 2 (`--flood 200000 100`): the control FAILS, for the reason the test names.** The
flooding child stays alive and keeps stdin open, so the write succeeds; the reader aborts on the
byte budget and exits. Every later request is in exactly the defective state.

### Fixed build

```
running 1 test
test a_request_after_the_reader_stops_is_refused_even_though_the_child_is_alive ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 28 filtered out; finished in 14.20s
```

### Control — identical test, fast-refusal removed from `request`

```
---- a_request_after_the_reader_stops_is_refused_even_though_the_child_is_alive stdout ----
panicked at crates\epsilon-ext\tests\support\mod.rs:52:19:
a request issued after the reader stopped did not return within 2s
(this is a hang, not a slow test)

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 28 filtered out; finished in 16.46s
```

The fix was then restored from the pre-control copy and `touch`ed, so cargo could not run a
stale binary (the mtime trap in `AGENTS.md`). Restored state confirmed: the fast-refusal string is
present once, the test is present once.

## Full suite at the same commit

`host_protocol`: **29 passed, 0 failed** (was 27 passed / 1 failed at the `m197-baseline`
battery). The 27 that passed before still pass — this is additive, not a rewrite.

## The rule this earns

> When a negative control passes, suspect the fixture, not the code.
> Ask what the broken build would produce, and confirm it differs.

Concretely here: a control that passes means the test's setup lets the broken build succeed by
some *other* route. The dead-child case succeeded through the write failing; the flooding case
has no such route, because the write is the one thing that still works. Choose a fixture that
leaves the defective component as the **only** possible failure.
