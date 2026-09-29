# M197: the baseline, and two real "frozen harness" defects

**Status:** two defects found, fixed, each with a working negative control, both committed.
- `bc6203e` — a request to a dead extension waited the full 10s invoke budget.
- `8846654` — every tool command inherited the TUI's console stdin (the Windows freeze/shrink).

## The uncommitted tree at session start

M196's second half was never committed: the `!` shell escape (`epsilon_core::exec::run_local`,
`App::run_shell`), the page-key test, and the test-support additions. Verified working, not
verified *shipped*. Still uncommitted — do not mistake it for lost work.

## Baseline battery (`m197-baseline`): GATE FAIL, two real defects

Measured, not read off a document:

| Gate | Result |
|---|---|
| check-links | PASS (380 links, 0 broken) |
| fmt --check | PASS |
| clippy | **FAIL** — `len_zero` at `epsilon-tui/src/app/tests.rs` (fixed, uncommitted) |
| test | **FAIL** — 1 hang in `epsilon-ext` (fixed, `bc6203e`) |
| demo | PASS |
| real-check | PASS |

The repo was left red. A "green M196" in `docs/history.md` did not describe this tree.

## Defect 1 (clippy, trivial)

`short_text.len() > 0` → `!short_text.is_empty()`. Mechanical.

## Defect 2: a request to a dead extension waited the full invoke budget

`ExtHost::request` registers a oneshot and waits for the reader task. The reader's `fail_all`
wakes everyone **already registered**; a request arriving after the reader exited has nobody to
wake it, so it waits out `INVOKE_TIMEOUT_SECS` and returns `Timeout` — a lie, since nothing was
slow. And because it is booked as a timeout it never spends the crash budget, so the host is never
disabled. Ten seconds of silence per call, indefinitely.

Fixed with an `Arc<AtomicBool>` `stopped`, set by the reader on the way out **only when it did not
stop on our own shutdown signal** (a clean teardown is not a broken host), checked at the top of
`request`. Error is `Crashed`, so the existing accounting disables a host that keeps dying.

Evidence: `audit/evidence/m197-ext-reader-hang.md`. Control (fix removed) fails naming the hang.

## Defect 3: tool children inherited the TUI's console stdin — the Windows freeze/shrink

`shell.rs` piped stdout/stderr and left `stdin` unset. **Unset means `Stdio::inherit()`.** In the
TUI that is the console in raw mode carrying the user's keystrokes, so every command the agent ran
was reading the keys the user was typing. Both reported symptoms follow: keys appear to do nothing
(freeze), and the child draws into the same console (shrink).

pi does neither thing Epsilon did — `nodejs.ts:533` passes `"ignore"` for stdin, `:534` sets
`windowsHide: true`. Epsilon did neither.

Fixed via `ChildStdio::configure`, which owns the three streams and returns what it decided:
`stdin` → `Stdio::null()`, plus `CREATE_NO_WINDOW` on Windows.

Evidence: `audit/evidence/m197-windows-stdin-inheritance.md`.

## Three controls, three different ways a test lies — the session's real lesson

1. **Wrong fixture.** The ext test used a dead child; the *write* failed fast, so the call returned
   promptly whether or not the host knew the reader was gone. The control passed with the defect
   live. The flooding fixture (child alive, pipe open, reader gone) is the one that isolates it.
2. **Wrong level.** A behavioural test for the stdin fix was green with stdin inherited, because
   `cargo test` gives the harness a *redirected* stdin — inherited and closed are indistinguishable
   from inside the suite. The property had to be asserted on the value the production path returns.
3. **Stale backup.** The "fixed" copy was taken after an earlier control had already stripped the
   line, so restoring it made the test pass against a flag that was false of the code beside it.
   Caught by reading the file, not the result.

The general form: **when a control passes, suspect the fixture, the level, or the backup — not the
code.** And a control should assert its own anchor applied before writing, and flip any flag it
depends on, so it cannot pass on a lie.

## The three things context/cache/cost already get right

Read, not assumed — all three are real and all three carry documented reasoning:

- `cache_hit_rate` uses numerator and denominator **from the same call** (G16). A session-total
  numerator pins the gauge at 100% and hides the thing being watched.
- `cache_reported` is carried from `Usage`, so `n/a` means *unknowable* and `—` means *none yet*
  (K1). Two different facts that used to render identically.
- `cost_is_floor` marks an unpriced-model total with `+` (G18), and `cost_summary` prices
  **per model** rather than at the active model's rates.
- `context_used` is the number the compaction trigger reads, not the last prompt's size, and
  carries a `*` when it is a heuristic rather than provider truth (C2).

**Do not rebuild these.** The remaining question is parity with pi, not correctness here.

## The TUI event loop is sound — do not "fix" it

`app/event.rs` already drains input before drawing, caps at ~30 fps, draws only when dirty, and
uses a `biased` select serving input first. The M59 fix for the reported input latency is real and
measured (99 chars took ~3 s before). The stdin defect above was the actual remaining cause.

## Machine notes

- `bash scripts/gate.sh` is still wrong here; `pwsh -File scripts/gate.ps1 <label>` works.
- `patch`/`write_file` report a flood of `E0670: async fn is not permitted in Rust 2015` on every
  edit. That is the **syntax checker running without `--edition 2024`** — pre-existing noise, not
  a real error. The repo builds clean with cargo; ignore it and read the diff.
- One build owner at a time. `CARGO_BUILD_JOBS=1`, target on `C:` in `%TEMP%\epsilon-target`
  (16 GB at last check; C: at 94%, 32 G free).
- A `cargo test` run of one `-p <crate> --test <name>` costs 3-5 min to compile. Budget it.
