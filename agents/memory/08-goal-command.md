# Working note — `/goal` (M203 state, M204 driver, M206 on the wire; all landed)

**Status: LANDED.** Commit `7ab5c18` (M203) and `a7fd0fc` (M204); batteries `m203` and `m204`
(GATE PASS, 1,607 then 1,613 tests across 64 binaries). Milestones:
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
3. ~~**`/goal` in the RPC and HTTP surfaces.**~~ **DONE at M206** — `SessionState.goal`
   (`SessionGoal`: `id`, `revision`, `armed`, `status`, `rounds_started`, `max_rounds`) is built by
   the one `session_state()` both transports share, so the field appeared in the RPC reply and the
   HTTP body and the generated OpenAPI document at once; `gate-m206.log` PASS, milestone
   [`M206`](../../docs/milestones/M206-goal-on-the-wire.md). **The residual is narrower than "RPC
   support" and is worth keeping:** the field is *read-only* — there is no `set_goal` op and no
   `POST /goal`, so a client can render a goal and cannot start, pause or re-arm one. `id` and
   `revision` are on the wire for exactly that compare-and-set, so the shape is ready; only the
   verbs are unbuilt. Note also that nothing in-tree consumes the field (M206's `Limits` says so).
4. **A spend warning — half done, and the other half is a deliberate absence.** The *round* half
   landed with the driver: `note_goal_budget` (`app/commands.rs:991`) warns once at each rung of a
   10% / 4% / 2% / last-round ladder, in fractions of the cap rather than fixed counts, because
   `/goal edit rounds <n>` can set any cap. So "nothing warns at round 200" is stale. What remains
   is the *money* question, and here the note has to stay honest: DSH deliberately has no token or
   currency budget (see the checklist below), so a currency warning would mean inventing one. The
   tractable version is to **report the session's own recorded usage against the goal's rounds** —
   `Usage` already carries tokens and cache rates per turn, so a goal could show what its rounds
   have cost without a new budget concept. That is a feature, not a correction.

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
