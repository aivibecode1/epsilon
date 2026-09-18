# TUI enhancement — active owner priority

## Verified state
- Owner redirected work from port compaction to high-effort TUI. No F04 code changed.
- HEAD d9c1810. Initial 38 modified / 70 untracked status entries preserved.
- New source edits only app.rs: caret-visible grapheme composer, wrapping/height,
  minimum 60x12 fallback, remove double transcript scroll, combined history/live
  slicing with newline-preserving live tails. Six new TestBackend tests.
- Final package suite: 157 passed, zero failed (tui-final-focused.log).
- Final working-tree five gates: 803 passed, 0 failed, 1 ignored / 56 summaries;
  fmt/clippy/test/demo/real-check all exit 0. gate-tui-final.log ends GATE PASS.
- First battery failed formatting + aws-lc-sys native build; no dep/flag workaround.
  Formatted and repeated successfully. Preserve first failed gate evidence too.
- Cleanup completed: external target cleaned and repo target removed. See
  audit/evidence/tui-cleanup.log. No builds running; proc_6a9687447b09 exited 0.

## Approval routing follow-up (in progress)
- Added denial regression for palette/model-picker overlap; expected assertion RED
  in audit/evidence/tui-modal-red.log; moved existing approval key handler before
  timeline/palette/session/model pickers. Targeted GREEN in tui-modal-green.log.
- Removed a broken rendering-test scaffold: it awaited next_ask without a requester
  and guessed cursor APIs. red2–red5 logs are compile errors, not behavior evidence.
  Ratatui TestBackend exposes cursor_visible(); use that in future rendering tests.
- Full gate tui-modal finished FAIL: fmt/clippy/demo/real-check passed; workspace
  tests failed in aws-lc-sys native cl.exe with 0xc0000142. Durable evidence:
  audit/evidence/gate-tui-modal.log. No full-pass claim for this revision.
- Confirmed no cargo/rustc processes before cleanup. External m86 target cleaned,
  repository target removed; audit/evidence/tui-modal-cleanup.log records recovery.
  No install from failed battery. Next cold verification may limit build concurrency
  after diagnosing native compiler startup failure; never disable dependencies.
- Consumed read-only review deleg_7a27c598: retained denial test directly covers
  routing regression; main handler move matches recommendation. No reviewer builds.
- Reviewer reports candidate composer/transcript functions and six tests match main;
  candidate lacks routing fix/test. This is static comparison, not gate acceptance.
  Candidate: C:/Users/Hp/AppData/Local/Temp/epsilon-tui-m86-candidate.
- Still open: paste mutates input during approval, unconditional composer cursor,
  late autocomplete drawing; guidance and other approval decisions untested.
- Consumed static review deleg_6098c049: isolated HEAD routing fix needs only
  existing pending_ask block moved after Ctrl+C/Q, before palette/model picker;
  preserve auth precedence and HEAD test_app/key helpers; no new dependencies.
  HEAD has no timeline/session picker. Use fresh epsilon-modal-isolated archive,
  not rendering-modified m86 candidate, for routing-only verification.
- Guidance caveat: existing `e` branch clears shared input; moving it exposes that
  behavior over filters. Only plain denial preserves filters in the executed test.
  Allow/always-session routing changes too; no coverage or full isolation claimed.
- TUI work PAUSED for F09 scheduling closeout (agents/memory/f09-path-policy.md).
  No TUI commit/install yet; do not launch competing builds.

## M86 independent review: BLOCKED
- deleg_9ba6a9a5 rejected candidate: undersized fallback hides approval details
  without disabling approval keys (high priority); no actual approval executed.
- Five static logic objections: wrapped tab stop, full-row newline caret,
  HEAD scalar-editor/grapheme-renderer mismatch, frozen viewport live growth,
  inherited wrapper clipping long words. Rust regression evidence still required.
- Durable report audit/evidence/m86-candidate-review.json; no candidate gates run.
  Candidate includes five files: workspace manifest/lock, TUI manifest/state/app.
  Do not commit/install; first resume task must guard invisible approvals.

## Boundaries and next actions
- NOT committed or installed: app.rs contains thousands of inherited changes;
  whole-file stage/install would absorb unreviewed baseline. Need isolated HEAD
  candidate with only selected rendering hunks; verify before commit/install.
- Broad dirty-tree pass does not establish HEAD candidate acceptance.
- Start app.rs/wrap.rs snapshots: C:/Users/Hp/AppData/Local/Temp/epsilon-tui-start/.
  Scoped diff audit/evidence/tui-enhancement-change.patch predates final long-tail
  correction; regenerate before review. No wrap.rs edits.
- Audit/report: audit/10-tui-enhancement.md. Docs/ADR/milestone integration pending.
- Consumed deleg_65099c9d: intermediate-snapshot composer findings addressed in
  principal scope, not final-diff approval. Parent confirmed modal key/draw order
  mismatch and footer scalar-width/unapplied truncation. Disposition in audit/10.
- Next UX priority after current closeout: approval preempts picker/palette across
  key/paste/draw/hints/cursor, preserving filters/drafts; then narrow footer.
  Proposed inert acceptance tests are not executed evidence.
- Remaining: frozen view counts logical rows; footer priority; modal cursor/focus;
  keyboard/interactive terminal validation; composer boundary/tab edge cases.
- Next safe action: select exact candidate source,
  review rendering patch and finish bounded closeout before broad redesign.
