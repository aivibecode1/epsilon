# M204 — the round driver: an armed goal continues itself

**Status:** landed. GATE PASS `m204` — see [Verification](#verification). Reinstall: **required** —
this is the difference between `/goal` being a note and being work.

## What

M203 shipped the goal's *state*, its command and its tools, and nothing that acts on them. This
is the acting: at a turn end, if the goal is continuable, the harness sends the next round itself.

```
/goal keep the doc gates green     →  ● goal 1/256   round 1 starts immediately
   …turn ends, nothing left running…  →  ● goal 2/256   continuing: a turn the user did not type
   …the model calls update_goal complete… →  ✓ goal complete 3/256   and the loop stops
```

Three call sites, each with one job:

| Site | Job |
|---|---|
| `App::continue_goal` | The decision — five conditions, re-read from the log rather than remembered |
| `app/event.rs` | The trigger — once per drained batch that contained a turn end |
| `app/keys.rs` | The stop — an abort disarms the goal, whether or not the cancel landed |

## The decision, and why each condition is there

`continue_goal` refuses unless **all** of these hold, and every one of them is a way the harness
could otherwise do something the person in front of it did not ask for:

- **The goal is continuable** — active, armed *in this process*, under its cap. Read from the log
  at the moment of the decision, so a `complete` or `pause` from a tool during the turn stops the
  loop on the same frame rather than one round later.
- **Nothing is in flight and no permission ask is parked.** Belt-and-braces at a turn end, and
  both would be a disaster if they were ever true.
- **The composer is empty.** A user halfway through typing a follow-up has not asked for another
  round, and starting one would race their Enter key. The goal stays armed, so their own turn
  continues it — the round is *held*, not lost.
- **The run produced at least one message** (in the caller, which can see the event). A round that
  failed at the provider boundary produces nothing, and repeating it would spend the whole round
  budget on a loop that cannot make progress. A successful turn always produces at least an
  assistant message, so this is exact rather than a heuristic.

Two more decisions, both about not fighting the user:

- **An abort disarms.** Esc during a run is a person saying stop; a driver that restarts the
  moment the turn ends is the harness arguing with them. The durable goal is untouched and
  `/goal resume` (or the model's `update_goal`) starts it again — DSH's split between the state
  and the permission to continue it. **The disarm hangs on the request, not on the cancel
  landing**: hanging it on `cancelled` would make the rule a function of whether a token happened
  to be live, and the intention is the request either way. That is also what made the branch
  testable, through the real key handler.
- **Creating or resuming starts the first round.** `/goal fix the gates` that parked an objective
  and then sat there until the user typed something else would be a note, not a goal. `pause` is
  how a user parks one without starting it.

## What the round is, on the wire and in the log

One user-role message, built by the same constructor every other prompt uses, echoed into the
transcript and appended to the session file exactly like a typed turn — so a resumed session
replays it, and "what did the harness say to the model" has one answer. It is **not** put through
`submit_text`: that would put a harness-authored continuation in the Up-arrow recall ring and in
the global prompt-history file, and neither is where a person's words go.

The prompt names the objective, the round and the cap, and says what ends the loop:

```text
Continuing the session goal (round 2/256). Objective: keep the doc gates green

Work on it, then stop and report what changed and what is left. If the objective is achieved,
mark the goal complete with the `goal` tool; if you are blocked by something you cannot change,
mark it blocked and say what it is. The harness starts the next round only if the goal is still
active.
```

## Limits — what this does not prove

- **No test drives a whole round loop end to end.** The decision is tested at thirteen points
  (`goal_tests.rs`), the round prompt and the counting are tested in `epsilon-core`, and the loop
  that joins them is three lines read by eye. A test that ran two real rounds through the event
  loop would need a scripted provider *and* a terminal; the resource exists (`terminal.rs`) and it
  is not what this slice spent its budget on. Recorded as the gap it is.
- **A model that never completes its goal runs until the cap.** That is DSH's design — rounds are
  the only budget, and there is no evaluator — and the cap is 256 by default. At a real provider's
  prices that is a great deal of money, and the only guards are the cap, the badge, and a human
  noticing. `/goal edit rounds <n>` lowers it; nothing warns at round 200.
- **The driver is the TUI's.** `--print`, `--serve` and `--rpc` run one turn and stop, so a goal
  in a headless session is state without a driver. That is defensible (a batch run has nobody to
  notice a loop) and it is not written down anywhere but here.
- **A round is not attributed to the goal in the log.** The message is an ordinary user turn; the
  only record that it was goal-sourced is its text and the `rounds_started` counter. DSH attaches a
  `GoalMessageSource { goalId, revision, round }` to the message event; Epsilon would need a field
  on `UserMessage`, which is a wire change for a fact the counter already carries.

## Verification

GATE PASS, 2026-09-29 — battery **`m204`**, run against this commit's contents with the
documentation already final. Seven gates, exit 0 each: `check-links` 0 s (449 relative links,
0 broken), `check-lint-tables` 0 s, `fmt --check` 2 s, `clippy --workspace --all-targets` 9 s,
`test --workspace` 286 s, `demo` 32 s, `real-check` 12 s.
**1,613 tests across 64 binaries, 0 failed** — summed from the 64 `test result: ok.` lines in
the log. M203 measured 1,607 across the same 64, so this slice adds the six tests named above.

Evidence: [`audit/evidence/gate-m204.log`](../../audit/evidence/gate-m204.log).

**Reinstall:** required — the driver is what a user notices about `/goal` — and done.

**What the battery cannot see**, and the honest statement of it: the *decision* is tested at
thirteen points and the round prompt and counting in `epsilon-core`, but nothing runs two whole
rounds through the event loop, because that needs a scripted provider and a terminal at once.
The three lines that join the decision to the trigger are read, not run.
