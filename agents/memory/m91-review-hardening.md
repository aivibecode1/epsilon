# M91 — review hardening (the M89/M90 follow-ups)

## State (2026-09-18, round in flight)
- Implementation complete in the working tree (deleg_4b740d43): A1 cycle guards, A2 seq-position
  guard, B1/B2/B3 loop tests, C **refused with reason** (anchor re-base needs `compact.rs`'s
  anchor shape + lifetime discipline). Files: `crates/epsilon-session/src/store.rs`,
  `crates/epsilon-session/tests/session_store.rs`, `crates/epsilon-core/tests/loop_scenarios.rs`,
  `docs/milestones/M91-review-hardening.md` (new). `epsilon-core/src/lib.rs` deliberately
  untouched; `crates/epsilon-tui/**` untouched.
- **Battery `m91a` running** (`proc_d6c0286728e3` chain) on the frozen candidate
  `C:/Users/Hp/AppData/Local/Temp/epsilon-m91a-candidate` (= HEAD `ff7e8cc` + those three code
  files + the doc, byte-verified), external target `$LOCALAPPDATA/Temp/epsilon-m91-target`,
  `CARGO_BUILD_JOBS=1`, `CARGO_PROFILE_TEST_DEBUG=1`.
- **My probes executed** on a disposable copy (`epsilon-m91a-negctl`), log preserved as
  `audit/evidence/m91-probes.log` (sha256 `05cc3915…`): P1 seq guard reverted → fails with
  `store.rs:726 index out of bounds (len 3, index 7)`; P2 cycle guard reverted → process dies
  with `STATUS_STACK_BUFFER_OVERRUN` (unbounded growth; the implementer saw a wall-clock hang
  with both guards reverted); **P3 overflow-pop reverted → suite still green** (so the pop is
  defensive, not observable — recorded, not smoothed over); P4 attach epoch bump reverted →
  `attaching_a_store_mid_run_invalidates_the_replacement` fails on the M89 fix's assertion.
- **Independent review complete** (deleg_3463cc68): all six items CONFIRMED; one
  documentation-only blocker (the "nothing in-tree consumes this" claim was false —
  `repair_on_open` + search walk the chain, so a corrupt file resumes silently shorter) **fixed**
  in the milestone limits; findings dispositioned in `audit/evidence/m91-review.md`; the
  implementer's byte-identity restore verified here (`store.rs` `7ff4cd15…`,
  `session_store.rs` `279cd366…` — exact matches).
- Implementer's own controls (seq guard → the named panic; cycle guard → no termination) and its
  focused run (session 16+27, core 200 across 15 summaries) are in its report; my verification
  ran in the same chain before the probes.

## Remaining to close M91
1. Battery result → fill the milestone's `Verification` section from the gate log (reconcile the
   per-binary test-count delta), copy the log to `audit/evidence/gate-m91a.log`.
2. `docs/history.md` row (test total from the battery).
3. Commit: three code files + the doc + `audit/evidence/{m91-probes, m91-review, gate-m91a}`
   + `docs/README.md`, `docs/history.md`, the M89/M90 cross-references, `agents/memory/*`.
   No index surgery needed (all files slice-owned).
4. Cleanup: candidate + negctl copies + the m91/m91b logs + the round target; record
   `audit/evidence/m91-cleanup.log`; retire this note; update `port-revalidation.md`.
5. No reinstall: nothing in-tree calls the changed APIs with new arguments (the store walks keep
   their signatures) — state that in the milestone.

## Then
RPC-02's implementation (brief banked in `agents/memory/rpc02-protocol.md`) dispatches once the
machine is free.
