# M146 — F05: a delivered command is not a passing command

## What

Two defects in the verification record, both in how the harness answers "did the model's
check pass?".

**The exit code was never read.** `crates/epsilon-core/src/lib.rs` decided by asking whether
the tool result was an error. `BashTool` reports a command exiting nonzero as a *successful*
tool result carrying `details.exit_code`, because the tool did its job and the program it ran
did not. So `cargo test` with six failing tests was recorded as a passing check, and
`stop_nudge()` had nothing to say to a model that shipped broken code. The type had been lying
about this the whole time: `CheckRun::ok` was documented as "Whether it exited zero" while its
only caller supplied `!result.is_error`.

**Order was measured in turn numbers.** `unverified()` used `c.turn >= *turn`. Two tool calls
in one assistant turn share a turn number, so a check that ran *before* an edit in the same
batch vouched for the edit it had not yet seen.

## Key decisions

**Read the exit code, don't redefine the tool contract.** `check_passed()` in `verify.rs`
demands `details.exit_code == Some(0)`. `is_error` is untouched: an ordinary nonzero exit
remains a successful *tool call* carrying a failed *command*, and only the verification record
distinguishes them. The ledger's acceptance criteria explicitly forbade redefining ordinary
nonzero exits as transport failures, and they should not be — the model needs to read the
failing output.

**A missing exit code is not a pass.** `None` means a signalled process or a tool that reports
no status. Treating it as success would turn a killed test run into a clean bill of health. An
unknown outcome stays unknown: a redundant nudge costs one message, a false all-clear costs a
user an afternoon.

**Order by a sequence, not by turn.** `VerificationLog` counts every observed call into `seq`,
recorded on both edits and checks, and `unverified()` asks `c.seq > *seq` — "did a check run
*after* this change". The counter advances *before* the early returns, so a refused write still
consumes its position and ordering never depends on whether something was recorded.

## Gotchas

**A grep I truncated produced a false claim, and I wrote it into a plan.** I searched for
existing verification tests, piped through `head -6`, saw six matches from
`guard_scenarios.rs`, and concluded no integration test existed. `head -6` truncated the list —
`verification_scenarios.rs` was right there with four tests, and I was about to create a
duplicate file. The write was refused because the path already existed, which is the only
reason I found out in time. **This is the fourth time this session that a bounded search was
read as a complete one**, after three greps over guessed identifiers. See
`agents/memory/next-hardening.md`.

**The existing tests were asserting the bug.** `an_edit_that_stops_without_a_check_is_nudged`
and `a_passing_check_after_the_edit_ends_the_run_without_a_nudge` used `tool("bash")`, a
fixture returning `Ok(ToolOutput::text("ok"))` with no `details` at all — a shell that reports
nothing, which the old rule read as a passing check. Both began failing under the fix. They
were not wrong to exist; their fixture was a fiction the buggy rule happened to believe. Both
now use `delivered_bash(Some(0))`, the shape a real passing `cargo test` produces.

**Each control isolates one fix.** Reverting both halves at once would have let the suite pass
on the strength of either. See below.

## Verification

- `cargo fmt --all -- --check` — exit 0.
- `cargo clippy -p epsilon-core --all-targets --features real -- -D warnings` — exit 0.
- `cargo test -p epsilon-core --test verification_scenarios --features real` — **9 passed, 0
  failed** (4 new, 5 pre-existing).
- `cargo test --workspace --features real` — **62 binaries, 1,242 passed, 0 failed, 0 ignored**,
  exit 0. Up from 1,232 at M145: 6 new unit tests in `verify.rs` and 4 new scenarios here.
- Evidence: [`audit/evidence/f05-negative-control.md`](../../audit/evidence/f05-negative-control.md),
  and the test log is reproducible from the commands above.

### Negative controls

Two, one per half, because F05 is two defects and a single control proves neither on its own.
Full captured output in
[`audit/evidence/f05-negative-control.md`](../../audit/evidence/f05-negative-control.md).

**Control 1 — `ok = delivered` (exit-code rule reverted).** 2 failed, 7 passed.
`a_delivered_check_that_exited_nonzero_is_not_verification` and
`a_check_with_no_reported_exit_code_is_not_verification` both report `left: 0, right: 1` —
*zero nudges* for a run whose tests failed. The ordering test passed here, because the `seq`
fix was still installed.

**Control 2 — `c.turn >= *turn` (ordering rule reverted).** 1 failed, 8 passed.
`a_check_in_the_same_batch_before_an_edit_does_not_verify_it` reports `left: 0, right: 1`, and
the exit-code test passes here. The controls isolate cleanly.

Both files were restored from their own backups and compared against `HEAD`; no `.ctrl-backup`
remains.

## What nothing exercises

- **The real `BashTool` is not in the loop test.** `epsilon-core` has no dependency on
  `epsilon-tools`, and adding one for a test would put a real edge in the architecture graph.
  The shape is instead pinned on the producing side
  (`a_non_zero_exit_is_a_command_outcome_never_a_confinement_one` in
  `crates/epsilon-tools/tests/builtin_tools.rs`, asserting `details["exit_code"] == 7` and
  `details["verdict"] == "exited:7"` on an `Ok` return), and core's behavior *on that shape* is
  pinned here. **Nothing asserts that the two halves still agree.** If `BashTool` stopped
  publishing `exit_code`, this suite would pass while the defect returned.
- **A check run in the same batch *after* the edit.** Covered in the unit tests
  (`a_check_after_an_edit_in_the_same_turn_does_cover_it`), not through the loop.
- **Batch reordering.** `seq` assumes calls are observed in execution order, which holds
  because the executor commits to model order. A scheduler that reorders parallel calls would
  make `seq` a fiction, and nothing tests that assumption.
- `verification_nudges` counted the nudge as issued, not delivered to a model that read it.

## Limits

- Ordering is only as good as the observation order, which is model order, not measured
  completion order. For parallel batches these can differ.
- `check_passed` keys on the literal tool name `"bash"`. A check run through another tool name
  would fall back to transport success. pi has a separate `powershell` tool; Epsilon routes
  through `bash`.
