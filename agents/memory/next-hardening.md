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
- Committed and closed 2026-09-18: **F20 mid-batch abort** (M88, 31b1794), **F04/F06/F17
  compaction correctness** (M86, 36a226c), **F13 admission, RPC + serve** (M87, 35da3d7).
  Round evidence: gate-m86-88f (751 passed / 0 failed, all five gates), red-round.log,
  m86-88-review.md. The global epsilon.exe was reinstalled from c4711a2 and its demo ran.
- Next port work, in order (owner: port first): CORE-02 callbacks, SESSION-01 tree/fork
  workflows, RPC-02 protocol breadth (correlation ids, abort/state/model ops, dropped
  events), then F09 resolved containment (alias-consistent identity, defensive only).
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
