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

## Operating notes (this machine)
- One build owner; batteries run with `CARGO_BUILD_JOBS=1` and an external
  `CARGO_TARGET_DIR` (every failure at parallel jobs was 0xc0000142 compiler startup;
  single-job + external has passed every round since).
- **Give every battery attempt its own label** (`gate-m86-88b`, never reuse): the wrapper
  truncates `target/gate-<label>.log` on start, so reusing a label destroys the failed
  attempt's evidence. (Learned 2026-09-18 — an attempt-1 log was lost that way; the rule
  belongs in AGENTS.md but that write needs owner approval.)
- `kill -0` is unreliable on MSYS for liveness — use the process tool or log growth.
  To stop a battery, take the WINPID of the MSYS wrapper from `ps -W` and run
  `MSYS_NO_PATHCONV=1 taskkill /F /PID <WINPID>`; the process tool's kill can time out
  and leave cargo running.
- Avira can pin a *specific* test-artifact path — seen 2026-09-18 on
  `epsilon-shared-target/debug/deps/wire_fixtures-76eddb95e46e81ff.exe`: linking then
  fails with `LNK1104` although no file exists, and the denial survives `rm`. Do not
  change security configuration from an agent session. Workarounds that keep the gate
  intact: let cargo relink under different metadata (e.g. a different feature set), or
  build the test profile with `CARGO_PROFILE_TEST_DEBUG=1` so every test exe gets a fresh
  name; disclose the knob in the verification log when used.
- Evidence out of `target/` before cleanup; clean after each round, never mid-round.

## Next
- Committed and closed 2026-09-18: **M89 / CORE-02 turn hooks** (7e23e6e) - pi's four
  `AgentLoopConfig` seams; two review-blockers fixed (frozen replacement, stale replacement
  after a committed compaction) and pinned by a negative control; battery m89e: 759 passed /
  0 failed, all five gates exit 0. Earlier the same day: M86 compaction correctness (36a226c),
  M87 admission (35da3d7), M88 mid-batch abort (31b1794), M84 find scheduling (d0ac16b).
- Next port work, in order (owner: port first): **SESSION-01b** (the TUI surface only — `/tree`
  listing every entry with a leaf marker, Enter's user-message→parent rule with editor-text
  restore, `/fork` through `fork_branch_at` + `switch_session_file` → `live_resume`; the store
  foundation landed in M90), RPC-02 protocol breadth (correlation ids, abort/state/model ops,
  dropped events), then F09 resolved containment (alias-consistent identity, defensive only).
  SESSION-01b is blocked on the TUI work item's uncommitted changes landing first.
- Recorded follow-ups from M89 (in the milestone limits, do not silently drop): unify
  `TurnModel` with `epsilon_llm::ModelRef`; **the anchor re-base is still open** — M91 (C)
  refused it with the exact reason and a sketch, and added the related precision gap that
  `attach_session_store`/`detach_session_store` bump the epoch without clearing the anchor; a
  correct fix needs `compact.rs`'s anchor shape plus lifetime discipline. **The three regression
  tests (drain-point mirror, overflow-discard pop, attach/detach invalidation) landed in M91**
  (`docs/milestones/M91-review-hardening.md`).
- Queued from M90's review: **the reader hardening landed in M91** — the walks stop on a repeat
  (the fork refuses a cycle) and a mismatched `seq` is a chain break, not a panic. Still open:
  whether a branch summary should survive a later compaction the way pi's entry-level window
  keeps it (today it is dropped with the replaced messages — the one non-pi-equivalent
  projection).
- Queued from M91's own review (`audit/evidence/m91-review.md`): (a) `pop_last` truncates the
  file but not `migrated_entries`, so open → pop_last → append serves a popped entry and breaks
  the chain — fix the bookkeeping and give `SessionStore::pop_last` a test (it has none
  anywhere); (b) the tolerant chain break is silent — let `repair_on_open` report a chain that
  stopped early and surface it in a frontend; (c) B2's store-side pop has no observable — attach
  a store in that test so the pop is pinned via `read_all`; (d) name the duplicate-ids shape
  beside the other two tolerant-stop triggers in the guard's doc comment; (e) the cycle test
  should hash the source file rather than assert a length.
- Then the audit/04-roadmap.md general-improvement phases; the port must have no open
  fidelity items first.
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
