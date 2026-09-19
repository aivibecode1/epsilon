# M67 — guards for runs that look busy and go nowhere

**Scope:** port-plan Tier 2 item 7 (roadmap Phase 3 item 3). Two failure modes the existing
brakes cannot see, both of which spend the user's money while looking like work:

- **No progress.** The doom-loop brake counts identical *calls*. A stuck agent does not repeat a
  call — it tries `rg`, then `grep`, then `find`, gets the same "no such file" three different
  ways, and reports success having learned nothing.
- **The empty turn.** A run can end on a turn that said nothing at all, or on one that repeated a
  single line until it hit the output token limit. The loop reads both as `Completed`.

## What

| Guard | Detects | Rungs |
|---|---|---|
| `guards::ProgressGuard` | consecutive identical tool **results**, whatever the calls were | warn → block |
| `guards::empty_turn_reason` | a final turn with no text and no calls, or one line dominating a long reply | nudge → fail |

**The result signature** is error-ness plus a normalized head of the result text (200 chars,
case-folded, whitespace-collapsed, FNV-1a). Normalization is the point: two runs that told the
model exactly the same thing are not progress, and a byte-diff would call them progress because
of a trailing newline. A difference inside the head resets the streak — that is real new
information. A difference only in the tail does not: a wall of logs that differs in its last
lines is the same result to the reader.

**The ladder:** at `no_progress_threshold` (default 3) repeats, the harness appends guidance to
the very result that repeated, so the model reads the observation in the same breath as the thing
it is about:

> `[harness] this result is identical to the previous one (3 times in a row) — whatever was just
> tried did not move anything. Consider a different approach.`

At twice the threshold the wording escalates and names the way out (`report the blocker … or
change strategy`). Separately, before each batch, a stalled run raises a `no_progress` permission
request — the same surface the doom-loop brake uses, so the block rung is visible to a frontend
and answerable by the user. `max_turns` remains the halt of last resort.

**The empty turn:** one nudge message (`[harness] your last reply was empty (attempt 1) …`),
delivered through the ordinary message path so it is in the transcript and in the session file;
if the next turn is empty again the run ends with `TurnEndReason::Error` naming the cause instead
of a false `Completed`. Degenerate repetition (one non-empty line taking ≥ 50 % of a reply of ≥ 20
lines) is the same failure wearing a longer coat, and takes the same path.

Both guards are cheap predicates and both are disableable: `no_progress_threshold: 0` turns the
stuck-run guard off entirely, `empty_turn_retries: 0` ends a run on its first empty turn.

## Key decisions

- **Judge the result, not the call.** Call identity is the wrong signal: a stuck model varies its
  arguments. What the model *learned* is what the result says, so that is what is hashed.
- **Tell the model, never fake a failure.** The guidance rides along the result; the harness never
  rewrites a successful tool result into an error to get attention. The tool did what it did, and
  the guard's job is to say what that means.
- **Ask once per batch.** A stalled run is a property of the batch, not of each call inside it: the
  permission request fires once, not once per call. (`asked_no_progress`, added after the first
  version asked N times for an N-call batch.)
- **Nudge once, then fail honestly.** An empty turn may be a fluke; two in a row is a broken run,
  and saying so beats reporting `Completed`. The reason string names the shape of the failure, so a
  headless caller can tell "empty" from "repetitive" from "provider error".
- **A zero threshold disables, it does not misfire.** `stalled_at(0)` is false by construction,
  because a misconfigured guard that blocks everything is worse than no guard.
- **Run-scoped state.** The progress guard resets at the start of every run, beside the doom-loop
  streaks: a new run starts clean.

## Test-caught bugs

1. **The stuck-run test could not reach its own block rung.** The permission check runs *before* a
   call, so with three attempts the guard had only ever seen two results by the time the last call
   was prepared — threshold 3 was unreachable. The fixture needed a fourth attempt. (The warn rung
   fires at result time, which is why the guidance assertion passed while the request one failed.)
2. **A signature test that tested nothing.** The "noise past the head" fixture put the differing
   fillers inside the 200-char window, so the signatures differed and the assertion was wrong
   rather than failing — the same class of mistake as the M66 fixture. Fixed by making the shared
   prefix longer than the window.
3. **An `unused_mut` that only the compiler saw.** The `push` closure in M66's
   `user_instruction_block` was flagged by rustc during this round's first build; the workspace
   lints deny warnings, so it would have failed the gate (M63's lesson, re-taught by the compiler).

## Verification

```
$ bash scripts/gate.sh m67b
===== fmt --check =====      clean
===== clippy =====           0 warnings
===== test =====             587 tests, 0 failed, 1 ignored (the live smoke)
===== demo =====             completes
===== real-check =====       compiles
```

Thirteen new tests — nine unit (`guards::tests`) and four through the real loop
(`tests/guard_scenarios.rs`):

| Test | What it pins |
|---|---|
| `different_commands_with_the_same_failure_are_detected_as_stuck` | the failure mode the doom-loop brake misses; whitespace and case are not progress |
| `a_different_result_resets_the_streak` | new information clears the streak |
| `an_error_and_a_success_with_the_same_text_are_not_the_same_result` | a command that started working is progress |
| `the_signature_ignores_noise_past_the_head` | walls of logs that differ only in their tail |
| `guidance_says_what_was_observed_and_never_that_the_tool_failed` | the wording contract, in both severities |
| `an_empty_turn_is_empty_but_a_silent_tool_call_is_not` | a turn that called a tool did something |
| `a_reply_that_repeats_one_line_is_degenerate` | the repetition predicate, with its length floor |
| `the_nudge_tells_the_model_what_was_wrong` | the nudge names the problem |
| `a_zero_threshold_never_reports_a_stall` | disabling is not misfiring |
| `different_commands_with_the_same_failure_trip_the_stuck_run_guard` (loop) | three different calls, one result: guidance on the third, a `no_progress` request before the fourth, and the original result text intact |
| `a_successful_run_never_sees_the_guard` (loop) | the other direction: progress is never interrupted |
| `a_run_that_says_nothing_is_nudged_then_failed` (loop) | the nudge reaches the transcript, and the run ends with a reason that names the cause |
| `a_degenerate_reply_is_caught_too` (loop) | repetition ends the run as itself, not as success |

Checked by mutation:

```
# ProgressGuard::observe() always returns Progress::New
guards::tests::different_commands_with_the_same_failure_are_detected_as_stuck ... FAILED
guards::tests::a_different_result_resets_the_streak ... FAILED
guards::tests::the_signature_ignores_noise_past_the_head ... FAILED
guard_scenarios: different_commands_with_the_same_failure_trip_the_stuck_run_guard ... FAILED

# empty_turn_reason() always returns None
guards::tests::an_empty_turn_is_empty_but_a_silent_tool_call_is_not ... FAILED
guards::tests::a_reply_that_repeats_one_line_is_degenerate ... FAILED
guard_scenarios: a_run_that_says_nothing_is_nudged_then_failed ... FAILED
guard_scenarios: a_degenerate_reply_is_caught_too ... FAILED
```

`a_successful_run_never_sees_the_guard` passed in both mutated builds, which is correct — it
asserts the absence of guard activity. Both mutations were reverted before the gate run above.

## Follow-ups

Verification enforcement (port plan item 5, the last behavioural item in Phase 3), cache-miss
attribution (Phase 2 item 4), provider-side message sanitization (item 18), and the standing
items: committing M33–M67 (D-12), live-provider evidence for every wire (D-13), release artifacts
(D-21). See [`audit/04-roadmap.md`](../../audit/04-roadmap.md).
