# Plan execution — phases 0-7

**Status: Phases 0-3 done, Phase 4 in progress. Defects D17-D21 all closed.**

Source of truth for *what* to do: `audit/07-plan.md`. This file tracks *where I am* and
*what I verified on the way*, so a context loss does not lose the position.

## Done

| Commit | What | Evidence |
|---|---|---|
| `47ca904` | Per-frame context-mirror measurement (test only) | 13.4µs@20, 144.3µs@200 — 10.8× for 10× |
| `b8b9b18` | The audit: 01–07, 2 retractions, defects D17–D21 | every claim names its command |
| `6c7a9d5` | 9 links repaired + `scripts/fix-links.py` | 146 left alone deliberately |
| `8907658` | 6 ponytail skills deleted, evidence recorded | — |
| `f318bc5` | `PacedProvider` + `--stream-pace-ms` | 125ms → 3875ms |
| `f9f1d25` | **Freeze measured: not reproducible** | typed 42 chars at `running 2s`, cmd + PowerShell |
| `b3fa012` | `typing_still_works_while_a_turn_is_streaming` | 8/8, negative control confirmed |
| `5d4e205` | **Stale lock fixed: 30s → ~4s** | real conhost, before/after measured |
| `fc9ed3b` | Transcript clone removed | 144.3µs → **276ns, flat** |

## The five audit defects — ALL CLOSED

| # | Defect | Commit | Verified by |
|---|---|---|---|
| D17 | Subagent tokens never counted in the cost | `12ae518` | 4 tests; negative control made 2 fail |
| D18 | Per-frame transcript clone | `fc9ed3b` | 144.3 µs → **276 ns, flat** |
| D19 | A panic bricks the terminal | `a4e9fa2` | Real conhost: control FAIL, fix PASS |
| D20 | A killed TUI blocks for 30 s | `5d4e205` | Real conhost: 30 s wait → none |
| D21 | `epsilon-lsp` outside the lint law | `dda2b54` | clippy clean; only the recorded exception remains |

Each was **measured**, not just fixed — that distinction is the whole point of this pass.

## The two answers that matter

**The reported typing-freeze does not reproduce.** Reproduced the *state* first
(PacedProvider makes a turn last seconds), then typed into it: 42 characters appeared
while the header read `running 2s · streaming…`, cursor advanced, output kept growing.
cmd and PowerShell, 100×30, debug build. Evidence:
`audit/evidence/tui-typing-during-stream-2026-09-28.md`.

**What DID match the symptom was the stale session lock (D20).** A TUI killed with
`taskkill /F` left a lock, and the next launch showed
`waiting for this session to be released — 4s elapsed, ~26s left` for **30 seconds**:
a live process, no input, no output. Fixed by heartbeat 10s→2s + a 4s settle window on
an unmoved lock. Safety proven: a live holder is never stolen from, checked in a unit
test *and* by watching a real TUI's mtime advance every 2s.

## Still open

- **Phase 4 (pi port parity)** — `/tree` re-projection (`7489c00`), delegated recheck folded
  in (`586cc09`), **4.3 done** (`e9d5b94`, ADR-019), **4.5a done** (`ece3af4`, transport),
  **4.5b done** (`9f73a5f`, `ExtensionToolHooks` in `epsilon-cli`). An extension can now veto
  a tool call; first production `set_tool_hooks` in the project's history. Milestone log:
  `docs/milestones/M181-extension-interception.md`.
  - **`after_tool_call` is now wired** (`2fb5219`) — result rewriting shipped, completing pi's
    tool-call event surface. `Option<AfterToolCallResult>`, `None` when nothing was modified
    (pi's `undefined`, and load-bearing: the loop uses it to keep the message byte-for-byte).
  - **Two divergences from pi, both deliberate and recorded** in `audit/02-port-fidelity.md`:
    (1) pi mutates `ToolCallEvent.input` in place with **no re-validation** — Epsilon's `args`
    crosses a process boundary so honouring it means re-validating, which pi does not do;
    porting the unsafe half is worse than the honest gap. (2) pi returns on the FIRST block;
    Epsilon asks all extensions so the reason names every objector. Veto semantics identical.
  - **`TurnHooks` shipped** (`570819e`) — the last test-only setter. **It is a PARTIAL port
    and that is the decision**: `transform_context`/`convert_to_llm` are **pass-through by
    design**, so extensions cannot reshape the prompt. Reasoning (not trust — the host is the
    user's own process and pi allows it): a malformed answer on that seam is a *silently
    corrupted conversation* with no error result and nothing to roll back, unlike a malformed
    veto. `prepare_next_turn` (model swap) and `should_stop_after_turn` ARE reachable.
  - **Fourth contracts seam wired; all four `ToolHooks`/`TurnHooks` methods are now
    reachable from a real extension process.** The "tested but never wired" defect class in
    `audit/03-defects.md` has no remaining instance on these two traits.
- **`branch_with_summary` still has no production caller** — confirmed three times now.
- **`TurnHooks` is still unwired** (10/10 setters in tests). The `ToolHooks` half is done;
  the `TurnHooks` half is not, and the interpose transport now makes it reachable.
- **M186 shipped 6 pi RPC commands** (`80a91b3`): cycle_thinking_level,
  get_available_thinking_levels, set_model, get_available_models, cycle_model,
  get_last_assistant_text. All mapped onto EXISTING core capability, so it was a wiring gap.
  Battery `m186` 1,425 / `m187` 1,427, both GATE PASS. Installed release verified over real
  stdio: 557-row model list, 365,819 bytes, one line, 0.08s. **22 of pi's 33 RPC commands
  remain**; the next cheap batch is get_messages, get_commands, get_session_stats,
  clear_queue.
- **M188 shipped 4 more RPC commands** (`30cb04c`): clear_queue, get_messages, get_commands,
  get_session_stats. **15 of pi's 33 now implemented.** get_session_stats needed real work: the
  session total existed only in the TUI's `UiState` (a VIEW), so RPC could not read it in
  principle. Added `Agent::session_usage()` + `session_usage_by_model()` — the per-model split
  is load-bearing because price belongs to the model that consumed the tokens.
  **D26:** pi's `follow_up` was refused as `unknown op`; only `followup` worked.
- **D23 CLOSED** (`f6636ff`): the 18 glued lines in `docs/history.md` repaired, all 18 rows
  reconstructed from the dev logs, and two structural tests added so it cannot recur
  silently (`38e7a5e`). D25 remains: one row numbered `| 018 |` that predates the corruption.
- Phase 5 (structure) not started. **Known Phase 5 work, newly found:** `docs/history.md`
  is corrupt below M160 (D23, in `audit/03-defects.md`) — milestone numbers M161–M171 sit
  on their own lines with their rows' leading `|` stripped, verified present before this
  session's changes, and the per-milestone suite table has no M161–M180 rows at all. Repair
  it by reading the milestone logs, with a mechanical check that every `M<n>` appears
  exactly once in a well-formed row. Do not invent milestone descriptions.
- **`branch_with_summary` still has no production caller** — confirmed twice, still true.
- **`ToolHooks`/`TurnHooks` are the same defect**: correct shape, loop calls them, 10/10
  setters are in tests. See the class note in `audit/03-defects.md`.
- **Phase 5 (structure)** — not started.
- **0.6** promote `docs/design/epsilon-architecture.draft.md`.
- **146 broken links**, all in `audit/history/` — deliberately unrepaired.
- **Wipe `D:/eps-target`** when the install finishes (C: was at 93%).

## Delegation: a child's summary is a claim, verify it before you act

- **A subagent can catch YOUR error, and will.** `deleg_bf96cf7a` found that
  `audit/02-port-fidelity.md` called branch navigation "Missing" when it is wired
  (`keys.rs:495`). It was right; I had written the claim. The audit searched for pi's
  identifier (`navigateTree`), found no `navigate_to`, and concluded absence.
- **Check whether the repo already refuted it.** Two evidence files
  (`port-gap-recheck-2026-09-21.md`, `F22-compaction-usage-never-read.md`) had
  already recorded navigation as wired. I superseded them without reconciling. Before
  publishing a verdict, grep `audit/evidence/` for the capability first.
- **`git worktree` is the right way to audit a tree a concurrent agent is editing.**
  The subagent used one at HEAD instead of reading my dirty files, and flagged my
  in-flight compile error rather than misattributing it. Worth doing again.
- **The subagent noticed a tool-state trap I was in.** It said "my probe is gone; the
  three modified files are the concurrent agent's" — i.e. it checked before blaming. Good
  discipline to keep in mind when reading a report that claims a defect.
- Ask a delegation to **compare two trees** (`references/pi-0.85.1` vs `crates/`) and
  explicitly *not* read the audit. Both tasks ignored the audit, which is the only reason
  they produced findings rather than paraphrasing my own conclusions.

## Verified state

**Gate `m180-final`: PASS** at `1790944` — fmt, clippy, test, demo, real-check.
**0 failures.** Log committed at `audit/evidence/gate-m180-final.log`; narrative at
`audit/evidence/gate-m180-final.md`. The log reports **1316 results across 63 targets** —
51 test binaries + 12 `Doc-tests` (each 0 passed).

**Never subtract a per-crate sweep from a battery total.** I published "1253 real tests,
1315 double-counts builtin_tools' 62" and it was false: parsing the log shows
`builtin_tools` runs once, the extra 12 lines are doc-test zeros, and the sweep and battery
were measured at different times against different target sets. Both numbers were real
measurements; the subtraction between them was not. See the correction in
`audit/evidence/gate-m180-1.md`.

Two batteries this session: `m180-1` (also PASS, committed) and `m180-final`.
Per-crate: tui 380 · core 263 · tools 168 · llm 101 · session 94 · safety 55 ·
contracts 34 · ext 33 · lsp 23 · cli 81 · server 15 · mcp 6.

Two things the battery does NOT cover, both found after it passed:
- `--no-default-features` — found a real pre-existing compile failure in
  `epsilon-llm/tests/usage_arithmetic.rs`; fixed in `57812ca` with `required-features`.
- Anything whose symptom is a *broken console* rather than a wrong value.

## Traps on this machine

- Build owner is one at a time. `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`.
- **A test binary can run twice in the battery.** `builtin_tools` does, so the log's
  total is 62 higher than the real one. Compare against a per-crate sweep before
  quoting any count.
- **Do not write a number you have not measured.** I invented a per-crate table from a
  `--lib`-only run, published it, and had to retract it. `cargo test -p X` ≠
  `cargo test -p X --lib`.
- **"Must not block" and "must not delay" are different requirements.** ADR-019 only wrote
  the first, so `interpose` reused `INVOKE_TIMEOUT_SECS` and a silent extension held every
  tool call open for 10 s — a 30-call turn would add 5 minutes. Found by a test that *expected*
  the policy to be enough. `INTERPOSE_TIMEOUT_SECS = 1` now. When writing a failure policy,
  ask what a *misbehaving* peer costs per call, not just whether it can stop the run.
- **`console-probe.py --exe-args "--crash"` is a FALSE PASS.** The probe sets
  `EPSILON_TUI_CRASH` in the child env itself (`console-probe.py:555`); passing `--crash` as
  an argument only makes the CLI print `unknown argument` and exit cleanly, so the console is
  restored and responsive *because nothing crashed*. Watch `crash_reported`, not the verdict
  string alone. Third time this session a control passed because the thing under test never
  ran (after the two reducer-test attempts).
- **Never dispatch a battery and then edit source while it runs.** It produced a `GATE FAIL`
  log describing a `u32`/`u64` mismatch at a line that no longer existed — the battery
  compiled the pre-fix snapshot (log 06:53, fix 06:47). The log is preserved as
  `audit/evidence/gate-m183-FAILED-stale-snapshot.log` and the re-run is `m183b`. AGENTS.md
  requires a fresh label per attempt; it does not say what to do when the source moves
  underneath a running battery, and the answer is: re-run, and keep the failed log named for
  what it is.
- **A test that calls the new function directly cannot see whether anything calls it.**
  Reproduced in *new* code this session: three prompt-template tests called
  `expand_template` directly and all three PASSED against a control that removed the call
  from `submit_editor`. The fix is one test that types the command and submits, through the
  only path a user takes. Keep the direct tests (they test the engine's contract) but never
  treat them as wiring evidence. Fourth occurrence of this class.
- **`docs/history.md` has TWO tables, and assuming one is a recurring error.** The
  `## Milestone timeline` runs to ~M186; `## Original acceptance criteria` is a second table
  restarting at M6. I scoped a check to one table (79 "missing" milestones, all false) and
  then scoped another to the whole file ("M6 after M186", also false) — same mistake, opposite
  directions. Check across both, or reset at every heading.
- **`#[serde(other)] Unknown` swallows `#[serde(alias = ...)]`.** A catch-all variant absorbs
  an unrecognised name before serde checks aliases, so the alias compiles, looks right, and
  silently does nothing. Use an explicit `#[serde(rename = "...")]` for the name that must
  work (D26, `follow_up`).
- **A protocol test must go through the wire.** Deserializing an enum proves the type parses;
  only dispatching a frame and reading the reply proves the protocol accepts what a client
  will send. `ops_deserialize_with_snake_tags` agreed with the code by construction and so
  could never have caught a wrong wire name.
- **An in-flight run drains both queues at its own pace.** A test that queues a follow-up
  against a RUNNING agent races the loop; measured, the item is gone before the next
  statement. Use an idle agent for queue assertions.
- **`docs/history.md` is `include_str!`'d, so editing it during a battery edits the source
  under test.** Caught by the m187 history test in m188 (`GATE FAIL`: "no row for milestones
  [187]"). Preserve the failure under a label; it is a truthful record, not a stale snapshot.
- **A control that does not remove what it claims to remove proves nothing.** Two
  instances this session: a parse default that was unreachable behind a digits predicate
  (control passed, correctly), and a directory-name drift control that replaced one of two
  occurrences (control passed, because my control was wrong, not the test). Renaming every
  occurrence failed it. Check that a control changed the live path before believing a pass.
- **Bash heredocs mangle commit messages containing backticks.** `` `@:` `` was read as a
  command and the text vanished. Write messages to a file and use `git commit -F` when the
  message contains markup — which is what AGENTS.md already says.
- **A doc-drift test needs anchored assertions.** `assert doc.contains("1")` passed a control
  that changed the documented budget; `contains("turn_end")` passed a control that renamed it
  to `turn_ending`. Anchor to the exact table cell and compare whole tokens. Sixth instance of
  this pattern.
- **A test that "runs" is not the same as a test that ran.** The CLI e2e tests locate the ext
  fixture by path and **skip** when it is absent. I built the fixture and checked the skip
  count was 0 — otherwise four green tests could be four tests that did nothing.
- **A concurrency test that has never failed is not a test.** Three now: `TerminalGuard`'s
  drop test (a unit struct's empty `Drop` is optimised away), and the ADR-019 forward-latency
  test. In the second case two earlier versions **passed against the control** — one asserted
  the answer arrived (the old design serialises, so it does, just late), one asserted a second
  request could be sent (it can, just after the first finishes). Neither measured the *wait*.
  The fix in both cases: assert on a **measured quantity** (elapsed time, observed effect) with
  a bound an order of magnitude away from the control's value, not on an outcome the broken
  design also produces.
- **A plausible mechanism is not a verified one.** I wrote "this deadlocks" into ADR-019 from
  reading `invoke`'s lock scope. It does not deadlock — the caller was its own reader. It
  serialises, which is bad in a different way (head-of-line blocking, measured at 9.74 s). Same
  family as publishing the 1315/1253 arithmetic. Write the mechanism down, then measure it,
  then correct the mechanism in place.
- **A background process can deliver a result for code that no longer exists.** One
  measurement dispatched at 00:33 (pre-fix) sat on disk and completed hours later, reporting
  `context_mirror_cost_does_not_track_transcript_length` FAILED at 16.8µs → 216µs (12.9x).
  That is the *pre-fix* profile exactly, and it contradicted the fix committed at 01:46 and
  the battery that passed 63 targets. Six clean re-runs were flat at ~300-500ns.
  **Before acting on any async result that contradicts a commit, check
  `git log -1 --format=%ad <sha>` against when the job was dispatched.** A late "FAILED" is
  evidence about the tree it ran on, not the tree you have. (Recorded as a near-miss: the
  mistake was briefly treating it as a regression, not as stale.)
- **A test count of 0 usually means the crate did not compile, not that it has no
  tests.** A sweep that sums `test result: ok.` lines gets nothing from a broken build, and
  "0" reads as "no tests" instead of "this is broken". Always confirm a 0 by running that
  crate directly. This is how `epsilon-llm` hid a real compile failure.
- **Never subtract a per-crate sweep from a battery total, and never publish "the real
  number is X".** I wrote that 1315 double-counted `builtin_tools`' 62 tests. Parsing the
  log showed `builtin_tools` runs once, the extra 12 result lines are `Doc-tests` reporting
  0 passed, and the sweep and battery were measured at different times against different
  target sets. A subtraction whose inputs were never measured against each other is a
  guess wearing arithmetic. To check a log, attribute each `test result:` to the
  `Running`/`Doc-tests` line above it — `builtin_tools` (62) and `session_store` (62) having
  identical counts is what made the false story plausible.
- **`MSYS_NO_PATHCONV=1 taskkill /F /IM epsilon.exe /T`** — without it, `//F` is invalid
  and the kill silently fails, leaving `epsilon.exe` locked (then `os error 5`).
- The **write_file / patch linter reports false Rust 2015 errors** (`async fn not
  permitted`, `let chains`). The crates are edition 2024; only cargo is authoritative.
- Wipe `D:/eps-target` after each round; C: is at 93%.
- `pwsh -File scripts/gate.ps1 <label>` is the battery that works here.
- Native tools need `C:/...` paths, never `/tmp`. Use
  `C:/Users/Hp/AppData/Local/hermes/cache/scratch` as `$S`.
- **read_file marks small files "partial view"** → `write_file` then refuses. Delete and
  rewrite, or use `patch`.
- `cmd` quoting in the probe: one opening quote, the path, args, closing `""`. Two
  wrong spellings both produce a usage dump that looks like a TUI failure.
