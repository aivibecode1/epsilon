# M197: the baseline, and a real "frozen harness" defect in the extension host

**Status:** open. One defect found, fixed, and under verification. Baseline evidence at
`target/gate-m197-baseline.log` (that file is wiped by the next round — copy it if kept).

## The uncommitted tree at session start

M196's second half was never committed: the `!` shell escape (`epsilon_core::exec::run_local`,
`App::run_shell`), the page-key test, and the test-support additions. Verified working, not
verified *shipped*.

## Baseline battery (`m197-baseline`): GATE FAIL, two real defects

Measured, not read off a document:

| Gate | Result |
|---|---|
| check-links | PASS (380 links, 0 broken) |
| fmt --check | PASS |
| clippy | **FAIL** — `len_zero` at `epsilon-tui/src/app/tests.rs` |
| test | **FAIL** — 1 of 543+ in `epsilon-ext` (`crashes_accumulate_…`, a HANG) |
| demo | PASS |
| real-check | PASS |

The repo was left red. A "green M196" in `docs/history.md` did not describe this tree.

## Defect 1 (clippy, trivial)

`short_text.len() > 0` → `!short_text.is_empty()`. Mechanical.

## Defect 2 (real): a request to a dead extension waits the full invoke budget

**The shape of it.** `ExtHost::request` registers a `oneshot`, writes the frame, and waits for
the reader task to complete it. The reader calls `fail_all` when it stops, which wakes everyone
*already registered*. A request registered **after** the reader exited has nobody to wake it: the
frame goes into a pipe nobody reads, and it waits out `INVOKE_TIMEOUT_SECS` (10 s) and returns
`Timeout` — a lie, because nothing was slow.

**Why it matters beyond the test.** This is the reported "frozen TUI" symptom one layer down. A
harness with a dead extension looks exactly like a harness that has stopped responding, and the
crash budget never engages because the failure is charged as a timeout, not a crash.

**The fix** (`crates/epsilon-ext/src/host.rs`): an `Arc<AtomicBool>` `stopped`, set by the reader
on the way out (and *only* when it did not stop on our own shutdown signal), checked at the top of
`request` before anything is written. The shutdown case must not set it: a host being dropped is
not a broken host, and marking it would make a clean teardown look like a crash.

## The control passed first — and that is the finding worth keeping

The first version of the new test used the `--die` fixture. It passed. **So did the control** —
the fix reverted, the test still green. The test was structurally incapable of seeing the defect,
and the defect was live in the tree.

**Why.** With a dead child the *write* to its stdin fails immediately, so the call returns fast
whether or not the host knows the reader is gone. The bug only shows when the child is **alive**,
its pipe **accepts writes**, and only the reader is missing.

**The fixture that works** is `--flood 200000 100`: the child stays alive, the reader aborts on
the byte budget and exits, and every later request is in exactly the defective state.

This is the repo's own documented rule ("a control that passes is a control you have not yet
proven works") arriving a fifth time. The general form: **when the negative control passes, the
fixture is wrong, not the code** — ask what the broken build would print and confirm it differs.

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

## Machine notes

- `bash scripts/gate.sh` is still wrong here; `pwsh -File scripts/gate.ps1 <label>` works.
- `patch`/`write_file` report a flood of `E0670: async fn is not permitted in Rust 2015` on every
  edit. That is the **syntax checker running without `--edition 2024`** — pre-existing noise, not
  a real error. The repo builds clean with cargo; ignore it and read the diff.
- One build owner at a time. `CARGO_BUILD_JOBS=1`, target on `C:` in `%TEMP%\epsilon-target`.
- C: was at 95% (26 G free) at session start; the 13 G build dir is the thing to watch.
