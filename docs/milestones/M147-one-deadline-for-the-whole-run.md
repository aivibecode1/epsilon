# M147 — one deadline for the whole confined run (F11, bwrap half)

## What

`BwrapSandbox::run` had three gaps that all pointed the same way — a confined command was not
actually bounded:

1. **The policy was discarded.** The signature read `_policy: &ConfinementPolicy`. The field is
   documented as "Wall-clock limit; the whole tree is terminated when it elapses", and the Linux
   backend simply dropped it. A confined `sleep 30` waited the full 30 seconds whatever the
   caller asked for.
2. **Even a wait-only timeout would not have been enough.** The child was awaited, and only then
   were the two pipe-drain tasks awaited. A descendant that outlives the direct child while
   holding stdout open returns from `wait()` promptly and then blocks forever in the drain — so
   the timeout would have moved, not disappeared.
3. **Cancellation leaked the tree.** Cancellation drops the future, and a dropped future with no
   `kill_on_drop` leaves a confined process tree running unsupervised. The unconfined path in
   `shell.rs` already set `kill_on_drop(true)`; the confined one did not.

## Key decisions

**One deadline over `wait` *and* both drains.** `timeout_at` wraps an async block that awaits the
child and then both readers, so "the run settled" means the output is complete, and the deadline
means it settles. `--die-with-parent` is already in the argv, so a held pipe cannot outlive the
kill.

**On expiry, kill and do not wait for the drains.** Awaiting a pipe that is still held is exactly
why the clock ran out; waiting there would relocate the hang to the next line. The drain tasks
detach and end when the namespace goes with the killed process.

**Cancellation is handled by the type, not by a caller convention.** `child.kill_on_drop(true)`
means every future that drops the child kills it, including paths nobody remembered to handle.

**The `wait` error variant changed from `Setup` to `Spawn`.** A failure to await a process that
did start is not a setup failure, and the acceptance criteria ask the three outcomes — did not
start, started then cancelled, normal nonzero exit — to stay distinguishable.

## Gotchas

**`tokio::spawn` was the wrong tool for the cancellation test, and the compiler would have
caught it — on a platform where the file compiles.** `sb.run(...)` borrows `sb` and `p`, so the
future is not `'static` and cannot be spawned. `bwrap_unix.rs` is `#![cfg(unix)]`, so this was
caught by reading, not by rustc. The test uses `tokio::select!` with a short timer instead: when
the macro returns, the unselected future is dropped, which is the same cancellation by a
different route.

**This file cannot be type-checked on the development host.** The whole suite is `#![cfg(unix)]`
*and* `BwrapSandbox` is itself `#[cfg(unix)]` in `lib.rs`. Lifting the gate in a scratch copy
still fails on the unresolved import; cross-compiling to `x86_64-unknown-linux-gnu` fails in
`aws-lc-sys`'s build script, which wants a C toolchain this box does not have. A scratch file
was created for the attempt and deleted.

## Verification

- `cargo clippy -p epsilon-safety --all-targets -- -D warnings` — exit 0 (the Windows-compiled
  surface; `epsilon-safety` has no `real` feature).
- `cargo test -p epsilon-safety` — **44 passed, 0 failed** (plus 5 + 5 + 4 in the integration
  binaries). Includes `timeout_kills_tree_and_reports_timeout_error` and
  `grandchildren_die_with_the_job`, which exercise the *same pattern* on the Windows job-object
  backend and are real executed proof that a deadline over a confined tree works.
- `cargo test -p epsilon-safety` also reports `bwrap_unix.rs … 0 passed`. **The three new tests
  did not run.** They are compiled out on Windows, not skipped after compiling.
- `cargo test --workspace --features real` — **62 binaries, 1,242 passed, 0 failed**, exit 0.
  (Same total as M146: this slice changes `epsilon-safety`, whose compiled-on-Windows test
  count is unchanged, and adds only tests that do not run here.)

**A failed first attempt, disclosed.** The first full run stopped at
`epsilon-session --test pi_import_reconciliation` with
`pi_import_carries_every_source_entry` failing on
`rename into …\before-compaction.v2.jsonl: The system cannot find the file specified. (os
error 2)`, after 46 binaries and 691 tests. That is a file operation failing under full-suite
load on this host — the same class as the artifact instability `AGENTS.md` records — and it is
**not** caused by this slice: the code changed here is `#[cfg(unix)]` bwrap code that does not
compile on Windows at all, and the test passes in isolation (5 passed / 0 failed) and in the
re-run. The re-run used `--no-fail-fast` so one flaky binary could not hide the rest.

### The three new tests, and their status

| Test | Asserts | Executed here |
|---|---|---|
| `a_confined_run_that_outlives_the_wall_clock_returns_timeout` | `sleep 30` under a 1s policy returns `Timeout { seconds: 1 }` in under 2s | **No** — `cfg(unix)` |
| `a_descendant_holding_stdout_cannot_outlive_the_deadline` | `sh -c 'sleep 30 & echo …'` settles within the deadline rather than hanging in the drain | **No** — `cfg(unix)` |
| `cancelling_a_confined_run_does_not_leave_it_running` | dropping the future leaves no marker written 5s later | **No** — `cfg(unix)` |

They run on the `ubuntu-latest` CI job, which installs bubblewrap and runs this suite
(`.github/workflows/ci.yml`). Until that job has run on this code, F11 is **partially** closed:
the source change is reviewed and the pattern is proven on Windows, but the bwrap behavior
itself is unexecuted.

**There is no negative control for this slice.** F05's fix could be reverted inside this crate
and the Windows suite would still pass, because the Windows backend does not share the code. A
control here means running the three new tests against the old implementation on Linux — which
this host cannot do. The milestone stays honest about that rather than claiming a control that
was never run.

## What nothing exercises

- The bwrap path at all, on this host (above).
- Whether `start_kill()` on bwrap reliably reaps a *detached* descendant, given
  `--die-with-parent` is implemented as a death signal relative to bwrap. The tests assert the
  observable contract — it settles, it does not hang — not the mechanism.
- The unconfined path's own residual gap: `shell.rs` bounds the child wait with a timeout and
  then awaits the drains outside it (`shell.rs:385–407`). F11's ledger text names this. **It is
  untouched here** and remains open.
- Whether the Windows backend's drain-after-timeout is bounded. It terminates the job first and
  then awaits the readers, so the pipes do close, but the wait is not itself under a deadline.

## Limits

- The deadline is `wall_timeout_secs` with a floor of 1 second; a policy asking for 0 gets 1.
- On expiry the collected output is discarded — the caller gets `Timeout`, not partial output.
  That matches the Windows backend, and is a deliberate choice over returning a truncated
  result that looks like a completed run.
