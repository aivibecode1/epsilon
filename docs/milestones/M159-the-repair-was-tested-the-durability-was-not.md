# M159 — the repair was tested; the *durability* was not

## What this is

Closing gap **G1** from [`audit/12-fresh-reference-audit.md`](../../audit/history/12-fresh-reference-audit.md).
Small slice, and the interesting part is that the audit that produced it was **half wrong** and
the correction is the substance of this milestone.

## What the audit claimed, and why it was wrong

The audit said `Agent::close_dangling_tool_calls` (`crates/epsilon-core/src/lib.rs:1157`) had
**no test**, citing a search that returned only three unrelated hits. It had three tests, written
at M70, in `crates/epsilon-core/tests/sequence_repair.rs`.

The subagent that found this did the right thing with the finding: it did not write a redundant
test, and it did not close the gap as written. It read the three existing tests and reported what
they actually prove.

**What they prove:** the repair works on the runtime-history view. Their `agent()` helper builds
an `Agent` with **no `SessionStore` attached**, and seeds history through `restore_history()`.

**What they do not prove:** anything about the store. And the store is the entire claim. The
repair synthesises "this call was never executed" — which is only a *fact* if the file already
held the call before the tool ran. An assertion about process memory cannot establish a property
about a file on disk.

So the accurate gap statement is narrower and sharper than the one I wrote: **the runtime view
was tested; the durable view was not.**

## The test

`the_repair_is_durable_and_leaves_the_answered_call_untouched`, in the same file.

It writes the interrupted turn **through a real `SessionStore`**, then resumes the way `--resume`
does — from that store's own `model_view()` rather than from a literal — and requires the run's
repair to land in **both** views as the **same** `ToolResultMessage` (`PartialEq`), not two
divergent ones.

Three details that matter more than the happy path:

- **The precondition is asserted first.** `open_call_ids(&restored) == ["call_dangling"]` proves
  the call survived a write and a read-back with no result. Without it, a later "never executed"
  could be an artefact of the test seeding nothing.
- **Both calls ride in the same assistant message.** A repair that closed *everything* fails
  here instead of passing by over-shooting. An over-eager repair is as much a bug as an absent
  one, and the existing `a_healthy_transcript_is_left_alone` cannot catch it — with nothing
  dangling, both behaviours look identical.
- **The answered call keeps its original message id** and stays `is_error == false`, so the test
  fails if the repair replaces a real result with a synthetic one.

## Negative control

`close_dangling_tool_calls` short-circuited to `return Ok(0)` at `lib.rs:1158`:

```
running 4 tests
test a_run_that_starts_on_a_dangling_call_closes_it_before_the_next_request ... FAILED
test a_healthy_transcript_is_left_alone ... ok
test a_second_run_does_not_close_the_same_calls_twice ... FAILED
test the_repair_is_durable_and_leaves_the_answered_call_untouched ... FAILED

---- the_repair_is_durable_and_leaves_the_answered_call_untouched stdout ----
panicked at crates\epsilon-core\tests\sequence_repair.rs:359:9:
assertion `left == right` failed: transcript: the unanswered call got exactly one result
  left: 0
 right: 1

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out
```

The shape is the thing worth noting: `a_healthy_transcript_is_left_alone` still **passes** under
the control, and correctly so — it asserts nothing is invented, which a disabled repair
satisfies trivially. So the control is specific to the property rather than a generic breakage,
and the existing M70 tests now have a demonstrated RED as well.

**Restore verified independently of the subagent's word:** `git diff HEAD -- crates/epsilon-core/src/lib.rs`
is 0 bytes, and no `NEGATIVE CONTROL` marker remains in the file.

## What this says about the audit method

Two of this session's corrections came from the same place, and they are worth stating as a rule
rather than as anecdotes:

1. The archive step globbed a stale file from a **previous session** and nearly committed it as
   today's evidence.
2. The G1 claim asserted an absence ("no test names it") on the strength of a single search.

Both are ways of being confident without having checked. The first is fixed by checking dates; the
second by searching more than once before claiming a negative — a *negative* claim is the one
kind of claim a single search cannot support, because the search itself is the only evidence and
it has no second opinion.

`audit/12` now carries a correction block on G1 rather than a quiet rewrite, so the record shows
that the audit was wrong before it was right.

## What nothing exercises

The test drives `agent.run()` against the faux provider, so it exercises the repair on the path
the loop takes. It does **not** exercise a real process kill: nothing here proves the bytes are
on disk before a crash, only that a store that already holds them produces a coherent repair.
Proving the ordering itself needs a fault-injection test that kills the process mid-batch, which
this slice does not attempt.

## Verification

- `cargo test -p epsilon-core` — exit 0, `sequence_repair` **4 passed, 0 failed**.
- Control: 1 passed, 3 failed.
- Full battery: see `docs/history.md`.
- No reinstall of the global `epsilon.exe` was needed: this is a test-only change plus a
  documentation correction, and nothing in the shipped binary moved.
