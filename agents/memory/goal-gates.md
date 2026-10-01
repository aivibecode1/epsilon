# Working note — goal gates, and what is still missing

**Status: storage + enforcement landed at M240. Declaration does not.**

## The one sentence

**A goal may declare commands that must exit zero before it can be completed.** The storage is
durable (`GoalSnapshot::gates`, `#[serde(default)]` so old sessions open) and the enforcement is
in `UpdateGoalTool`'s `Complete` arm.

## The trap I walked into

The first version **refused to run a gate when the sandbox was `None`**, on the reasoning that
an unconfined session cannot produce *evidence*.

That is backwards. `bash` refuses when a backend is **missing** and runs when the user
explicitly asked for `Confinement::Off`. `Off` is a choice, not a failure to confine — and
refusing there made the whole feature inert for exactly the people who opted in.

**The rule:** fail closed when the user asked to be confined and the machine cannot. Honour the
choice when the user asked not to be confined. Those are two different situations and they must
not share a branch.

The runner shares its `Sandbox` with the shell tool, resolved from the same `Confinement` in the
same `register_builtins_with` call. That is deliberate: a gate outside confinement is a confused
deputy, and one resolution means a transcript cannot name two different backends for a tool call
and the gate that judged it.

## What is missing, in the order it should be built

1. **No syntax to declare a gate.** `GoalOp::Edit` does not carry one, so today a gate is only
   reachable by hand-editing a session file, which makes the feature unreachable. Add the
   argument, then the test that drives `UpdateGoalTool` end to end with a gated goal — that test
   does not exist and is the most important missing one.
2. **The round driver does not consult gates.** They gate *completion*, not *continuation*: a
   failing gate should not stop the loop, because the next turn is what might fix it. Hermes
   uses a failing gate's output as the *continuation prompt*, which is a real improvement but
   changes what a round costs.
3. **`workspace_fingerprint` is computed and tested for stability, never acted on.** The skip on
   an unchanged workspace is the obvious next use; `GateReport::carried_over` is the field for it.
4. **No gate has ever run confined in a test.** Every gate test runs with `Confinement::Off`,
   because that is the only backend the harness sets up. The production branch is unexercised.
5. **The `/goal edit gate` parser is not driven by a test** for `gate clear` versus `gate <cmd>.

**A trap worth remembering:** a test beside the tool that exercised `goal_apply` rules **passed
against a build where the rule did not exist**, because it drove a `FakeHost`. A test that
exercises a stand-in is a test of the stand-in. Rules live in `Agent::goal_apply`; their tests live in
`epsilon-core/tests/goal_state.rs`.