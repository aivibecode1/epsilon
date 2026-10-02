# M257 — the gated-goal test that had never existed

`agents/memory/goal-gates.md` carried this from the start and it was true right up to this
milestone:

> The end-to-end test that drives `UpdateGoalTool` end to end with a gated goal — that test
> does not exist and is the most important missing one.

Every gate test until now drove a **`FakeHost`** or called the runner directly.
`FakeHost` answers `current_goal` from a snapshot it was handed, so two questions were never
asked of the thing that actually holds the goal:

- **does the gate the user declared reach the tool?**
- **does a refused completion leave durable state unrecorded?**

The second is the one a snapshot structurally cannot check. A refusal that still wrote
`complete` would pass every other test in the crate and leave a goal marked done for ever.

## It lives in `epsilon-runtime`, and that is not arbitrary

The test needs `epsilon-llm`, `epsilon-session`, `epsilon-core` and `epsilon-tools` at once.
`epsilon-tools` — where the tool and its `FakeHost` tests are — does **not** depend on the
first two. Putting the test beside the tool would have meant adding two dependencies to a
crate to test it, or leaving it in the place where it cannot exist.

`epsilon-runtime` is the crate whose entire job is being where a workspace, a configuration
and a runtime become a running agent, and it depends on all four. **A composition test
belongs in the composition crate.**

## What it takes, and one thing it found

The host is `agent.clone_goal_host()` — the adapter `epsilon-core`'s own execution path puts
in that slot, not a second construction. Getting there needed it to be `pub`, which is one
word and is justified by composition rather than by the test: a frontend invoking a goal tool
outside a turn needs the same adapter, and a second implementation would be a second answer
to "where does a goal live".

Then the first run failed, and the failure was worth having:

> the goal moved: this was issued against revision 2 of `g1`, and the session holds
> revision 2 — re-read it and try again

Same revision, so the message reads as "nothing changed", which is the least useful thing it
could say. The cause was mine: **`"g1"` is a `FakeHost` artefact.** A real `Agent` generates
its own goal id, and every gate test had hardcoded `"g1"` without noticing that it was only
ever a stand-in's id.

Two things came out of it:

- `GoalError::Stale` now prints the **actual goal id** as well, and says *"a different goal;
  `get_goal` names the current one"* when the ids disagree. A caller that named the wrong
  goal was previously told only a revision — the one half of the answer it already had.
- The test reads the real id and revision from the agent rather than assuming any.

## Verification

Battery `m257` — **GATE PASS on attempt 1: 1,837 tests across 76 binaries, 0 failed**
(`audit/evidence/gate-m257.log`). Up from 1,835, one new test binary.

Two tests, and the pair matters: a real `Agent` whose own gate **fails** refuses the
completion, names the command the *user* declared, leaves the status un-recorded and leaves
the gate in place; and a real `Agent` whose gate **passes** records the completion and keeps
the gate. Without the second, the first is satisfied by a tool that refuses everything.

**Negative control:** with the tool's gate check disarmed,
`a_real_agent_refuses_a_completion_its_own_gate_fails` fails and
`a_real_agent_records_the_completion_when_its_own_gate_passes` still passes — the tool then
refuses nothing, which is exactly the defect.

**Reinstall:** none. `epsilon-cli` and `epsilon-tui` are untouched; the change is one
`pub` on an adapter, a better error message, and a test.

## What nothing exercises

- **The gate runs unconfined.** `GoalGateRunner::new(None)`, because this test is about the
  tool-to-goal wiring, not the backend. The confined path has its own coverage (M255).
- **The goal is never armed and no round is ever taken**, so this proves the *completion*
  gate and not the *continuation* gate. M254's `goal_round_gates.rs` covers the other half,
  and neither exercises the two together: a real loop where a round's gate check and the
  model's own `complete` call race for the same revision.
- **No concurrent `complete`**, so the compare-and-set is exercised only in its uncontended
  form.
- **The desktop window still has no round driver**, so gates remain terminal-only.