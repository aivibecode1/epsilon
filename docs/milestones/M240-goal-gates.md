# M240 — a goal's gates: "done" stops being something the model says

`/goal` makes the harness keep working between turns. Until now it stopped when the model
said it was finished. **That is the one judgement in the loop with nothing behind it.**

## What a gate is

A command that must exit zero before a goal may be completed. `cargo test` passes or it does
not; "I believe the fix is correct" is neither reproducible nor checkable. Hermes calls this
`GoalGate` (`hermes_cli/goals.py:337-368`) and it is the single most valuable idea in that
reference — but note what it is *not*: a new stopping condition. It is **evidence for the
stopping condition that already exists**, which is why it fits a harness that has no LLM
judge. `crates/epsilon-contracts/src/goal.rs` already said so in its header — *"an evaluator
that certifies completion (the caller that records it is authoritative)"*. A gate is that
evaluator, written in a shell rather than in a model.

It is durable state on `GoalSnapshot`, beside `objective`, because "the goal is not done
until the tests pass" is a fact about the goal. A completion rule that lives somewhere else
can be bypassed by anything that forgets to look.

## The decision inside the decision: which sandbox

**The gate runs through the same `Sandbox` the `bash` tool uses**, resolved from the same
`Confinement` in the same `register_builtins_with` call — one line, and it is a security
property rather than an implementation convenience:

- A gate declared *outside* confinement would be a **confused deputy**. Everything the agent
  runs is confined; a command the agent cannot run but the goal can turns the permission gate
  into decoration.
- One resolution means a transcript cannot say `job-object` for a tool call and `off` for the
  gate that decided whether it succeeded.

**And the first version got this backwards.** It refused to run a gate when the sandbox was
`None` — reasoning that an unconfined session cannot produce *evidence*. That inverts the
shell tool's own rule: `bash` refuses when a backend is **missing** and runs when the user
explicitly asked for none. `Confinement::Off` is a choice, and refusing there made the feature
inert for exactly the people who opted in. The test that pinned the wrong behaviour is
replaced by one that pins the right one, and the reason is written where the branch is.

## What a refusal says

The refusal names the command, the exit code, and the **tail** of its output — a test suite's
failure is at the end, and the head is build noise. Output is capped at 3,000 characters: a
gate must not blow the context of the very round it was meant to inform. A gate with no
command is reported as one that can never pass, rather than as one that has not been run yet.

## Two small correctness things this had to get right

**An absent timeout is the default, not zero.** A gate with `timeout_seconds: 0` would fail
instantly and always, which looks like a rule and behaves like a bug.

**A session written before gates existed still opens.** `gates` carries
`#[serde(default)]` — the M202 rule made concrete. A field added to a durable type without a
default turns every old session file into one the harness cannot read, and
`a_session_written_before_gates_existed_still_decodes` is that test.

## What is deliberately not here

- **The round driver does not consult gates.** They gate *completion*, not continuation: a
  failing gate does not stop the loop, because the next turn is exactly what might fix it.
  Hermes uses gates a second way — as the continuation prompt when a gate fails — and that is
  a real improvement, but it changes what a round costs and belongs in its own slice.
- **The fingerprint skip is computed, not acted on.** `workspace_fingerprint` exists and is
  tested; nothing consults it yet. Recording an unused helper is still better than pretending
  the skip is implemented, and the field on `GateReport` is there for the slice that will.
- **No syntax to declare a gate.** `GoalOp::Edit` does not carry one yet, so today a gate is
  only reachable by editing a session file — which is useless. The next slice adds the
  argument, and the type is already durable and round-tripping.

## Verification

**Battery `m240` — GATE PASS on attempt 1, 1,774 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m240.log`). Up from 1,759: eight gate tests plus the contract additions.

The tests that matter are the ones that **run a gate**: `a_gate_that_passes_lets_the_completion_through`
and `a_failing_gate_names_the_command_and_its_output` spawn `cargo --version` and a deliberately
bad subcommand and read the report. A test that only checks a struct field would prove nothing
a user would notice.

## What nothing exercises

- **The completion refusal through `update_goal`.** The refusal branch is written and typed, but
  no test drives `UpdateGoalTool` with a gated goal, because there is no way to *declare* a
  gate yet. That is the single most important missing test, and it is missing for a reason
  that is also the next slice.
- **A gate under real confinement.** Every test here runs with `Confinement::Off`, because
  that is the only backend this test harness sets up. The confined branch — the one that
  matters in production — is unexercised.
- **A gate that times out.** The bound is enforced and the test asserts only that the wait is
  bounded; no test makes a gate actually exceed its limit.
- **`workspace_fingerprint` distinguishing two workspaces.** It asserts stability, not
  difference, because nothing acts on the difference yet.