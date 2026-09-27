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

- **D17 — subagent tokens never counted in the cost.** About money, silent, unbounded.
  `task.rs:231` produces usage → `exec.rs:736` carries it → `state.rs:1022` never reads
  it. **This is the next thing to fix.**
- **D19 — a panic leaves the terminal unusable.** No `Drop` guard, no panic hook.
- **D21** — `epsilon-lsp` compiles without `unsafe_code = "forbid"`. One line.
- Phase 0 leftovers: 0.6 promote the architecture draft; 146 links in `audit/history/`.
- Phases 4 (port parity) and 5 (structure) not started.
- **A full gate battery has not been run this session** — individual crates only.
- The installed `C:/Users/Hp/.cargo/bin/epsilon.exe` is still the old build. Reinstall
  before the user tests, per AGENTS.md.

## Traps on this machine

- Build owner is one at a time. `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`.
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
