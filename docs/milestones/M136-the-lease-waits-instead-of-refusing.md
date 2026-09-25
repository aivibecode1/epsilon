# M136 — a killed TUI must not lock you out of your own session for 30 seconds

Close the window, reopen it, and Epsilon tells you another instance is writing the session — and
exits. For thirty seconds. Every time.

## What

`SessionLease` is correct in what it protects: one writer per session file, because two appenders
to the same `.jsonl` interleave and corrupt it. That part must not move.

The defect is in what it does when the holder is **gone**. A live holder heartbeats its lock file
every `HEARTBEAT` (10 s) and a lock older than `STALE_AFTER` (30 s) is stealable — so the
heartbeat exists precisely to distinguish "busy" from "crashed". The refusal path
(`crates/epsilon-session/src/store.rs:432-439`) treats both cases identically: it reports
`Held` and the caller gives up, whether the holder is a TUI on the user's other monitor or a
process the user killed thirty seconds ago.

So the ordinary way to restart is to press Ctrl-C, close the tab, and see an error instead of a
prompt. The console probe reproduced exactly this on 2026-09-21: an 80×25 run refused to start
because `pid=9944` — the previous run's TUI, killed seconds earlier — still held the lock
([`audit/evidence/console-probe-2026-09-21.md`](../../audit/evidence/console-probe-2026-09-21.md)).

This is the TUI's worst remaining user-facing behaviour, and it sits in the same neighbourhood as
the freeze the owner reported: from inside a terminal, "refuses to start" and "hangs" are the same
experience.

## The decision, and why it is not a liveness probe

`audit/11-current-assessment.md` recorded this as a design decision and named two options: a
narrowly scoped liveness probe, or a refusal that waits out the remaining window with a countdown
instead of exiting. It leaned toward the probe and noted the obstacle — checking whether a pid is
alive needs a platform call, and this workspace forbids `unsafe` outside a reviewed per-crate
override.

**The probe is the wrong answer, and the obstacle is the reason.** A liveness check tells you the
holder is dead at one instant, which is strictly weaker than what the heartbeat already gives you,
and it introduces a new failure mode: pid reuse, and a platform call whose semantics differ per
OS. Meanwhile the cost of the alternative is thirty seconds of *waiting* in the only case where the
holder is genuinely alive — a case where waiting is the correct behaviour anyway, because the user
is looking at a second window that owns the session.

So: **wait, then steal.** No `unsafe`, no new dependency, no pid semantics, and it fails safe —
if the holder is alive we time out and report it, exactly as today; if it is dead we take the
session the moment the heartbeat window closes.

## The shape

- `acquire_with` grows a `wait: Duration` parameter (production: the full `STALE_AFTER`; tests:
  milliseconds, so the lease's own suite still does not spend real seconds proving a real window).
- On a `Held` outcome whose lock is not yet stale, it sleeps in short steps to the stale deadline,
  re-reading the lock each time — because a holder that exits mid-wait *removes* the file
  (`Drop`), and the wait must notice that rather than sit out the remainder.
- The wait ends in one of two honest outcomes: the lock went stale and was stolen (`Acquired`), or
  the deadline passed with the holder still live (`Held`, reported as today, now with the holder's
  pid and the time actually waited).
- A zero `wait` preserves today's behaviour exactly, which is what gives the tests a cheap way to
  assert *both* outcomes without sleeping.

**The countdown belongs in the TUI, not in the library.** `epsilon-session` must not depend on the
TUI, so the lease returns what it learned (held, pid, how long it waited) and the caller decides
whether to print a spinner. A headless `-p` run waits in silence, which is correct — nothing is
watching.

## Verification

Battery **`m136a`**, [`audit/evidence/gate-m136a.log`](../../audit/evidence/gate-m136a.log), and a
negative control recorded in
[`audit/evidence/m136-negative-control.txt`](../../audit/evidence/m136-negative-control.txt).

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean, zero warnings |
| `cargo test --workspace` | **1188 passed, 0 failed, 1 ignored** across 61 binaries |
| `cargo run -q -p epsilon-cli -- --demo` | completes |
| `cargo check -p epsilon-cli --features real` | compiles |

The suite delta is +3 tests and nothing else moved: 1185 at M135, 1188 here, and all three are the
lease tests below. The ignored test is the env-gated real-provider check.

The control is the part worth reading. It changes one line — `if elapsed >= wait` to
`if elapsed >= Duration::ZERO`, which is exactly the pre-fix condition, since every elapsed time
compares greater than zero — and reruns the lease suite. Three tests fail, and the one that names
the defect fails like this:

    a dead holder must not make the next launch fail: Held { holder: Some("pid=999999\n"), waited: 309.3µs }

**309.3 microseconds.** The acquirer looked once, found a lock file from a process that does not
exist, and refused. That is the bug in one number: the user killed a TUI, reopened it, and was told
to wait 30 seconds by a check that took three hundred millionths of one.

Two details from the control that are worth keeping:

- **It fails three tests, not one, and the third fails for an incidental reason.** The
  live-holder test asserts `waited >= 500ms`, so under the control it fails because the lease
  refuses *too eagerly* — the opposite of the property it exists to protect. It cannot do its job
  while the bug is present. That is honest and worth recording: a test that guards against stealing
  too eagerly only proves anything once the wait exists.
- **The five pre-existing M61 tests pass under the control.** The change is additive to the
  exclusivity guarantee; it does not weaken a single one of them.

The three tests (all in `crates/epsilon-session/src/lease.rs`):

- `a_waiting_acquirer_takes_the_lock_the_moment_it_goes_stale` — the defect, in miniature, with
  timings compressed 200× (300 ms window, lock aged 150 ms, 2 s budget). Asserts both that the
  steal succeeds *and* that it waited — a test that only asserted success would pass just as
  happily against a lease that steals instantly, which is the corruption this whole mechanism
  prevents.
- `a_waiting_acquirer_still_refuses_a_heartbeating_holder` — the other half: a live holder survives
  the full wait, is still named, and the refusal reports the wait actually taken.
- `a_holder_that_exits_mid_wait_is_noticed_at_once` — the case this document originally listed under
  "what nothing exercises". A holder quits 150 ms into a 3-second wait; the acquirer must take the
  lock promptly rather than sitting out the remainder. The wait re-reads the lock on every poll
  specifically so that `Drop`'s removal is noticed, and now there is a test that would notice if
  somebody removed the re-read.

One test elsewhere changed rather than being added: `lease_blocks_second_writer_until_released` in
`tests/session_store.rs` used `acquire`, which now waits. Since that test *is* the live holder and
keeps heartbeating, the waiting call would have sat out the full 30-second window to assert a
refusal. It now calls `acquire_now` — the explicit no-wait entry point, which is also why that
method exists.

## Limits — what this does not prove

- **A wait is not a fix for a holder that is alive and stuck.** If a TUI is hung rather than
  closed, the user still waits 30 s and then gets the honest refusal. Distinguishing "hung" from
  "busy" needs the liveness probe this milestone deliberately rejected.
- **The pid in the message is still whatever the lock file says.** It is written by the holder and
  never verified, so it is a hint for the user, not evidence. The wording has to say so.
- **`discover.rs:217` has its own staleness rule** using the same `STALE_AFTER` constant. It
  decides which sessions a picker offers, not who may write, so this change does not touch it —
  but the two now disagree about what "recently used" means if the constants ever diverge.
- **Nothing here was measured on a real conhost window.** The console probe is the harness that
  can answer it, and it has not been re-run for this change.

## What nothing exercises

- **The TUI's rendering of the wait.** `epsilon-session` returns *what it learned* (held, pid, how
  long it waited) and `StoreError::Leased` now says so in its message, but a 30-second silent pause
  at startup is still a 30-second silent pause. The spinner belongs in the TUI, which cannot depend
  on the session crate, so it needs a callback or a state the caller can poll. Not written. **This is
  the slice's real remaining gap** — the fix removes the *refusal*, and the *silence* is what the
  user still experiences.
- **A real conhost measurement.** The console probe is the only harness that can answer "what does a
  user see during those 30 seconds", and it has not been re-run for this change.
- **The `discover.rs` divergence.** Its staleness rule reads the same `STALE_AFTER` constant but
  answers a different question (which sessions to offer, not who may write), so the two now use one
  number for two meanings. Harmless today; noted so a future change to the constant knows.
