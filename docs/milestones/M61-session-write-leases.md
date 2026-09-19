# M61 — Session write leases, actually wired

Continuing the solidity pass (M60 audited the transport; this round audits
durability). The finding had the same shape as M60's: `epsilon-session/src/lease.rs`
implemented a complete cross-process write lease — `create_new` acquire, stale
steal, holder pid, Drop release, its own tests — and **nothing called it**.
Meanwhile `SessionStore::open*` acquired no lock at all, so two `epsilon`
processes could open one session file and interleave appends. `repair_on_open`
cannot fix that: it heals a torn tail and dangling tool calls, not a conversation
whose turns were spliced out of order.

## The design: the store owns the lease

The store *is* the writer, so the writer's lock belongs to it. `SessionStore`
now holds a `SessionLease`, acquired in the shared constructor
(`open_with_parent`) — which means every writer path is covered by construction:
the CLI's session open, its `--resume` repair pass, `Agent::live_resume`, and
every delegated child session (M58's `open_child_in`). Readers are untouched:
session discovery parses files directly and never opens a store, so `/sessions`
keep listing (and the picker now marks the busy ones).

A second writer gets `StoreError::Leased { holder }`, whose message names the
holder and both honest ways out:

> another epsilon is already writing this session (pid=1234) — one writer per
> session file; close it, or remove the .lock file if it is a crash leftover
> (a live holder re-stamps it every few seconds)

## Two real defects the new tests found

1. **The "steal a stale lock" path never removed the stale file.** It checked the
   mtime, fell through to `create_new`, and got `AlreadyExists` — so a stale lock
   was reported as `Held`, and a crash leftover would have wedged the session
   *forever*. The doc comment claimed stealability; the code only ever
   re-reported `Held`. This had never mattered because nothing called the lease.
   Now the stale file is removed before `create_new` (and losing that race still
   degrades safely to `Held`).
2. **A live holder was indistinguishable from a dead one after `STALE_AFTER`.**
   The lock was written once at acquisition, so a TUI open for an hour looked
   identical to a crashed process after 30 seconds — the lease would have
   cheerfully handed the file to a second writer, i.e. recreated the exact
   corruption it exists to prevent. Fixed with a **heartbeat**: the lease holds a
   small wake-on-demand thread that re-stamps the lock's mtime every
   `HEARTBEAT` (10 s), so `STALE_AFTER` (30 s) now bounds how long a *crashed*
   session stays wedged — not how long a busy one may run. Drop signals the
   thread and joins it before deleting the file, so a heartbeat can never
   re-create a lock nobody holds.

## The same-process trap (twice)

`Agent::live_resume` re-resuming the session this process already holds would
conflict with itself, so it releases first — and keeps the old handle so a
*failed* switch (locked by another process, or corrupt) leaves the current
session attached. That rollback clone is exactly what bit twice:

- keeping the `Arc` alive through the reopen meant the "released" lease was
  still held (`Acquired` kept the file, `previous` kept the store);
- fixing the obvious case left the clone in scope, so the reopen *still* refused.

Both are the same lesson: **`Drop` only runs when the last handle goes**, and a
"keep it for rollback" binding is a handle. The fix drops the clone explicitly
before opening and only keeps a rollback target when we were not the holder.

Two existing tests had to be rewritten for the new rule, and honestly rather than
by weakening it: `child_sessions_record_their_parent_link` and the v1-migration
test re-opened a file while their own store was alive (now they drop first, which
is what a real second reader would have to do), and the M58 durability scenario
now reads the child file as **bytes** — the live child still holds its lease, and
"what a restarted process would parse off disk" is the more faithful check.

## Verification

- `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets`
  zero diagnostics.
- `cargo test --workspace`: **457 passed, 0 failed** (448 before: +4 lease
  scenarios, +2 store scenarios, +2 core scenarios, +1 picker marker).
- New coverage: lease heartbeat keeps a live holder unstealable across several
  stale windows; a dead holder's lock *is* stolen (the defect above); drop stops
  the heartbeat and removes the lock; `Held` names the holder pid; a store
  refuses a second writer and allows one after the first drops; child sessions
  lease their own files without contending with the parent; the core refuses a
  lease another writer holds, allows a same-file re-resume, and keeps the current
  session attached when a switch is refused; the picker marks `· in use`.
- `cargo test -p epsilon-llm --features real`: 131 passed, 0 failed.
- `cargo run -q -p epsilon-cli -- --demo` completes.
