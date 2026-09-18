# Independent review — M93 task state when resuming (deleg_8f50fdd0) + fix round (deleg_ccb8406a)

Read-only review of the uncommitted slice. Verdicts 1–7: the single-hold claim VERIFIED, the
call-site ordering VERIFIED, the `contain_panic` refactor and M89's hook path VERIFIED (the M89
regression is green), five of six tests pre-fix discriminating with the sixth honestly admitted,
the ledger citations VERIFIED against the HEAD blob (the recorded ones verified at `3afafd3`), the
residuals present, and hygiene clean (nothing staged, `epsilon-tui/**` untouched, the evidence
logs' hashes and line numbers match the claims).

## Blocker (documentation) — FIXED at four sites

The slice claimed that after an evicted running child, a later resume "rehydrates a SECOND child
from the same session file while the first is still running — two live children, two writers on
one file". **Contradicted by in-tree code:** `rehydrate_task` opens through `SessionStore::open`
(`crates/epsilon-core/src/lib.rs:2498-2499`), and the store refuses a second open of the same file
in-process or cross-process (`epsilon-session/src/store.rs:231-237`, refusal `:292-299`,
`lease.rs:78-88`), so while the first child holds the lease the rehydrate is refused.

*Fixed* in the ledger's F18 impact bullets 1 and 3, the milestone doc, the test comment and the
memory note. The corrected consequence is worse in a different way and is now stated: **the running
task goes untracked** — a later resume or status probe fails with the store's lease error rather
than an honest "still running", and the abort cascade cannot reach it. What M93 closes is the
window itself: the claim writes `Running`, and the eviction selector (`:2723`) skips `Running`
entries, so the entry cannot be evicted while the child runs. The minor anchor nit (a post-fix
`rehydrate_task` range inside a HEAD-anchored paragraph) was corrected to HEAD anchors.

## Findings — dispositions

| # | Finding | Disposition |
|---|---|---|
| F2 | **The foreground leg was not contained**: `await_task_run` (unlike the detached background body) is not wrapped by `contain_panic`, so with the entry now claimed `Running` before the split, a panic there leaves a permanently-claimed, unevictable entry answering "still running" forever (pre-fix it kept a stale terminal state, so the claim genuinely worsened it) | **Fixed (code)** — the foreground body is wrapped in `contain_panic`; its panic arm writes the same terminal record as the background leg and returns the error. New test `a_panicking_foreground_resume_lands_failed_not_running` (task_scenarios 20 → 21). Probe B now reds **both** panic tests, which is the proof the foreground leg really goes through the containment (without the wrap, probe B could not affect it) |
| F3 | The eviction test's timing reliance (hardcoded private cap 16; 16 rehydrate probes must fit the 200 ms `SlowProvider` stall) was disclosed only in Gotchas | **Fixed** — carried into the milestone's Limits per the workflow rule, with the failure mode named (a slow box turns it into a false red) |
| F4 | "Tests fail for the reasons they name" was loose: probe A reds tests 1 and the background panic test at the dispatch-claims-`Running` assertion, and tests 2–4 at that same precondition rather than their headline assertion (each headline property is still entailed by the claim) | **Fixed** — the milestone doc and the memory note state exactly which assertion each test reds at and why the property still holds |

Accepted as-is (recorded, not defects): the sixth test (`a_resumed_run_that_fails_lands_failed_not_running`)
does not fail pre-fix — it is a fix-shape guard (no pre-fix RED is possible for "the claim is never
left unpaired by a failure path"), and the slice documented that honestly; and the new foreground
panic test likewise does not red under probe A (it is a containment guard, not a claim red).

## Environment finding from the fix round (carried into the milestone's Verification)

Avira false-positived the **debug-built core lib-test executable** (`HEUR/APC.Misc`; execution
denied, `LNK1104` on relink — the same class as the earlier `wire_fixtures` block). The fix round
worked around it with `CARGO_PROFILE_TEST_DEBUG=0` (fresh executable metadata), and the round for
the frozen candidate re-earned that with a pre-flight that picks the knob which actually executes
on the box. No security configuration was touched (owner/admin action, per AGENTS.md).
