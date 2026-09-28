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
  - **`after_tool_call` is deliberately NOT wired** — result rewriting is still missing. The
    seam is shaped for it. That is the obvious next slice of 4.5.
- **`branch_with_summary` still has no production caller** — confirmed three times now.
- **`TurnHooks` is still unwired** (10/10 setters in tests). The `ToolHooks` half is done;
  the `TurnHooks` half is not, and the interpose transport now makes it reachable.
- Phase 5 (structure) not started.
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
