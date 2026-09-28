# Gate battery `m180-1` — PASS

**Date: 2026-09-28. Commit: `cac8c35`. Log: [`gate-m180-1.log`](gate-m180-1.log).**

Run with `pwsh -File scripts/gate.ps1 m180-1`, one build job, `CARGO_TARGET_DIR=D:/eps-target`.
The log is committed alongside this file. It is the one exception to the `audit/evidence/*.log`
ignore rule (see `.gitignore`), because a claim that cites a log is not traceable if the log
is not in the repository.

## Result

```
===== fmt --check =====   [exit=0   2s]
===== clippy =====        [exit=0 153s]
===== test =====          [exit=0 600s]
===== demo =====          [exit=0  56s]
===== real-check =====    [exit=0  49s]
===== GATE PASS =====
```

**The battery reports 1315 passing test results across 63 binaries. The real number is
1253, and the difference is worth explaining rather than rounding away.**

`tests/builtin_tools.rs` (62 tests) appears **twice** in the log — the battery's
`--workspace` run executes it, and something in the battery then runs it again. So 1315
double-counts 62 tests.

Per-crate totals, measured directly with a `cargo test -p <crate>` sweep
(`epsilon-llm` is 101, not the 74 a `--lib`-only run reports):

| Crate | Tests |
|---|---:|
| `epsilon-tui` | 380 |
| `epsilon-core` | 263 |
| `epsilon-tools` | 168 (106 lib + 62 `builtin_tools`) |
| `epsilon-llm` | 101 |
| `epsilon-session` | 94 |
| `epsilon-safety` | 55 |
| `epsilon-contracts` | 34 |
| `epsilon-ext` | 33 |
| `epsilon-lsp` | 23 |
| `epsilon-server` | 15 |
| `epsilon-cli` | 81 (incl. the 8-test terminal suite) |
| `epsilon-mcp` | 6 |
| **Total** | **1253** |

`epsilon-safety`'s `bwrap_unix.rs` contributes 0 on Windows: it is `#![cfg(unix)]` and
runs in CI on ubuntu-latest. That is the one place where this machine's total and CI's
differ by design.

The `real-check` gate is `cargo check -p epsilon-cli --features real`. The flag is
redundant — `real` has been a default feature since M63 — but harmless, and the gate is
kept so the real-provider path cannot rot silently.

## What this battery covers that the per-crate runs did not

Every count before `cac8c35` came from single-crate runs, which cannot catch a
workspace-level break. This one does, and it ran clean first time — including the two
cross-crate changes that were the riskiest part of the session:

- **`epsilon-lsp` now inherits `[lints] workspace = true`** (`dda2b54`). This adds
  `unsafe_code = "forbid"` and `clippy::all = "warn"` to a crate that had neither. It
  compiled clean, so no `unsafe` had crept in while the crate was unprotected — which is
  the answer the change was made to get.
- **`epsilon-core` gained `with_history`** (`fc9ed3b`), and `epsilon-tui` began calling
  a new `record_tool_usage` path (`12ae518`). Both change behaviour in crates that other
  crates depend on, and both are now exercised by the full suite rather than by one
  crate's own tests.

## The gates that could not have caught the session's actual bugs

Worth being explicit, because it is the argument for the probe work:

- **D20 (the stale session lock)** is a 30-second wait. Every test that acquires a lease
  on a clean directory passes, and so did all 1253 of these. It was caught by killing a
  real TUI and measuring the next launch.
- **The reported typing-freeze** needed `--stream-pace-ms` (`f318bc5`) to be reachable at
  all. Without that flag every harness in the repository drives a turn that finishes
  before the first keystroke.
- **D19 (a panic bricking the console)** is a broken *console*, and a test harness that
  breaks its own console takes every other test with it. `the_terminal_guard_restore_is
  _reachable_and_idempotent` pins the mechanism; only
  `console-probe.py --crash` pins the outcome, and its negative control is in
  [`tui-panic-restore-2026-09-28.md`](tui-panic-restore-2026-09-28.md).

A green battery means the code compiles, is lint-clean, and its tests pass. It does not
mean the thing a user complains about is fixed. Those are different questions, and the
session's real results came from the second kind.

## Not covered

- **`--no-default-features`.** Found by a per-crate sweep *after* the first battery
  reported 1315/1315 (really 1253): `cargo test -p epsilon-llm` did not compile, because
  `usage_arithmetic` calls `providers::test_support` behind `#[cfg(feature = "real")]`.
  Pre-existing (M140), and invisible here because `real` has been a default feature since
  M63 — no gate builds without default features. Fixed in `57812ca` with
  `required-features`, which skips the test honestly rather than compiling to a green line
  that ran nothing.

  The sweep is worth keeping as a record, because the number it produced is the evidence:
  it reported **`epsilon-llm: 0`** — not a failing count, a *silently absent* one, since
  the sweep sums `test result: ok.` lines and a crate that does not compile produces none.
  A count of zero reads like "no tests" rather than "this crate is broken", which is
  exactly why the error only surfaced when somebody read the per-crate line closely. After
  the fix the same sweep reports **101**.

- The **installed** `C:/Users/Hp/.cargo/bin/epsilon.exe` was the pre-session build when
  this battery started. Reinstalled after it passed, per AGENTS.md, since CLI/TUI
  behaviour changed.
- Real providers. The battery is offline by design; paid-API checks stay `#[ignore]`d
  behind env vars.
- 146 broken Markdown links in `audit/history/` — deliberately unrepaired, documented in
  [`../03-defects.md`](../03-defects.md).

**The lesson, stated so it is not lost:** a green battery is a statement about the
configurations the battery builds. This one builds exactly one. Everything it says about
`--no-default-features` is nothing, and a per-crate sweep said otherwise within minutes of
it finishing.
