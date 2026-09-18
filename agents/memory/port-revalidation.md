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

## Next safe actions
- Owner redirected priority to TUI enhancement; see tui-enhancement.md. F04 investigation deferred without code changes.
- M85 docs/evidence closeout accompanies this note; inherited M84 hunks in
  docs map/history/ADR/verification log are excluded. Queue work note retired.
- Audit initial phase is complete, not production readiness. Maintained-doc
  rewrite incomplete; keep history as history rather than rewriting it as success.
- Next port work: queue breadth (explicit modes/events/tool cycles) or compaction
  correctness from core review; choose one reproducible behavior before editing.
- Still open: callbacks, compaction/abort/session correctness, RPC admission,
  attachment/TUI/provider/extension parity. General improvements deferred.
- Gate before future code commits; use isolated HEAD candidates to avoid mixing
  inherited source with the verified slice. Review once, act on blockers, converge.
