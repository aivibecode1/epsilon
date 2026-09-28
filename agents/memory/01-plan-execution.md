# Plan execution — phases 0-7

**Status: Phase 3 starting. Phases 0, 1, 2 largely done.**

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

- **Phase 4 (pi port parity)** and **Phase 5 (structure)** — not started.
- **0.6** promote `docs/design/epsilon-architecture.draft.md`.
- **146 broken links**, all in `audit/history/` — deliberately unrepaired.
- **Reinstall `C:/Users/Hp/.cargo/bin/epsilon.exe`** before the user tests it: the
  installed binary is still the pre-session build. Per AGENTS.md, CLI/TUI behavior
  changed, so a reinstall is required.
- **Wipe `D:/eps-target`** after the battery finishes (C: at 93%, 37 G free).

## Verified state

**Gate battery `m180-1`: PASS** at `cac8c35` — fmt, clippy, test, demo, real-check.
**1253 tests, 0 failures.** (The log's own 1315 double-counts `builtin_tools`' 62; the
per-crate sweep in `audit/evidence/gate-m180-1.md` is the measured breakdown.)

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
- **A test count of 0 usually means the crate did not compile, not that it has no
  tests.** A sweep that sums `test result: ok.` lines gets nothing from a broken build, and
  "0" reads as "no tests" instead of "this is broken". Always confirm a 0 by running that
  crate directly. This is how `epsilon-llm` hid a real compile failure.
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
