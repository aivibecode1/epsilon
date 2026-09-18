# Autonomous audit and complete-port arc

## Authority and scope
- Owner authorizes audit first, docs/layout reconciliation, complete faithful pi
  port, then general reference improvements. Small autonomous commits, no push.
- Preserve inherited dirty source/docs; do not invent historical commits.
- .agent absent; repo skills live in .agents/skills. Read AGENTS and relevant skills.
- Use absolute paths and explicit terminal workdir; relative writes previously
  landed under a reference checkout while parallel agents were working.

## Verified baseline and boundaries
- Initial HEAD 80b32eb, 38 modified / 70 untracked. Broad TUI/contracts/config/
  registry/RPC changes remain unapproved and excluded from queue commits.
- Dirty-tree battery: all five gates, 795 passed / 0 failed / 1 ignored.
  Evidence audit/evidence/gate-revalidation-baseline.log. Not HEAD acceptance.
- Baseline cleanup removed 9.2 GiB. Never purge shared dependency/runtime caches.
- F09 find/search inherited slice: isolated repeat passed 741 tests; not committed.
  See f09-path-policy.md; do not bundle its docs or evidence into M85 closeout.

## Completed audit and first correction
- 13773b4: source-free fresh audit and three independent reference/source reviews.
- bcc1233: narrow README/AGENTS/roadmap/architecture corrections, not full rewrite.
- Keep conventional twelve-crate layout; no evidence for wholesale source moves.
- Reviews: audit/evidence/revalidation-{core,surface,structure}-review.md.
  Findings distinguish source inspection from runtime evidence; audit/09 is plan.
- 3afafd3: default followups All -> OneAtATime, matching pi agent.ts:231-232.
  RED expected 3 requests / actual 2; GREEN plus explicit All preservation 2/2.
- Code commit preceded full gate completion (process error; no owner request to
  skip gates). Subsequent isolated candidate matched both committed files.
- Candidate bcc1233 + two queue files: all five gates exited 0, 742 passed / 0
  failed / 0 ignored, 55 summaries. gate-queue-candidate.log is preserved.
- Final independent review queue-final-review.json passed with no blocking errors.
- proc_4dbf3a21f39f exited 0: cleanup removed 16,325 files / 9.4 GiB; repo,
  candidate and external targets absent; C: 60G free. No build running.
- No CLI/TUI source changed in this library slice; global binary not reinstalled.
- Durable M85 scope/evidence/limits: docs/milestones/M85-followup-default.md.

## M86-M88 round (2026-09-18)
- 36a226c (M86 compaction correctness), 35da3d7 (M87 admission reflects reality),
  31b1794 (M88 mid-batch abort), c4711a2 (round docs/evidence/memory).
- One round-B candidate, ten files byte-verified against the working tree; RED executed
  for six tests against the pre-fix code; GREEN battery gate-m86-88f: all five gates
  exit 0, 751 passed / 0 failed / 0 ignored across 55 summaries.
- Independent review (deleg_32066967): no blockers; four minor findings fixed (doc order,
  biased select, guard ordering, poisoned-history rollback) and three documented with
  reasons. Record: audit/evidence/m86-88-review.md.
- Attempts 1-5 failed first, each for a real reason now recorded: nine fmt diffs, E0716
  (temporary Arc in compact_now's select), E0004 (non-exhaustive match in admit_prompt),
  then a path-pinned AV denial on wire_fixtures (LNK1104 on a nonexistent file). Attempt
  6 passed with CARGO_PROFILE_TEST_DEBUG=1 (fresh test-artifact names) - disclosed in the
  verification log; no security configuration was touched.
- Cleanup removed the 15G shared target and both candidates; C: 59G free.
- CLI behaviour changed, so the global epsilon.exe is reinstalled from this commit before
  the round closes (proc_9dd1df275fb6); install evidence then the memory closeout.

## M89 / CORE-02 turn hooks (2026-09-18)
- 7e23e6e: pi's four `AgentLoopConfig` seams as `epsilon_contracts::turn::TurnHooks` (async,
  exactly like `ToolHooks`; defaults reproduce the pre-slice loop). ADR-002 and ADR-014 carry
  the amendments; the milestone records the pi deltas and residuals.
- Two blockers found by the first independent review are fixed and pinned: a **frozen
  replacement** (the loop now mirrors the turn's own assistant message, tool results, steering
  and nudges into an installed replacement — pi pushes onto `currentContext.messages`) and a
  **stale replacement after a committed mid-run compaction** (a history epoch is checked before
  each request; store attach/detach also bumps it, and the run's model/thinking snapshot
  survives). Hook panics are contained at the poll boundary.
- Battery `m89e` (candidate = HEAD `df5d3be` + the four slice files, byte-verified): all five
  gates exit 0, **759 passed / 0 failed / 0 ignored across 55 summaries**. No pre-fix RED is
  possible (the seams do not exist before the slice); a negative control reverts each fix in a
  disposable copy and the named tests fail as designed. Records: `audit/evidence/m89-review.md`,
  `m89-negative-control.log`, `gate-m89e.log`, `gate-m89a-attempt1.log`.
- Cleanup removed the 10G round target and every candidate; C: 59G free. No reinstall: no
  in-tree frontend installs turn hooks, so the shipped CLI is unchanged.
- Recorded follow-ups (M89 limits + next-hardening.md): unify `TurnModel` with
  `epsilon_llm::ModelRef`; re-base the usage anchor for replacement-built requests; tests for
  the drain-point mirror, the overflow-discard pop and the attach/detach epoch bump.

## Next safe actions
- Owner redirected priority to TUI enhancement; see tui-enhancement.md. F04 investigation deferred without code changes.
- M85 docs/evidence closeout accompanies this note; inherited M84 hunks in
  docs map/history/ADR/verification log are excluded. Queue work note retired.
- Audit initial phase is complete, not production readiness. Maintained-doc
  rewrite incomplete; keep history as history rather than rewriting it as success.
- Next port work (owner: port first): CORE-02 callbacks, SESSION-01 tree/fork workflows,
  RPC-02 protocol breadth (F13 remainder), then F09 resolved containment
  (alias-consistent identity, defensive only).
- General reference improvements remain deferred until the port has no open fidelity
  items (audit/04-roadmap.md Phases 3-4).
- Gate before future code commits; use isolated HEAD candidates to avoid mixing
  inherited source with the verified slice. Review once, act on blockers, converge.
