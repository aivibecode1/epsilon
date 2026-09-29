# Working note — `/goal` (M203 state, M204 driver; both landed)

**Status: LANDED.** Commits `7ab5c18` and `a7fd0fc`; batteries `m203` and `m204` (GATE PASS,
1,607 then 1,613 tests across 64 binaries). Milestones:
[`M203`](../../docs/milestones/M203-goal.md) (state, command, tools, prompt) and
[`M204`](../../docs/milestones/M204-round-driver.md) (the round driver).

What exists now: the durable goal (`EntryKind::Goal` + `SessionStore::goal()`), the service
(`Agent::goal` / `goal_apply` / `goal_begin_round`), the model's three tools, `/goal` with seven
verbs, the footer badge, the objective in the CONTEXT prompt tier, and a driver that continues an
armed goal at each turn end.

## What is left, in the order it should go

1. **An end-to-end round test.** Two real rounds through the event loop, with a scripted provider
   and a pseudo-console. Everything either side of the join is tested; the three lines that join
   them are read, not run. `crates/epsilon-cli/tests/terminal.rs` is the harness that could do it.
2. **A goal-change event.** The badge is refreshed at two points (a `/goal` command, a turn end)
   because reading the goal folds the session log. A `GoalChanged` `AgentEvent` would make it
   event-driven and let the transcript announce a model-initiated change. Still not worth a wire
   variant on its own — but the driver makes it likelier, because a tool now changes the goal
   *during* a turn whose end triggers the next round.
3. **`/goal` in the RPC and HTTP surfaces.** `session_state()` is the one builder both transports
   share (D29's lesson); a goal is the next field a client wants.
4. **A spend warning.** The cap is 256 rounds and nothing warns at round 200. At a real provider's
   prices that is a money question, not a tidiness one.

## The traps this slice met, recorded so they are not met again

- **`GoalError::NoGoal` was carrying two facts** (a session fact and a frontend fact) until a test
  expected the right sentence and got the wrong one. `NoStore` is the split.
- **Two agents may not open one session file** — the lease refuses the second open. Write through
  `agent.session_store()` in tests instead of reopening.
- **An async spawn needs a runtime**: every goal test that creates one is now `#[tokio::test]`,
  because creating a goal starts a round, and a round spawns a run.
- **A key handler posts through the notice channel**, so a test asserts on what the loop would
  drain, not on `ui.lines()`.
- **The disarm had to stop depending on the cancel landing.** Hanging it on `abort_current_run()`
  returning true made the rule a function of whether a token was live — and untestable. Hang a
  policy on the *request*, not on the mechanism's success.

## The checklist for anything else ported from DSH's goal domain

Read `references/deepseek-harness-dsh-v0.2.0-rc.1/packages/goal/goal/README.md` first; the
contract is small and every clause of it is load-bearing. Then decide, explicitly, which of these
you are *not* doing — they are all deliberate absences there, not gaps: a token or currency
budget, an independent evaluator of completion, a goal database, parallel goals, and any
scheduling policy beyond the round cap.
