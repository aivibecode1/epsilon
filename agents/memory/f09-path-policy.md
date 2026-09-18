# F09 — resolved path policy

## Starting point
- HEAD 80b32eb after verified baseline integration b57c8bf; index empty.
- No edits yet to core paths/exec or tools files/util (checked git diff).
- Shared resolver core paths.rs:13 returns lexical (PathBuf, inside).
  tools util.rs:125 reexports it. Comment incorrectly delegates native symlink
  safety to sandbox; native filesystem calls are not surrounded by it.
- F09 remains open: read/write/edit/search/attachments/checkpoint identity must
  agree. Canonicalize-then-open is not race-resistant containment.

## Current bounded slice
- Map completed: audit/evidence/f09-path-policy-map.md, reviewed by parent.
- Implement find directory resource key using existing search_key helper;
  this fixes only a scheduling omission. Resolved containment stays open.
- Added regression in tools/src/find.rs (previously clean): default root,
  normalized explicit roots and directory-prefix conflict expectations.
- RED test running in isolated HEAD + test export:
  C:/Users/Hp/AppData/Local/Temp/epsilon-f09-find-candidate.
  Process proc_20e1403ea6a5, log audit/evidence/f09-find-red.log.
- RED observed: expected directory key, actual None (exit 101).
- Production fix: FindTool::resource_key delegates to existing search_key;
  helper visibility becomes pub(crate). Only find.rs/search.rs source modified.
- GREEN: regression 1/1 and tools library 78/78 passed in isolated candidate.
- Full battery finished FAIL: fmt/clippy/demo/real-check exit 0; test exit 101.
  auth::tests::concurrent_saves_keep_every_provider timed out acquiring temporary
  auth.lock; unrelated to find changes. Evidence: audit/evidence/gate-f09-find.log.
- Cleanup completed exit 0; candidate target absent, C: 60G free.
  Evidence: audit/evidence/f09-find-cleanup.log. No builds remain running.
- No commit/install yet. Auth source read only; no changes or ignored tests.
- Unchanged repeat completed: gate-f09-find-repeat.log ends GATE PASS.
  Repeat cleanup log reports 14,923 files / 8.8 GiB removed. Prior process
  references are historical, not running jobs in the current audit.
- Independent review completed: audit/evidence/f09-find-review.json approves
  the bounded source change; it does not approve broad dirty work or containment.
- New owner request starts audit-first full-port revalidation; see
  port-revalidation.md. No F09 commit yet; candidate identity still needs checking.
- Before commit: complete independent review, ADR/milestone/history/ledger docs,
  exact candidate match, required install/cleanup. F09 containment remains OPEN.
- One build owner, no concurrent builds. Preserve logs and clean at round end.

## Current closeout — 2026-09-18
- Narrow scope selected: finish find scheduling commit; TUI work paused.
- Parent compared normalized find.rs/search.rs against prior isolated candidate:
  exact matches. Previous independent review still describes the 27/0 + 4/1 diff.
- HEAD d9c1810 differs from old tested 80b32eb only in follow-up default and queue
  tests (plus docs). Fresh archive HEAD + exactly find.rs/search.rs at
  C:/Users/Hp/AppData/Local/Temp/epsilon-f09-closeout.
- Full gate finished FAIL with jobs=2: fmt/clippy/demo/real-check passed;
  test build failed starting rustc for epsilon-cli (0xc0000142 DLL_INIT_FAILED).
  Process manager reports killed; durable gate log nevertheless ends GATE FAIL.
  Evidence audit/evidence/gate-f09-closeout.log. No current-HEAD test pass.
- No cargo/rustc processes remained. Cleaned 4.7 GiB; 60 GiB free. Evidence
  audit/evidence/f09-closeout-cleanup.log. No install/commit. No further broad
  rebuilds until compiler-startup failure is diagnosed; TUI remains paused.
- Review deleg_4a18d7c6 APPROVED: candidate/main match, planner/trait/tool
  dependencies unchanged since 80b32eb, old review applies; no builds by reviewer.
  Current-HEAD gate failed at compiler startup; no pass claimed.
  F09 containment/alias/replacement-race guarantees explicitly excluded.
- Pre-existing M84 docs diffs reviewed: docs map/history/ADR and audit ledger/log
  contain only this scheduling slice. M84 install wording corrected; no reinstall
  needed for a library-only slice. Direct rustc -vV succeeds (1.97.1); the
  intermittent DLL startup failure remains unexplained. Current blocker in audit/05.
- Narrow diagnostic now running: cargo build -p epsilon-cli --lib -j 1, same
  isolated HEAD+find candidate, default features preserved; no workspace tests.
  proc_00a49fb6a4da / pid 17964, notify enabled; log f09-cli-single-job.log under
  audit/evidence; target C:/Users/Hp/AppData/Local/Temp/epsilon-f09-compiler-probe.
  This tests compiler-startup at one job, not acceptance. Preserve result and clean
  after exit; no commit/install or broader rebuild until diagnosis supports it.
- F09 resolved containment remains OPEN. Do not retire this topic after scheduling
  commit: next defensive design task is alias-consistent identity, not an exploit.
- 2026-09-18 round 2: fresh isolated candidate `C:/Users/Hp/AppData/Local/Temp/
  epsilon-f09-r2` = `git archive d9c1810` + only find.rs (sha256 c7fc7957…) and
  search.rs (87800abf…), byte-identical to working tree. Battery launched with
  `CARGO_TARGET_DIR` external and `CARGO_BUILD_JOBS=1` (single-job probe build
  previously finished clean after jobs=2 hit 0xc0000142). Run: proc_641c4474afaa,
  log `gate-f09-r2.log` inside the candidate. Pass ⇒ commit M84 slice + docs.
