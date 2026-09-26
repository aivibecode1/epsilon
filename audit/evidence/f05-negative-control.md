# F05 negative controls — a delivered command is not a passing command

**Date:** 2026-09-26 · **Milestone:** M146 · **Defect:** F05

F05 had two independent halves. Each was controlled separately, because a single control that
reverts both would pass even if only one of the two fixes did anything.

## What was wrong

`crates/epsilon-core/src/lib.rs:1646` decided "did this check pass?" by asking whether the tool
result was an error:

```rust
let ok = results.iter().find(|r| r.tool_call_id == call_id)
    .map(|r| !r.is_error).unwrap_or(false);
```

`BashTool` reports a command exiting nonzero as a **successful** tool result carrying
`details.exit_code` — the tool did its job, the program it ran did not
(`crates/epsilon-tools/tests/builtin_tools.rs` pins this: `details["exit_code"] == 7`,
`details["verdict"] == "exited:7"`, and the call returns `Ok`). So a `cargo test` with six
failing tests recorded as a **passing check**, and the harness had nothing to say to a model
that shipped broken code.

The second half was independent: `unverified()` compared **turn numbers** (`c.turn >= *turn`).
Two calls in one assistant turn share a turn number, so a check that ran *before* an edit in the
same batch vouched for the edit it had not yet seen.

## Control 1 — revert the exit-code rule only

`ok = delivered`, the pre-F05 line, in `lib.rs`. Everything else, including the ordering fix,
left in place.

```
test a_delivered_check_that_exited_nonzero_is_not_verification ... FAILED
test a_check_with_no_reported_exit_code_is_not_verification ... FAILED
test result: FAILED. 7 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out
```

```
panicked at crates/epsilon-core/tests/verification_scenarios.rs:351:5:
assertion `left == right` failed: a check that exited 101 is not a passing check, however cleanly it was delivered
  left: 0
 right: 1

panicked at crates/epsilon-core/tests/verification_scenarios.rs:435:5:
assertion `left == right` failed: an unknown outcome stays unknown rather than counting as a pass
  left: 0
 right: 1
```

`left: 0` is the defect stated numerically: **zero nudges** were issued for a run whose tests
failed. The harness stayed silent exactly as it should not have.

Note what did **not** fail: `a_check_in_the_same_batch_before_an_edit_does_not_verify_it`
passed under this control, because the ordering fix was still installed. That is why control 2
exists.

## Control 2 — revert the ordering rule only

`c.seq > *seq` back to `c.turn >= *turn` in `verify.rs`. The exit-code fix left in place.

```
test a_check_in_the_same_batch_before_an_edit_does_not_verify_it ... FAILED
test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

```
panicked at crates/epsilon-core/tests/verification_scenarios.rs:476:5:
  left: 0
 right: 1
```

Again `left: 0`: a passing `cargo test` that ran *before* the edit silenced the nudge. And
`a_delivered_check_that_exited_nonzero_is_not_verification` passed here, so the two controls
isolate cleanly — neither test passes on the strength of the other fix.

## After restoring

```
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Both source files were restored from their own backups and compared against `HEAD`; the working
tree holds the intended change and no `.ctrl-backup` files remain.

## Two pre-existing tests changed, and why

`an_edit_that_stops_without_a_check_is_nudged` and
`a_passing_check_after_the_edit_ends_the_run_without_a_nudge` both used `tool("bash")`, a
fixture returning `Ok(ToolOutput::text("ok"))` with **no `details` at all** — a bash tool that
reports no exit status whatsoever. Under the corrected rule that is an unknown outcome, not a
pass, so both began getting two nudges instead of one.

The failures were not the fix being wrong. The fixtures were a fiction that the old rule
happened to believe: a shell that reports nothing was treated as evidence that everything
passed. Both tests mean "a check that passed", so both now use `delivered_bash(Some(0))` — the
shape a real passing `cargo test` produces. The intent is unchanged; the fixture now matches it.
