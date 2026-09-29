# Working note — `/goal` (M203, landed)

**Status: LANDED.** Commit `7ab5c71`; battery `m203` (GATE PASS, 1,607 tests / 64 binaries).
Milestone: [`docs/milestones/M203-goal.md`](../../docs/milestones/M203-goal.md).

What exists now: the durable goal (`EntryKind::Goal` + `SessionStore::goal()`), the service
(`Agent::goal` / `goal_apply` / `goal_begin_round`), the model's three tools, `/goal` with seven
verbs, the footer badge, and the objective in the CONTEXT prompt tier.

## What is left, in the order it should go

1. **The round driver (M204).** `Agent::goal_begin_round()` is written, tested and **called by
   nothing**. It returns the prompt a continuation would send; the TUI's turn-end path should
   send it while `is_continuable()` holds. That is the whole of the feature's *doing* half and it
   is one call site plus a loop guard — the reason M203 stopped short of it is that an
   unattended loop needs the tools (which now exist) and a stop condition that a *test* can pin
   without a provider.
   Watch for: the driver must not fire while a permission ask is pending, must not fire twice for
   one turn end, and must respect the cap through `goal_begin_round` rather than by counting
   locally.
2. **A goal-change event.** The badge is refreshed at two points (a `/goal` command, a turn
   end) because reading the goal folds the session log. A `GoalChanged` `AgentEvent` would make
   it event-driven and would also let the transcript announce a model-initiated change. Not
   worth a wire variant until something else needs it — but it is the honest fix.
3. **`/goal` in the RPC and HTTP surfaces.** `SessionState` carries model/thinking/session
   fields; a goal is the next thing a client would want, and `session_state()` is the one
   builder both transports already share (D29's lesson).

## The four things this slice got wrong first, recorded so they are not repeated

- **`GoalError::NoGoal` was carrying two facts.** "No goal is set" (a session fact the user can
  act on) and "this session cannot hold a goal" (a frontend fact) produced the same sentence
  until a test asserted the right one and got the wrong one. `NoStore` is the split.
- **Two agents may not open one session file.** The lease refuses the second open — the product
  is right and two of my tests were wrong. Write through `agent.session_store()` instead.
- **A `SessionLine`-shaped error names nothing** (M202's lesson, and the reason `diagnose_line`
  re-decodes through the tagged parser).
- **`edit` with neither an objective nor a cap is a mistake, not a no-op**, and the refusal says
  so. Same for a create with an empty objective.

## The checklist for anything else ported from DSH's goal domain

Read `references/deepseek-harness-dsh-v0.2.0-rc.1/packages/goal/goal/README.md` first; the
contract is small and every clause of it is load-bearing. Then decide, explicitly, which of
these you are *not* doing — they are all deliberate absences there, not gaps: a token or currency
budget, an independent evaluator of completion, a goal database, parallel goals, and any
scheduling policy at all.
