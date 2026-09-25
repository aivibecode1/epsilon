# M138 — the lease wait is visible, not a blank screen

**Status:** implemented; verification below
**Follows:** M136 (which replaced a 30-second refusal with a 30-second wait)
**Evidence:** `audit/evidence/gate-m138b.log`, `audit/evidence/m138-negative-control.txt`

## The defect

M136 stopped the session lease from *refusing* a crashed holder's lock. That was
right, and it left a worse complaint behind: a blank terminal for up to 30 seconds
while work happens underneath. That is the same shape of failure the owner reported
about the Windows TUI — "it freezes and shows no input or output" — reproduced in
four lines of startup code, and it is why a fix can be correct in its own terms and
still be the wrong fix.

`StoreError::Leased` had no production handler at all before M136, and after M136 the
message existed but was only produced *after* the wait. The user got a clear error
eventually and nothing at all in the meantime.

## The fix

Three pieces, each doing one job.

**1. The lease reports what it is doing.** `SessionLease::acquire_with_progress` takes
an `on_wait(waited, remaining)` callback, invoked once per poll before sleeping. It
is `Fn`, not `FnMut`: a progress reporter has no business needing mutable capture, and
the tighter bound is part of the contract. Crucially it returns `()`, so a frontend
**cannot** influence the outcome — it may not shorten or extend a correctness window.
That is the reason this is a callback and not a "should I give up?" hook.

`acquire_with` and `acquire` keep their signatures and pass `|_, _| {}`, so every
existing caller and every test is untouched and pays nothing.

**2. The store threads it through.** `SessionStore::open_with_progress` and
`open_in_with_progress`. The existing three constructors delegate with a no-op.

**3. The CLI renders it.** `lease_wait_notice` writes a redrawn line to stderr:

    waiting for this session to be released — 3s elapsed, ~27s left (another process may still be writing it)

The rendering lives in `write_lease_wait(w, …)`, taking any `&mut impl Write`, so the
exact bytes a user sees are assertable in a test. A function writing straight to the
process's stderr could not be tested at all.

## Decisions worth stating

- **stderr, not stdout.** Every other startup notice uses `eprintln!`; a caller piping
  stdout must not get control characters in its data. Nothing else is drawing yet.
- **One redrawn line, not a spinner.** `\r` plus `\x1b[K` (clear to end of line) keeps
  the notice from becoming a 20-line scrollback of near-identical text, and works in
  both CMD and PowerShell, which is the whole platform matrix here. A spinner needs a
  timer thread and a redraw loop; this runs on a blocking path before the async runtime
  is doing anything. A countdown that changes about once a second is enough to prove
  liveness.
- **The `\x1b[K` is load-bearing.** Without it, the previous longer countdown leaves a
  stale tail on screen. A missing one is not a cosmetic regression — it produces
  numbers that are wrong to anyone who reads them, which is the same class of defect as
  the M135 split-diff overlap. There is a test for it.
- **Sub-second remainders round up.** `~0s left` while 400 ms remain reads as
  "finished" when it has not, which is the exact confusion this line exists to remove.
- **The wait is `ACQUIRE_WAIT`, named separately from `STALE_AFTER`.** They are equal
  by design — the wait exists to let a dead lock go stale, so anything shorter would
  still refuse the ordinary restart. Naming it separately makes "wait at least as long
  as staleness" a visible invariant instead of a coincidence two constants happen to
  share.
- **The message says *why*, not just that something is slow.** "another process may
  still be writing it" is the fact a user can act on; the pid, if there is one, is in
  the error they get if the wait runs out.

## Tests

Five, in two places.

In `lease.rs` (the behaviour):
- `the_wait_reports_progress_with_a_shrinking_countdown` — fires repeatedly across a
  300 ms wait, and **remaining never increases**. A countdown that goes up is the
  natural bug here.
- `an_uncontended_acquire_never_reports_a_wait` — the other half, protecting normal
  startup. Without it, a "helpful" progress line would appear on every single launch.

In `main.rs` (the rendered bytes):
- `the_notice_shows_a_countdown_and_why_it_is_waiting`
- `the_notice_clears_the_previous_polls_tail` — guards the `\x1b[K`.
- `a_sub_second_remainder_rounds_up_not_down`

The negative control removes the single `on_wait(elapsed, remaining)` call. The
behaviour test then fails with `expected several progress reports across a 300ms wait,
got 0` — the user would have been shown nothing at all, which is the defect in its
purest form — while the other nine lease tests pass, showing the fix is additive to
M136's correctness guarantee rather than a trade against it.

## Verification

Battery **`m138b`**, log at `audit/evidence/gate-m138b.log`; negative control at
`audit/evidence/m138-negative-control.txt`.

An earlier battery, `m138`, **failed** and is recorded here rather than deleted: clippy
rejected three things in `epsilon-cli` — two needless borrows of `lease_wait_notice`
at the call sites, and a redundant `use std::io::Write as _` inside `write_lease_wait`
that shadowed the one already in scope at module level. All three were mechanical, all
three were real (the workspace gate runs `-D warnings`, so an unused import is a
failure, not a note), and none of them indicated a behavioural problem. `m138b` ran
against the corrected bytes.

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | 1196 passed, 0 failed, 1 ignored (64 binaries) |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0 |
| `cargo check -p epsilon-cli --features real` | exit 0 |

The suite delta from M137's 1191 is **+5**: the two lease tests and the three CLI
render tests. No test was deleted or altered.

## Limits — what this slice does not do

- **The TUI itself still shows nothing.** This is a CLI-startup fix. The interactive
  TUI takes the same `SessionStore::open` path, so it inherits the *wait* (M136) but
  not the notice, because the notice is drawn to stderr before the alternate screen is
  entered. Rendering it inside the TUI means threading the state into the app, which is
  a larger change and is listed below.
- **stderr is written before the TUI's alternate screen.** In a TUI launch the line
  will be scrolled away by the first full-screen draw. Correct for `--resume` in a
  normal shell; not yet correct for the interactive path.
- **`\x1b[K` is assumed to be understood.** True of CMD and PowerShell's conhost; a
  terminal that ignores it would show a stale tail. There is no TTY capability probe,
  deliberately — the platform matrix here is Windows, and a probe that is wrong is
  worse than a line that is slightly untidy.
- **The notice is not interruptible.** `Ctrl+C` during the wait works, but there is no
  "stop waiting and fail now" affordance. The wait is bounded at 30 s, which is the
  current mitigation.

## What nothing exercises

- **The end-to-end path.** The behaviour tests drive the lease directly and the render
  tests drive `write_lease_wait` directly. Nothing holds a real lock in a subprocess
  and asserts the bytes a user would actually see on their console. That needs a
  pseudo-terminal and belongs with the existing terminal integration test.
- **The TUI alternate-screen interaction.** Nobody has measured whether this notice
  survives or is erased when the TUI takes the screen. Per the repo's own rule, a claim
  about what a user sees has to be measured with `scripts/console-probe.py`, not
  asserted from the code — and it has not been.
- **A notice during a *live* holder's wait** (as opposed to a dead one). The 30-second
  countdown against a second, genuinely-open window is the case where the user is most
  likely to want to abort, and nothing exercises the rendering of it.
- **Concurrent waits from two processes**, where both terminals are drawing this line.
