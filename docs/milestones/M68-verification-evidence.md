# M68 — verification evidence and the stop-time nudge

**Scope:** port-plan Tier 2 item 5 (roadmap Phase 3 item 1, defect D-14) — the plan's own words
call it "the single highest-value behavioural port", because it is the difference between a rule
the model is asked to follow and a mechanism the harness enforces.

`crates/epsilon-core/assets/agent-contract.md` tells the model to run the project's checks after
editing. Nothing recorded whether it did. A run could edit four files, claim the tests pass, and
stop — and from the outside there was no way to tell that claim from a verified one.

## What

`crates/epsilon-core/src/verify.rs` keeps the books per run:

- **Edits.** Every `write` / `edit` that *succeeded* is recorded with the turn it happened in
  (failed and refused mutations changed nothing, so they are not evidence of anything).
- **Checks.** Every `bash` call whose command looks like running the project's checks is recorded
  with its exit status. The predicate is deliberately conservative about what counts: a known
  runner (`cargo`, `npm`, `go`, `python`, `dotnet`, …) plus a check-like sub-command (`test`,
  `check`, `clippy`, `lint`, `build`, `vet`, `audit`, … — flags count, so `node --test` works), a
  program that *is* the check (`pytest`, `tsc`, `ruff`, `eslint`, `mypy`, `make`, `just`,
  `./gradlew`, …), or one of those as a segment of a compound command (`cd foo && cargo test`).
  `grep -r test .` mentions a check and is not one; `cargo fmt` is style, not verification.
- **The verdict.** A file is *unverified* when no **successful** check has run at or after the
  turn it was last changed in. A failing check does not clear an edit — "I ran the tests" is not
  "the tests pass", and the record is where that distinction lives.

At the stop point — the model produced a final turn and the run is about to end — if there are
unverified edits, the harness injects one message:

> `[harness] you changed src/lib.rs, and no check has run. Run the project's checks (e.g.
> cargo test, npm test, pytest) and report what they said — or state plainly why no check is
> needed here.`

Twice at most (`verification_nudges`, default 2), delivered as an ordinary user message so it is
in the transcript, in the session file, and in the TUI. The run is never *refused*: the model
decides, and either way the record stays.

A failed check changes the wording rather than clearing the edit (`N check(s) ran, none
succeeding after the change`), and the harness emits a `Notice` event so a frontend can show the
state without parsing the transcript.

## Key decisions

- **Evidence, not enforcement.** The harness does not run the checks itself (a build the user did
  not ask for, at a moment they did not choose) and does not refuse to stop. It records, and it
  says once. A refusal would need a policy for every project that has no checks at all — the
  nudge needs none.
- **A passing check is the only thing that clears an edit.** Anything weaker makes the record
  useless exactly when it matters: a run that edited, tested, saw red, and stopped.
- **Turn ordering, not timestamps.** Verification is "did this happen after the change", and the
  loop already counts turns deterministically — no clock, no ordering assumptions across
  processes.
- **Run-scoped.** The log lives in the run, like the stuck-run guard: a new run is a new question.
  Cross-run memory would need a definition of "still verified" that the session format does not
  carry.
- **Best-effort detection, stated as such.** The precise version reads the project's declared
  commands (Hermes `agent/verify/recipes.py`, its own mining item 6). This one recognises common
  invocations and errs toward "that was a check" once a known runner is involved, because a false
  positive costs one missing nudge while a false negative nags a model that did verify.
- **Known limitation, in the open:** the log belongs to the agent that owns it. A `task` child's
  edits are recorded against the child, so a parent that delegates an edit and stops is not
  nudged about it. Recorded here rather than left to be discovered.

## Test-caught bugs

1. **Two mutations that hid each other.** The first mutation run disabled both `stop_nudge()` and
   `is_check_command()` at once; the "a passing check needs no nudge" test then passed in the
   mutated build, because the *other* mutation had already suppressed every nudge. Re-run one at a
   time, it fails as it should. A mutation check that changes two things at once proves nothing
   about either.
2. **Dead code the compiler found, not the tests.** `has_edits()` and `check_count()` were
   written for the API and used only by tests, which the workspace lints report as dead code in a
   non-test build. `check_count` was deleted (its information is in the nudge text and the
   `Notice`); `has_edits` is now part of the loop's condition, where it reads better anyway.
3. **`pytest -q` was not a check.** The first predicate required a runner *plus* a marker word,
   so a bare `pytest` (or `tsc --noEmit`) went unrecognised — the check that a Python project
   would actually run. Caught by the predicate's own test, fixed by separating programs that *are*
   the check from runners that need a sub-command, and by letting flags count as words.

## Verification

```
$ bash scripts/gate.sh m68
===== fmt --check =====      clean
===== clippy =====           0 warnings
===== test =====             600 tests, 0 failed, 1 ignored (the live smoke)
===== demo =====             completes
===== real-check =====       compiles
```

Twelve new tests — seven unit (`verify::tests`) and five through the real loop
(`tests/verification_scenarios.rs`):

| Test | What it pins |
|---|---|
| `the_runners_that_count_as_checks_and_the_ones_that_do_not` | 15 real check invocations (including `pytest -q`, `node --test`, `cd x && cargo test`, `RUST_LOG=debug cargo test`) and 7 that must not count (`grep -r test .`, `cat build.log`, `cargo fmt`, `git commit -m 'run test'`) |
| `an_edit_with_no_check_is_unverified` | the basic record, and that the nudge names the paths and the tool to run |
| `a_successful_check_after_the_edit_clears_it` | the verified case |
| `a_failing_check_does_not_count_as_verification` | the distinction the record exists for |
| `an_edit_after_a_passing_check_needs_its_own_check` | ordering by turn |
| `a_failed_or_refused_mutation_is_not_an_edit` | a refused write is not evidence |
| `the_nudge_lists_at_most_three_paths_and_counts_the_rest` | the message stays bounded |
| `an_edit_that_stops_without_a_check_is_nudged` (loop) | the nudge through the real loop, and that it names the file |
| `a_passing_check_after_the_edit_ends_the_run_without_a_nudge` (loop) | the other direction |
| `a_check_that_did_not_pass_is_not_verification` (loop) | the failing-check wording |
| `read_only_work_is_never_nudged` (loop) | a question is not an edit |
| `the_nudge_gets_two_chances_and_then_stops` (loop) | the cap, and that the run still ends `Completed` |

Checked by mutation, one at a time:

```
# stop_nudge() always returns None
verify::tests::an_edit_with_no_check_is_unverified ... FAILED
verify::tests::a_successful_check_after_the_edit_clears_it ... FAILED
verify::tests::a_failing_check_does_not_count_as_verification ... FAILED
verify::tests::the_nudge_lists_at_most_three_paths_and_counts_the_rest ... FAILED
verification_scenarios: an_edit_that_stops_without_a_check_is_nudged ... FAILED
verification_scenarios: a_check_that_did_not_pass_is_not_verification ... FAILED
verification_scenarios: the_nudge_gets_two_chances_and_then_stops ... FAILED

# is_check_command() always returns false
verify::tests::the_runners_that_count_as_checks_and_the_ones_that_do_not ... FAILED
verification_scenarios: a_passing_check_after_the_edit_ends_the_run_without_a_nudge ... FAILED
```

`read_only_work_is_never_nudged` passed in both mutated builds, which is correct — there is
nothing to nudge about. Both mutations were reverted before the gate run above.

## Follow-ups

Cache-miss attribution (Phase 2 item 4) and provider-side message sanitization (item 18) are what
remain of the port plan's ranked list. Standing items: committing M33–M68 (D-12) — now 180+
files, and the largest open risk in the repository — live-provider evidence for every wire (D-13,
blocked on an expired stored key), and release artifacts (D-21). See
[`audit/04-roadmap.md`](../../audit/04-roadmap.md).
