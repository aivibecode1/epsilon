# M211 — reaping locks whose holder is gone

**Status:** landed. GATE PASS `m211` — see [Verification](#verification). Reinstall: **required**
(the sweep runs at launch).

## What

One session per file (M210) means one **lock** per session, and a process that was killed — a closed
terminal window, a `taskkill` — never removes its own. So a workspace used daily accumulates two
entries per launch and half of them are dead, in the directory a user lists when they go looking for
their sessions. M210 introduced that shape; this closes it.

`epsilon_session::sweep_stale_locks(dir)` removes `*.lock` files whose mtime has not moved for
`STALE_AFTER` (30 s), and the CLI calls it once per launch on the session directory.

## Why this is safe, in the same terms the steal is

**The predicate is exactly the one `SessionLease::acquire` already uses to decide a lock may be
stolen.** A live holder re-touches its lock every `HEARTBEAT` (2 s), so a lock untouched for 30
seconds has no live holder and `acquire` would take it without hesitating. The sweep deletes
precisely the files `acquire` would otherwise have to work around — so it cannot take a lock from a
living process, and it changes no decision: removing a stealable lock only spares the next process
from watching it.

That is also why this is the *safe half* of the lease problem, and the ledger says so: shortening
`DEAD_HOLDER_SETTLE` (4 s) would make a killed holder cheaper still, and it is the wrong fix,
because that constant is the margin that stops a live-but-stalled holder from being mistaken for a
dead one. Cleaning up after a dead holder is free; deciding faster that a holder is dead is not.

## The two things it deliberately does not touch

- **Files that are not locks.** The sessions directory holds the history itself, and a sweep that
  matched `*` would delete it. Asserted directly: an aged `old.jsonl` survives.
- **A failed removal.** A directory that cannot be read, a file another process just took, Windows
  permissions — none of them is a reason to fail a launch, so errors are invisible by design and the
  return value is a count for tests, not a diagnosis. It is not logged either: a launch is not the
  place for housekeeping chatter.

## Verification

GATE PASS, 2026-09-30 — battery **`m211`**, documentation final before it started, disk knob as
M208 established (`CARGO_PROFILE_DEV_DEBUG=0`/`CARGO_PROFILE_TEST_DEBUG=0`). Seven gates, exit 0: `check-links` 0 s (481 relative links, 0 broken), `check-lint-tables` 0 s, `fmt --check` 2 s, `clippy` 29 s, `test` 321 s, `demo` 25 s, `real-check` 1 s.

**1,631 tests across 65 binaries, 0 failed** — summed from the 65 `test result: ok.` lines. M210
measured 1,629 across 65; this slice adds the two sweep tests in `lease.rs`.

The compile caught one real mistake on the way here, worth recording because it is the kind that
reading the diff would not: `epsilon_session::lease` is a **private** module, so the CLI could not
call the new function — the fix was the re-export the crate already uses for its other lease types
(`pub use lease::{LeaseOutcome, SessionLease, sweep_stale_locks}`), not making the module public.

Evidence: [`audit/evidence/gate-m211.log`](../../audit/evidence/gate-m211.log).

**Reinstall: outstanding.** The sweep runs at launch, so the installed binary does not have it yet;
one release build covers this and anything else landed before the next install.

## Limits — what this does not prove

- **No end-to-end observation.** The sweep is unit-tested on a real directory with an aged lock and
  a fresh one, and the call site compiles; nothing launched the TUI, killed it, and then launched it
  again to watch the stale lock disappear. The `-c` probe from the previous round is the natural
  place to add that, and it is still outstanding for the resume question too.
- **The sweep runs on *every* store open**, including `--resume <path>` for a file in an unrelated
  directory — where it will reap that directory's stale locks. That is consistent (the same
  predicate, the same meaning) but it is a side effect of *opening* a file, which a reviewer could
  reasonably call surprising. It is recorded rather than argued away.
- **Nothing sweeps the session files themselves.** Dead `.jsonl` files accumulate by design (one per
  launch, kept forever); the sessions browser is the only way to see them and there is no retention
  policy. That is a product question, not a bug, and it is now the larger of the two directory-growth
  issues.
