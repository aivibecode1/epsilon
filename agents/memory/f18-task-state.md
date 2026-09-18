# F18 / M93 — task state when resuming

## State (2026-09-18)
Implementation dispatched as **deleg_40696843** (M93) and **landed in the working tree**: nothing
committed yet. Brief was read-only (deleg_ad748aeb, verified at HEAD `fec032a`). A review round
(same day) fixed the review's documentation blocker (the false two-live-children claim — the M61
store lease refuses the second open, so a running evicted child goes untracked) and the uncontained
foreground leg; working tree only, nothing committed.

## Implementer round (deleg_40696843, 2026-09-18) + review round (same day)
- Fix in `crates/epsilon-core/src/lib.rs`: `claim_task_running` (:2787-2802, one lock hold, called
  at :2859 after the child lookup / before the background split; the racy guard at :2790-2793 is
  gone), `contain_panic` (:308-338, `contain_hook` is now a wrapper at :294), spawn body contained
  at :3046-3109 with a poison-recovering terminal write (:3089-3091). Review round: the foreground
  body is contained too — `await_task_run` (:2952-3031) wraps its fallible work in `contain_panic`
  and its panic arm writes the same terminal `Failed` record before returning the error.
- Tests in `crates/epsilon-core/tests/task_scenarios.rs`: the five briefed scenarios plus
  `a_panicking_resumed_run_lands_failed_not_running` (the wrap's own regression — nothing else
  exercised it) and, from the review round, `a_panicking_foreground_resume_lands_failed_not_running`
  (the foreground leg's containment). Round-2 focused run green: fmt exit 0; clippy exit 0 (three
  named crates, `--all-targets -- -D warnings`); `cargo test -p epsilon-core` 207/0/0 across 15
  summaries — run under `CARGO_PROFILE_TEST_DEBUG=0` because Avira quarantines this machine's
  debug-built lib-test exe as HEUR/APC.Misc (per-target fallback also green: lib 108 relinked
  without debuginfo + integration 99 via cargo); `task_scenarios` 21/21. Log:
  `audit/evidence/m93-focused-round.log` (ROUND 2); raw logs: `audit/evidence/m93-round2-verify.log`,
  `audit/evidence/m93-round2-command.log`.
- Probes (round 2 in `audit/evidence/m93-probes.log`, superseding round 1): with the claim disabled
  the claim-dependent set reds — test 1 + the background panic test at the dispatch-claims-`Running`
  assertion, tests 2-4 (second-resume, abort, eviction) at that same precondition (each headline
  property is still entailed by the claim); **tests 5 and the new foreground panic test do not**
  (fix-shape/containment guards, not claim reds; the brief said all five would fail). The existing
  `resuming_a_running_background_task_is_refused_until_it_settles` also reds (proof the claim took
  over the guard). With containment disabled both panic tests fail (background never settles;
  foreground unwinds). Restores byte-identical (sha256 `f0789260…`, zero markers). Docs/ledger
  written (M93 doc Verification PENDING; F18 rewritten with corrected HEAD citations + refined
  impacts and the review round's corrections; F20 stamped with `gate-m86-88f` 751/0/0 and
  `31b1794`).
- Extra residual to carry: a contained panic still leaves the child agent's `running` flag set
  (un-runnable until rehydrated) — recorded in M93 Limits, belongs to the CORE-02 seam; now
  shared by the background and foreground contained bodies.

## The defect (verified, corrected citations)
*(Pre-fix HEAD `fec032a` line numbers — the M93 fix shifts the task region by +31 lines and
deletes the guard at :2790; the round's section above carries the post-fix anchors.)*
- Registry `TaskEntry` (`crates/epsilon-core/src/lib.rs:2260-2271`) in `Agent::tasks`, cap
  `TASK_REGISTRY_CAP = 16` (:2275); states `Running | Completed | Failed`
  (`crates/epsilon-contracts/src/tool.rs:238-245`).
- **The resume branch (:2768-2824) never writes `Running`**: the guard at :2790 ("still running —
  poll it…") is unreachable for a resumed child; the background arm spawns (:2802-2809) and
  returns `TaskOutcome::Running(now)` at :2813 carrying the OLD terminal state.
- Consequences (all real): abort cascades skip it (`abort_delegated_tasks` filter
  `e.state == Running`, :1130); eviction takes it (`register_task_entry`, :2692) and the running
  child then **goes untracked** — a later resume/status probe of that id fails on the child's
  still-held M61 store lease (`rehydrate_task` opens the file through `SessionStore::open`;
  reviewed and corrected: the earlier "rehydrates a second child from the same session file while
  the first still runs, duplicate execution" reading is contradicted by in-tree code — the lease
  refuses the second open), so no honest "still running" is available and the abort cascade
  cannot reach it, and only after the first leg ends (lease released) can the id rebind as a
  fresh child (identity loss); the background arm swallows the child's own `AlreadyRunning` and
  writes the entry `Failed` (:2990-2994) while it keeps running, so the parent model is told a
  lie.
- pi ships **no task tool** — the registry is Epsilon's own (M53); nothing to port, so no pi
  citations belong in this milestone.
- Recorded ledger citations were stale (anchored to revision `3afafd3`: abort 989, eviction 2234,
  guard 2334, spawn write 2532; drift +135 / +458). The child corrects them.

## The fix (as dispatched)
1. `claim_task_running(&self, id) -> Result<(), String>` — ONE lock hold: find (honest "no longer
   tracked" error) / refuse "still running" / set `Running`.
2. Called after the child lookup (:2800) and before the background split (:2801). Refusal, abort
   reach and unevictability then fall out of the state change — no separate guard, no change to
   the eviction selector.
3. **Panic honesty**: the detached spawn body (:2965) and the `if let Ok` lock write (:2987) must
   not leave a permanently `Running` entry — wrap the body like the existing `PanicGuard` (:986).
4. No public API change; `epsilon-tui/**` and AGENTS.md untouched.
Review round: item 3 grew the foreground `await_task_run` body too — the claim runs before the
background/foreground split, so both legs need the boundary (see the round section above).

## Tests (task_scenarios.rs) + docs
Five tests: running-before-dispatch-returns; second-resume refused without forging failure; parent
abort reaches the resumed child; survives eviction at the cap; failed run lands Failed (and the
rehydrate-failure path registers nothing); plus the two panic regressions named in the round
section above (21 tests total). Docs: `docs/milestones/M93-task-state-on-resume.md`
(Verification PENDING), ledger F18 rewrite + **F20 stamp** ("M88 fix … pending verification" is
stale — m86-88f passed 751/0/0, commit 31b1794).

## Owed by me after the child reports
Probes round 2 done (claim probe: 6 claim-dependent reds, each named in the probe log; containment
probe: both panic tests red; byte-identical restores). Review done: its findings — the false
two-live-children claim, the uncontained foreground leg, the Limits carries, the probe wording —
are addressed in this round (docs + ledger updated). Still owed: frozen candidate + battery `m93a`
(the machine's Avira quarantines the debug lib-test exe, so either add an Avira exclusion for the
build directory or run the suite with `CARGO_PROFILE_TEST_DEBUG=0` as this round did; expect 207
core tests), history row, then commit with its own evidence. Machine rules: `CARGO_BUILD_JOBS=1`,
external `CARGO_TARGET_DIR`, stop the battery if the bytes change.
