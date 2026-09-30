# Working note — the t2 ranked sweep, and what it changed about the queue

**Status:** t2 done; the TUI half of the queue is now **closed by M201**. The deliverable was
the **Reconciliation** section at the top of [`audit/03-defects.md`](../../audit/03-defects.md),
not a parallel file — a second ledger is a ledger that drifts. Frames:
`audit/evidence/frames/m199-baseline/` and `audit/evidence/frames/m201/`.

## What is left, in the order it should go

M201 closed A1, A2, A3, A4, A5, A6 and A13; **M202 closed a defect no row named** (a session
written before M194 could not be opened at all); **M205 closed A17** (a plain launch never said a
prior session existed). The rows still open, re-ranked by what each one costs:

| Row | What | Why here |
|---|---|---|
| **A20** | CLOSED at M214: the notice now counts the *other* session files in the session directory (`epsilon_session::list_sessions`, excluding this launch's own) and reports "this workspace holds N earlier sessions — `epsilon -c` opens the most recent". Verified off a real console: it says "1 earlier session" while the directory holds two files | What survives: the count is of files not conversations, and nothing prunes — the retention question, not a defect. |
| **A19** | CLOSED at M212 (the reorder plan 021 specified). --continue was opening two stores; the second landed on the empty file the launch had just created — thirty seconds of countdown and nothing restored. Verified against the installed binary: two files before `-c`, two after, zero countdown. What remains: **no automated test drives uild_runtime** | The one thing I would write next: a test for the resume open path, because a future reorder can reintroduce this with every gate green. |
| **A16** | DECIDED (round 15): a launch gets its own session file. The fixed `tui.jsonl` is a **divergence from pi** — `repo.ts:42-45` names one file per session as `<ISO>_<id>.jsonl` in `sessions/--<cwd>--/`, and `resolveNewSessionPath` runs on *create* — which is what makes `-c` ("newest for this workspace") meaningful at all. Execution: [`docs/plans/020-one-session-per-file.md`](../../docs/plans/020-one-session-per-file.md) | Needs a build. The user-visible win is the largest on the queue: today three launches share one lane, so `-c` hands the model all three conversations as one context and the header's `id` names only the first. |
| plan 020 | execute the four steps of [`docs/plans/020-one-session-per-file.md`](../../docs/plans/020-one-session-per-file.md) — step 1's formatter landed at M208 (`epsilon_session::time`), so the naming rule can be written without a second copy | **The largest user-visible win on the queue.** Needs a build; a full battery now costs **4.19 GB** with `CARGO_PROFILE_DEV_DEBUG=0`/`CARGO_PROFILE_TEST_DEBUG=0` (measured, written into AGENTS.md, disclosed per commit). |
| plan 019 | execute the four steps of [`docs/plans/019-one-config-reader.md`](../../docs/plans/019-one-config-reader.md) | Step 1 deletes the duplicate `global_dir` (verified: written byte-for-byte twice); step 2 removes `toml` from `epsilon-llm`. |
| M207 reinstall | the installed binary is the M206 build | `cargo install` needs a cold release build (~30-45 min) and refills ~1 GB. |
| A8 | **LANDED (M215)**: [`docs/milestones/M215-tools-lsp-inversion.md`](../../docs/milestones/M215-tools-lsp-inversion.md). The `Lsp` policy enum and the service construction moved to `epsilon_cli::runtime::diagnostics_for`; `register_builtins_with` takes `Option<Arc<dyn DiagnosticsProvider>>`; `cargo tree -p epsilon-tools -i epsilon-lsp` no longer matches a package and `epsilon-cli` is the only dependent. 1 new test with a reverted-fix control, battery `m215-final` PASS (1,634 tests / 65 binaries). |
| A9 | **LANDED (M216)**: [`docs/milestones/M216-line-endings.md`](../../docs/milestones/M216-line-endings.md). `* text=auto eol=lf` + a renormalization commit (both ignore-diffs empty) + the working tree refreshed; the two traps are recorded there (`checkout-index -a -f` is a no-op under the filter; a stale stat cache reads as a dirty tree). **Not enforced by any gate** — that is the open follow-up. |
| A14/A15 | secret scrubbing in the durable store, and the `read`/`@` asymmetry | **Documented** (round 13) in `docs/user/permissions.md`; what remains is the owner's choice on scrubbing. |

**A7 was already fixed and is now marked so** — the dead-holder steal has been in `lease.rs` since
before the ledger was written, with two tests on it, and the wait *is* drawn. Its remaining half is
A18. The lesson worth keeping: **a ledger row is a claim about code, and this one was written from
the constant rather than from the behaviour.** Reading `STALE_AFTER = 30s` and concluding "30 s" is
exactly the mistake the reconciliation passages exist to catch — the *constant* is not the
*predicate*.

`/goal` is complete as a capability (state at M203, driver at M204); its four follow-ups are in
[`08-goal-command.md`](08-goal-command.md).

## M202 was not a row, and it was the most valuable find of the two rounds

Driving the **installed binary** through `scripts/console-probe.py` — the only check in this
repository that runs what a user runs — found that `epsilon` would not start in this
repository's own directory: `Usage::cache_reported` was added without a serde default, and
`Usage` is inside every assistant message, so every session file older than M194 was
unreadable. No test could see it, because every fixture is built with the current shape.

**The tool to reach for next time is the probe, not another test.** It answers a question no
in-tree test can: *what does the shipped artifact do in a real console, in a real workspace?*
It costs one command and about ten seconds:

```sh
python scripts/console-probe.py --late --size 80x24 "C:\Users\Hp\.cargo\bin\epsilon.exe" ""
```

Two shapes of finding it can produce that nothing else can: a refusal at startup (this one), and
a frame that differs from what `frame_dump` renders (the rows come back as JSON). Run it after
every reinstall.

## The method fact worth keeping

A scratch probe (`wrap_line` over the exact fixture string, widths 56–61) proved
`wrap_line` is **content-intact at every width** — so the character loss in A1 was in the
*draw* path, and reading `wrap.rs` could not have found it, because the wrapper was correct
about content and wrong about *width*. The repo already had a property test asserting no
content is lost; it passed, and the bug was downstream of the thing it tested.
M201 added the assertion that was missing (`no_row_is_wider_than_the_width_it_was_given`,
widths 1..=len) rather than changing the one that passed.

**A test that proves a component is correct is not a test that the component is used
correctly.** That is the M198 lesson in a third costume.

## Two greps that would have produced false rows

1. `grep -rl "epsilon-core::" crates/epsilon-server/src` → **0 hits for all five
   deps**, because Rust crate paths use underscores. It looked like D2 confirmed.
   D2 is a *retraction*; the identifier was the error.
2. Counting `.unwrap()` across `crates/*/src` → 365 in the TUI, which says nothing:
   `#[cfg(test)] mod tests` is in the same files. Production unwraps need a
   rust-aware pass, not a grep. **Not attempted — recorded as a gap.**

## Gaps in this sweep, stated rather than hidden

- **No security advisory scan.** No `cargo audit`, no network. Dependency risk is
  therefore *unassessed* — exact pins are verified, their vulnerability status is not.
- **No `cargo deny`/license check.** Same reason.
- **The unwrap/panic surface in production paths is unmeasured** (see above).
- **D4's architecture-guide claims were not re-verified line by line.** The guide
  exists and cites 3 line references; whether they point at the right code is
  unchecked.
- **Port fidelity was not re-measured.** `02-port-fidelity.md`'s 38/22/24 split is
  inherited, not re-derived, and the goal is quality-first — so it stays inherited
  and is labelled as such.

