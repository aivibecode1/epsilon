# M253 — a finding recorded instead of a feature shipped, and a palette you can click

Two things, and the first is the more useful one: **a half-built capability, measured
properly and written down rather than started.**

## The goal round driver never consults the gates it can declare

Epsilon can declare a goal gate. `/goal edit gate <command>` parses it, sends
`GoalOp::Edit { gates }`, and the contract carries the field. The gate runner is built from
the same sandbox as `bash`, so there is one confinement resolution and a gate cannot become
a confused deputy. Gates run **before** completion is recorded, and completion is refused —
with an explanation — when a gate cannot run or has not passed.

All of that is correct, and all of it is worth having. And then:

```rust
pub fn goal_begin_round(&self) -> Result<Option<String>, GoalError> {
    ...
    Ok(Some(goal_round_prompt(&goal)))
}
```

The round driver takes the round, persists it, and returns **the same "continue toward the
objective" text every round**. It never looks at `goal.gates`.

Hermes runs the gates at the turn boundary and makes a failing gate's bounded output *the
continuation prompt* (`goals.py:97-111`, `:1293-1345`), explicitly so an unattended loop
cannot spin on a red suite. Epsilon's loop can: a goal with `cargo test` as its gate spends
its entire round budget re-reading the same sentence while the suite stays red.

**What it would cost, measured rather than guessed.** `GoalGateRunner::run` is `async` — it
spawns a process. `continue_goal` is sync inside the TUI's frame loop. The gate list lives on
`Agent`; the runner lives in `epsilon-tools`, which depends on `epsilon-core`, so core cannot
reach it without a cycle. `epsilon-runtime` depends on both and is the right home — but the
TUI has no runner to hand it, because the sandbox is resolved *inside* `build_runtime` and
that function returns an `Agent`. So the smallest honest fix is `build_runtime` handing back
the runner it already built, across its six call sites, plus a delivery channel so the frame
loop can start the round when the gates finish.

**Why nothing was shipped.** An earlier attempt wrote that function. It was reverted rather
than committed. A primitive with no production caller is the defect class this repository
tracks most carefully — `set_tool_hooks`, `branch_with_summary`, `approve_pending`, and the
`with_gate_runner` call that *looked* like one until it turned out to be wired — and doing
this half-wired would have manufactured another. Recorded as **A26** with the measurements
attached, so the next person starts from facts rather than from a guess about the cost.

### Two stale claims, corrected

Checking this properly turned up two notes that were simply wrong, and they are corrected in
place rather than left to mislead:

- *"No syntax to declare a gate — `GoalOp::Edit` does not carry one."* **False.** It has
  carried `gates` since M241, and `/goal edit gate <cmd>` has parsed it since.
- *"`with_gate_runner` has no production caller."* **False, and nearly acted on.** It is wired
  at `epsilon-tools/src/lib.rs:163-166`. A grep for `with_gate_runner` in `epsilon-runtime`
  returns nothing because the wiring is one crate over. Five rounds of "a gate makes a goal
  uncompletable" was a conclusion drawn from where a name was *not*, which is the same
  mistake the tab-strip hotspot was.

## The palette rows could not be clicked

`Ctrl+P` listed files and commands, and the only way to choose one was to type and press
Enter. A reader who sees the file in the list and reaches for it with the mouse gets a box
that does nothing — **the window ignoring me**, which this window had now produced three
separate ways.

Rows are hit-tested against the rectangles recorded **last frame**, before the list is drawn
again. That is the same discipline as the tab strip, and for the same reason: egui only knows
a widget was pressed once it has been drawn, so a list that reacts to its own answer opens
the file one frame after the highlight moves to it.

The second test clicks the **last** of three rows and asserts it is not the first — one row
would have satisfied "clicking a row opens it", and would have passed if every rectangle were
the same one.

## Verification

Battery `m253` — **GATE PASS on attempt 1: 1,820 tests across 74 binaries, 0 failed** (`audit/evidence/gate-m253.log`). Up from 1,818.

**Negative control:** with the palette click path disarmed, both click tests fail
(`clicking_a_palette_row_opens_it`, `clicking_a_later_row_opens_that_row_and_not_the_first`).

**Reinstall:** none. `epsilon-cli` and `epsilon-tui` are untouched.

## What nothing exercises

- **A26 is not started.** The measurements are in the ledger; nothing is built.
- **The end-to-end gated-goal test still does not exist** — the half of the stale item 1 that
  was true. A test that drives `UpdateGoalTool` with a gated goal has never run.
- **`workspace_fingerprint` is still computed and never acted on**, so an unchanged workspace
  re-runs a slow gate every round.
- **No gate has ever run confined in a test.** Every gate test runs `Confinement::Off`, so
  the production branch is unexercised — the one branch that matters most, because it is the
  security property.
- **The palette still has untested arrow keys**, and the desktop still cannot be seen on this
  machine (A25).