# M93 — Task state when resuming (F18)

## What

The F18 half of `audit/02-defect-ledger.md` (P1, "resumed child task is not marked running before
dispatch"), implemented from a verified read-only brief (banked while the work was in flight and
retired with this milestone). The defect:
`Agent::delegate_task`'s resume branch looked up the tracked child and dispatched a new leg
without ever writing `Running`, so everything that reads the registry was told the child was
settled while it ran:

- the "still running — poll it with task_id and no prompt…" guard compared a snapshot taken two
  calls earlier, so it could not fire for a resumed child — and two racing resumes could both
  pass it;
- `abort_delegated_tasks` selects `e.state == Running`, so a parent abort never reached a
  resumed child;
- `register_task_entry` evicts the first non-running entry, and nothing ever wrote `Running`, so a
  resumed child's entry kept its old *settled* state and was eligible for eviction: once evicted,
  the running child went untracked — a later resume or status probe of that id failed on the
  child's still-held store lease (`rehydrate_task` reads the child's file through
  `SessionStore::open`, `:2498-2499`; M61 refuses a second open) rather than reporting "still
  running", and the abort cascade could not reach it;
- the background arm swallowed the child's own `RunError::AlreadyRunning`, flipped the entry to
  `Failed` and queued a `state="failed"` notice for a run that was still in flight.

**The fix.** `Agent::claim_task_running` (`crates/epsilon-core/src/lib.rs:2787-2802`) does the
find, the refusal and the state change in **one lock hold**: an id that is no longer tracked is
an honest error, a child that is already `Running` is refused with the message the old guard
used, and otherwise the entry is claimed. It is called after the child lookup — the last fallible
step — and before the `if request.background` split (`:2850-2860`), and the racy guard it
replaces is gone: the claim *is* the refusal. Nothing else in the registry changed; the eviction
selector still skips `Running` entries (`:2723`), and because the claim now writes `Running`
before the split, a running child is genuinely out of the eviction pool — the untracked window
described above cannot open while it runs.

**Panic/poison honesty.** Claiming `Running` before a leg runs creates a way to strand an entry
if the leg dies. Both leg bodies' fallible work (run + summary) is now contained by
`contain_panic` (`:308-338`) — the std-only `pin!` + `poll_fn` + `catch_unwind` boundary the hook
seam already used, extended to keep the panic payload — so an unwinding run is recorded as
`failed` with the panic's own message (on the detached path the notice is still queued). The
foreground body (`await_task_run`, `:2952-3031`) is contained the same way, its panic arm
writing the same terminal record, because the claim runs before the background/foreground split:
without it, a foreground panic would unwind through the caller and leave the just-claimed entry
`Running` forever. `contain_hook` (`:294`) is now a thin wrapper over that one helper instead of a
second copy of the mechanism. The detached body's terminal write recovers a poisoned registry
lock (`:3089-3091`) rather than silently skipping the write, so no path out of either body leaves
`Running` behind.

**Tests** (`crates/epsilon-core/tests/task_scenarios.rs`, all offline — `FauxProvider` plus
`SlowProvider` for timing, no network). The five the brief asked for:

1. `a_resumed_background_child_is_running_before_the_dispatch_returns` — the resume's dispatch
   reports `Running` (previously the old `Completed`), with exactly one running entry, and the
   leg still lands its own completion.
2. `a_second_resume_of_a_running_resumed_child_is_refused_without_forging_failure` — refused with
   "still running", the claim left untouched, and the parent's next run receives only the
   completion notice — never a forged `state="failed"`.
3. `aborting_the_parent_reaches_a_resumed_running_child` — `abort_delegated_tasks() == 1` while
   the leg is in flight, and the child settles failed-aborted.
4. `a_resumed_running_child_survives_registry_eviction` — the entry first in dispatch order
   survives the 17th registration; the oldest settled filler is evicted instead. The fillers are
   seeded child session files that status probes rehydrate, so the whole fill needs no provider
   call and fits inside the resumed leg's stall.
5. `a_resumed_run_that_fails_lands_failed_not_running` — the failing leg's claim is followed by a
   terminal `Failed` carrying the reason, and a rehydrate that cannot rebuild its child
   registers nothing.

Plus two tests the containment owes, because nothing else exercises it:
`a_panicking_resumed_run_lands_failed_not_running` — a provider that panics on the resumed
background leg with no `PanicGuard` in the way; the panic must land as a `failed` record naming
it (and reach the parent as a failed notice) instead of dying silently — and
`a_panicking_foreground_resume_lands_failed_not_running` (review round), the same provider on the
foreground path, where the panic must surface as the call's error and still land the
just-claimed entry `Failed` naming the panic.

**Implementer's focused round — round 2 (review round)** (working tree, *not* a frozen candidate;
raw output in `audit/evidence/m93-focused-round.log` section "ROUND 2", taken at `lib.rs` sha256
`f0789260…` and `task_scenarios.rs` sha256 `791934d5…`; the round-1 numbers described bytes that no
longer exist): fmt exit 0; clippy exit 0 on the three named crates with `--all-targets` and
`-D warnings`; `cargo test -p epsilon-core` green — **207 passed / 0 failed / 0 ignored across 15
summaries** (the lib target's 108, thirteen integration binaries' 99, doc-tests) — and the focused
`--test task_scenarios` run green, **21 passed / 0 failed** in ~1 s: the 14 existing tests plus the
7 above.

Two negative controls in the same round (raw output in `audit/evidence/m93-probes.log`, round 2):
with `claim_task_running` disabled, the claim-dependent tests all red, but not uniformly at their
headline lines — test 1 and the background panic test stop at the dispatch-claims-`Running`
assertion, and tests 2-4 (second-resume refusal, abort reach, eviction) stop at that same
precondition; each headline property is still entailed by the claim, since none of the three can
hold before the dispatch reports `Running`. The existing running-refusal test reds too (the claim
covers the guard it replaced); test 5 and the new foreground panic test pass — both are fix-shape
guards, not claim reds. With the containment's recovery disabled, both panic tests fail
(background: the entry never settles; foreground: the containment's own arm panics out of the
call). One briefed test cannot have a pre-fix RED — see Gotchas.

**Environment caveat this round carries.** Avira false-positives the debug-built lib-test
executable as `HEUR/APC.Misc` on this machine (detections at 18:32 in `epsilon-m93-target`, 18:54
in its replacement, 18:57 on a renamed relink; execution denied, `LNK1104` on relink), so a
default-profile `cargo test -p epsilon-core` cannot complete here. The 207 figure comes from the
same command run with `CARGO_PROFILE_TEST_DEBUG=0` (a profile knob only — the quarantined content
is the debug build), and a per-target fallback under the default profile was also recorded: lib
target 108 (relinked without debuginfo) plus the thirteen integration targets' 99 via cargo. Raw
logs: `audit/evidence/m93-round2-verify.log`, `audit/evidence/m93-round2-command.log`. Until an
AV exclusion exists for the build directory, the frozen-candidate battery will meet the same
block.

## Key decisions

- **One lock hold, not a re-checked guard.** Two acquisitions cannot make a check-then-set
  atomic: two resumes both read a settled snapshot, both see it settled, both dispatch. The claim
  does find + refuse + set under one hold, which is the whole fix.
- **The claim is the refusal.** Deleting the old guard rather than keeping it beside the claim
  keeps one mechanism (and one message) instead of two that can drift; the old guard also ran
  *before* the child lookup, the order the fix deliberately inverts.
- **The claim is the last fallible step.** Every failure a resume can meet — unknown id, failed
  rehydrate, a child that vanished — happens before any state is written, so no leg that never
  started leaves a phantom `Running`.
- **Containment, not a second await.** The panic boundary reuses the crate's own std-only
  mechanism (`contain_hook`'s `pin!`/`poll_fn`/`catch_unwind`) extended to pass the payload, and
  `contain_hook` became a one-line wrapper so there is exactly one implementation. The
  `futures-util` route (`epsilon_llm::PanicGuard`'s) would have added a dependency this crate
  deliberately avoids.
- **The terminal write survives a poisoned lock.** `unwrap_or_else(|poisoned| poisoned.into_inner())`
  on this one write: the write is the point of the body, and a `Running` entry whose run is over
  is exactly the lie the milestone removes. No other registry access changed.
- **No public API change.** `TaskState`, `TaskSnapshot` and `TaskHost` are untouched, so the tool
  envelope and the TUI's `/tasks` badge follow the snapshot automatically; `epsilon-tui/**` is not
  part of this slice.
- **pi has nothing to port here.** pi ships no task tool (the registry is Epsilon's own, M53), so
  no pi citation belongs in this milestone.

## Gotchas

- **The eviction scenario cannot be built from timed runs.** The resumed child must still be
  `Running` when the registry hits its cap, but every leg of one agent shares a single
  `SlowProvider` delay — fifteen real filler runs would take fifteen times longer than the leg
  they are meant to outlast, so the child would settle first and the test would pass for the
  wrong reason. The fillers are therefore *rehydrated* entries: seeded child session files
  (`SessionStore::open_child_in` plus a `task_meta` entry, store dropped so its M61 lease is
  free) that a status probe rebuilds with no provider call, keeping the whole fill inside the
  200 ms stall. The residual if the fill ever outlasts the stall is carried in Limits, not only
  here.
- **The claim's coverage of the old guard is proven by accident.** Probe A (claim disabled, guard
  already deleted) also breaks the *existing*
  `resuming_a_running_background_task_is_refused_until_it_settles`: with neither mechanism in
  place a fresh background child can be resumed while it runs. That red is evidence the claim
  took over the guard's job, not a regression.
- **One briefed test cannot be a pre-fix RED.** `a_resumed_run_that_fails_lands_failed_not_running`
  asserts the failed leg's terminal state; pre-fix that state is also `Failed` (the failure write
  already existed), so the test passes with the claim disabled. Its property — a claim is never
  left unpaired by the failure path — did not exist before the claim, which is the no-pre-fix-RED
  case AGENTS.md answers with a negative control (the probe above). The panic tests cover the
  other failure paths and do fail without containment: with the recovery arm disabled, the
  background entry never settles and the foreground containment's own arm panics out of the call
  (probe B in `audit/evidence/m93-probes.log`, round 2).
- **A contained panic still leaves the child agent un-runnable.** The unwind skips `run`'s tail
  cleanup, so the child's own `running` flag stays set and later legs on that child answer
  `AlreadyRunning`. The registry is truthful (Failed, panic named); the child is not re-runnable.
  Both contained bodies share this residual — the foreground wrap catches the panic and lands
  the record, but the unwind has already skipped the child's cleanup; closing it belongs to the
  seam CORE-02 addressed for hooks — recorded in Limits.
- **`TASK_REGISTRY_CAP` is private**, so the eviction test hardcodes 16 with a comment naming the
  constant; a cap change must move that number with it.
- **The ledger's recorded F18 citations had drifted** (they anchor to revision `3afafd3`: the
  abort selector `:995`, eviction `:2229`, guard `:2334`, spawn write `:2495` — +135 at the abort
  site and +448…+458 in the task region by HEAD). The F18 entry is corrected against HEAD in this
  change, with the old→new mapping recorded there.

## Verification

**GATE PASS** on the frozen candidate `m93b` — `audit/evidence/gate-m93b.log`
(sha256 `19ecc04e…`): all five gates exit 0 (fmt 2 s, clippy 104 s, test 1329 s, demo 374 s,
real-check 38 s), **798 passed / 0 failed / 0 ignored, 55 summaries**. Per-binary delta against
the M92 baseline (`gate-m92b.log`, 791): `task_scenarios` 14 → 21 (**+7** — the six slice tests
plus the foreground containment test); every other binary unchanged, so the delta is exactly the
new tests.

- Candidate: HEAD `fec032a` + this slice's eleven files (code, docs, evidence), every one
  hash-verified against the working tree, with an explicit check that the other work item's
  hunks are absent (the files it touches — `stream.rs`, `config.rs`, `registry.rs`,
  `agent-contract.md`, `write.txt`, the TUI tree, `Cargo.*` — are not in the candidate).
- **Avira note (a new environment block, disclosed):** the debug-built `epsilon-core` lib-test
  executable is false-positived (`HEUR/APC.Misc`, execution denied; the same class as the
  earlier `wire_fixtures` block). The round therefore pre-flighted the test-profile knob and ran
  the battery with **`CARGO_PROFILE_TEST_DEBUG=0`** (fresh executable metadata), recorded in
  `audit/evidence/m93b-preflight.log`. No security configuration was touched — an exclusion is
  an owner/admin action.
- Focused run on the candidate before the battery: fmt clean, clippy exit 0 on the three touched
  crates with `--all-targets -- -D warnings`, `cargo test -p epsilon-core` 207 passed across 15
  summaries (108 lib + 99 across 13 integration binaries), `task_scenarios` 21.
- **Two negative-control probes on the candidate's own bytes** —
  `audit/evidence/m93b-probes.log` (sha256 `05d6c456…`):
  - P-A the `claim_task_running` call disabled → **7 tests fail** (the four claim-dependent ones,
    both panic tests, and the pre-existing `resuming_a_running_background_task_is_refused_until_it_settles`
    — proof the claim took over the deleted guard's job).
  - P-B the containment's recovery arm replaced by a panic → **both** panic tests fail. The
    foreground test failing here is the proof that the foreground leg really goes through
    `contain_panic` (without the wrap, this probe could not affect it) — the gap the review found
    is closed and pinned.
- **Independent review** — `audit/evidence/m93-review.md`: the single-hold claim, the call-site
  ordering, the containment refactor (M89's hook path unchanged, its regression green), the
  ledger citations and the hygiene all VERIFIED; one documentation blocker (the false
  "two live children" claim, contradicted by M61's store lease — the true consequence, an
  untracked running task, is now recorded at all four sites) and one real gap (the uncontained
  foreground leg, fixed here with its own test) were fixed before this battery; every residual is
  in Limits.
- **Reinstall (owed because this slice is CLI/TUI-visible — the `/tasks` badge and the
  second-resume refusal change):** the global `epsilon.exe` was rebuilt from this candidate's
  bytes, so the installed binary carries the verified state and none of the other work item's
  uncommitted changes.

## Limits

- **The cap is soft when every entry is Running.** `register_task_entry` (`:2718-2728`) evicts
  only when it *finds* a settled entry; with sixteen running children a 17th registration pushes
  and the registry grows past `TASK_REGISTRY_CAP`. This fix widens that window on purpose (a
  running resumed child is now unevictable) and adds no overflow policy.
- **A settled entry evicted for capacity can still be rebuilt as a fresh child.** The fix keeps a
  *running* child out of the eviction pool; once a child has settled it can be evicted, and a
  later resume then rehydrates it from its session file (`:2470-2604`) — one child reading the
  same history, but with its in-memory identity (turn/usage continuity, its place in dispatch
  order) gone. Closed: the *untracked window* evicting a *running* entry opened — pre-fix, the
  entry could go while the child ran, and a later resume or status probe of that id failed with
  the store's lease error (`rehydrate_task`'s `SessionStore::open`, `:2498-2499`; the running
  child's M61 lease refuses the second open) rather than an honest "still running", with the
  abort cascade unable to reach it; only after the first leg ended (lease released) could the id
  rebind as a fresh child. The eviction selector (`:2723`) skips `Running` entries and the claim
  now writes `Running` before the split, so the window cannot open while the child runs.
  Not closed: identity loss — for a child evicted *after* it settled, and for the pre-fix
  untracked case once its leg ended, a rebuild from the session file comes back without the
  in-memory identity.
- **The eviction test is timing-coupled to the stall.** It hardcodes the private cap (16) and
  needs the sixteen rehydrate probes — file reads, no provider calls — to complete while the
  resumed leg sits in its 200 ms `SlowProvider` stall (per-stream delay,
  `crates/epsilon-core/src/testing.rs:90`). The fill is cheap enough here, but a slower disk, a
  slower CI, or a cap/delay change can push the fill past the stall, at which point the resumed
  child settles before the 17th registration and the test's premise (still `Running` when the
  eviction decision is made) no longer holds. Carried here because every residual lands in
  Limits, not only in Gotchas.
- **`TaskOutcome::Running(snapshot)` also wraps settled status probes** (`:2848`): the variant
  says "this call produced no completion envelope", not "a run is active". Callers and assertions
  must read `snapshot.state` — this milestone's tests do.
- **A contained panic leaves the child un-runnable** (see Gotchas): the child's `running` flag is
  not cleared, so a later resume of that id answers `AlreadyRunning` until a rehydrate builds a
  fresh child. Fixing it belongs to `Agent::run`'s cleanup seam, not to the task registry. The
  residual is now shared by both contained bodies; the new foreground test pins the registry's
  terminal record and the returned error, not the child's re-runnability, and the foreground
  path queues no notice, so how the panic reaches the *parent model* (through the tool call's
  error result) is the tool envelope's concern, outside this slice's asserts.
- **A poisoned registry lock is still fatal to readers.** The spawn body's terminal write
  recovers the poison, but `task_snapshot`/`task_ids`/`task_snapshots` still answer
  empty/`None` on a poisoned lock (pre-existing shape, unchanged here), and `claim_task_running`
  refuses with "task registry lock poisoned" rather than guessing.
- **The probes ran on the working tree, not on candidate bytes** — the round owes them again on
  the frozen candidate (see Verification).
- **No racing test exists for the window the one-hold claim closes.** Its atomicity is structural
  (a single `Mutex` hold, like M87's admission slot); a deterministic single-thread test cannot
  stage two resumes interleaved inside the old check-then-set window, so the property is argued
  from the shape and pinned by the refusal test, not by a concurrent test.
- **CLI/TUI files are untouched**: core-only slice (`crates/epsilon-tui/**` carries another work
  item's uncommitted changes and was out of bounds), and the `/tasks` badge needs no change to
  follow the snapshot.
