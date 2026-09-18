# Next hardening slice

## Baseline now reproducible
- b57c8bf integrates the required companion sources forward. All 169 committed
  source/manifest/script/asset files match the isolated verified candidate.
- Gates: 740 passed, 0 failed; all five gates green; locked real/offline checks pass.
- Candidate install and installed demo passed; cleanup removed 11.0 GiB.
  Both repo/candidate targets absent at closeout, C: 60G free.
- Evidence and residual risks: docs/milestones/M83-baseline-integration.md,
  audit/evidence/integration-*-review.json. No release/security certification.
- The earlier 794-test result describes broader uncommitted owner work, not HEAD.

## Next
- Queued fidelity slices, in order (owner: port first): **F20 mid-batch abort**
  (implemented 2026-09-18, unverified, rides in round B — see ledger for the rule),
  then CORE-02 callbacks, then SESSION-01 tree/fork workflows, then RPC-02 protocol
  breadth, then F09 resolved containment. Serve-mode admission (F13 HTTP half) is a
  small follow-up: `run_depth` can become the atomic reservation.
- Continue approved Phase 3 per audit/04-roadmap.md: F09 path identity is the
  remaining Batch B item. Read ledger/source first; use inert defensive tests.
- Canonicalization is not race-resistant access. Windows/Linux differences must
  remain explicit. No exploit reproductions or paid API calls needed.
- Review newly recorded M83 residuals in context; do not silently drop them.

## Preserve owner work
- Broad TUI, registry sorting, contracts stream folding, core config/prompt assets,
  RPC/HTTP references and historical/local files remain dirty/untracked.
- Do not stage whole files containing excluded changes. Isolate candidate and
  commit only verified selections; check index against tested source contents.
- No push or history rewrite. Explicit integration approval was already granted.
- Original 313-file snapshot: C:/Users/Hp/AppData/Local/Temp/
  epsilon-integration-preserved-350c9ec/project-snapshot.zip; keep this backup.
- No builds or child agents running at closeout.
