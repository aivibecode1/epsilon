# Next hardening slice

## Baseline now reproducible

**This section is historical.** It describes the M83 candidate; the suite has more
than doubled since and the numbers below are not today's. The live baseline is
`audit/11-current-assessment.md`, and the verification command list is in
`AGENTS.md`. Kept because the *method* it records — freeze a candidate, hash it,
run the gates against those bytes, and keep the evidence — is still how a round is
verified here, and because deleting a worked example of it would lose more than
the stale numbers cost.

- b57c8bf integrates the required companion sources forward. All 169 committed
  source/manifest/script/asset files match the isolated verified candidate.
- Gates: 740 passed, 0 failed; all five gates green; locked real/offline checks pass.
- Candidate install and installed demo passed; cleanup removed 11.0 GiB.
  Both repo/candidate targets absent at closeout, C: 60G free.
- Evidence and residual risks: docs/milestones/M83-baseline-integration.md,
  audit/evidence/integration-*-review.json. No release/security certification.
- The earlier 794-test result describes broader uncommitted owner work, not HEAD.

## Since M83 (2026-09-20)

- Suite: **1,017 passed, 0 failed, 1 ignored** on the workspace; add ~45 s for
  `crates/epsilon-cli/tests/terminal.rs`, which drives the built binary through a
  real pseudo-console and is the only test that can fail because the program never
  reached a terminal. Invocation in `AGENTS.md`.
- TUI: `app.rs` 6,700 → 3,594 lines, with the keyboard model, the event loop and
  the test modules beside it; `ui/` owns everything that draws. See
  `agents/memory/tui-redesign.md`.
- The owner-reported "shrink and freeze" was the bash tool spawning
  `powershell -Command` without `-NoProfile` — 6.2 s per call. Not a rendering bug.
- **A protocol test must not fail because the machine was slow** (fixed 2026-09-20).
  `a_child_that_never_says_hello_times_out_at_spawn` flaked because the fixture's
  silence was *finite* (30 s) while the test's outer bound was 20 s, so on a loaded
  machine the bound fired before the host's 10 s handshake deadline. Measured on this
  box: process start-up 1.7 s, spawn+teardown of a silent child 23.4 s under load,
  and the host itself timing out correctly at 10.03 s. The fixture now stays silent
  *forever* (no race to lose) and `MANIFEST_TIMEOUT` is 45 s with the measurements in
  its doc comment. Suite went from wedging at ~30 s serial to 14 s parallel: 22/22.
  Method note: the root cause came from a throwaway phase-timing probe test, not from
  reading the code — instrument before theorising.


## Operating notes (this machine)
- One build owner; batteries run with `CARGO_BUILD_JOBS=1` and an external
  `CARGO_TARGET_DIR` (every failure at parallel jobs was 0xc0000142 compiler startup;
  single-job + external has passed every round since).
- **Give every battery attempt its own label** (`gate-m86-88b`, never reuse): the wrapper
  truncates `target/gate-<label>.log` on start, so reusing a label destroys the failed
  attempt's evidence. (Learned 2026-09-18 — an attempt-1 log was lost that way. The rule
  is now in AGENTS.md under "Verification rounds on this machine", so this note is the
  story rather than the only record.)
- `kill -0` is unreliable on MSYS for liveness — use the process tool or log growth.
  To stop a battery, take the WINPID of the MSYS wrapper from `ps -W` and run
  `MSYS_NO_PATHCONV=1 taskkill /F /PID <WINPID>`; the process tool's kill can time out
  and leave cargo running.
- MSYS `sha256sum` prefixes a `\` to paths that contain backslashes, so comparing
  `sha256sum "$a" | cut -c1-16` against `sha256sum "$b" | cut -c1-16` gives a false
  MISMATCH whenever one side came from `$LOCALAPPDATA` (which expands with backslashes).
  Byte-verified comparison therefore hashes through forward-slash paths and strips the
  escape: `sha256sum "$f" | tr -d '\\' | cut -c1-16`. (Cost 2026-09-18: the M92 candidate
  assembly aborted on a spurious mismatch — the bytes were identical.)
- PowerShell process sweeps (`Get-CimInstance Win32_Process | Where-Object { $_.CommandLine
  -match "pattern" }`) **match the sweeping process itself** when the pattern appears in its
  own command line — it killed its own shell and the report was lost. Split the literal
  (`-match ("rpc0"+"2")`) or filter on `Name` as well, and restrict the sweep to the process
  names you mean (`cargo.exe`/`rustc.exe`).
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
- A round script's `echo "BATTERY_EXIT=$?"` sentinel is unreliable: if the battery process is
  killed mid-run (e.g. by a kill sweep), the echo can still print 0 while `gate-<label>.log`
  stops mid-gate. **The gate log is the authority, not the sentinel**: require the log to end in
  `===== GATE PASS =====` (or a gate `[exit=` line for the failing gate) before believing any
  round result. Seen 2026-09-18: an 84-byte log (fmt done, clippy started) alongside
  "BATTERY_EXIT=0".
- **Dispatch a fix round once.** A slice's follow-up work must have exactly one writer: before
  dispatching, list the running children (`delegate_task action='list'`); before assuming a
  round finished, check the tree, not the message. Seen 2026-09-18: the same M94 fix brief went
  out twice (two children, ~50 calls each, both editing app.rs) and had to be reconciled
  afterwards — the round script for such a recovery should grep for duplicated `fn` names before
  building.
- **TUI: a replaced approval is denied silently.** `drive`'s ask arm replaces `app.pending_ask`
  (app.rs ~:2786): both the unpainted and the painted-but-unanswered case fail closed to Deny
  (channel drop semantics; verified, cannot stall), but the user gets no notice — the dialog
  content just swaps. Cheap fix, queued: push a notice at the replacement site. Batch it with the
  run-loop ask FIFO (seat one ask per frame) so it costs one battery cycle, not two.
- **`docs/milestones/M33–M59` exist only as untracked files** (≈27 historical logs). They are real
  project documentation that a `git clean` would destroy. They need their own disclosed docs
  commit (spot-check then state the scope) — deliberately NOT folded into the M94 code commit.
- **M95 follow-up (queued, delta re-review F2-F5; one battery cycle):** the `limit:0` "N more
  lines" count uses `total` where pi uses `total − startLine` (visible at `offset>1`); an empty
  file with `limit:0` is silent where pi emits its phantom notice; nothing pins `limit:0`; the
  ninth tests's refusal matches `Err(_)` not the `only read in part` diagnosis; the over-cap branch
  compares raw bytes vs pi's lossy-decoded length; `details.read.offset` says 1 on an empty page
  while the notice disagrees. None affects the read-coverage guard. Recorded in the M95 doc Limits.
