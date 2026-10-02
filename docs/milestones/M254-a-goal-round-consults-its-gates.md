# M254 — a goal round consults the goal's gates

M253 recorded this as **A26** and declined to build it, because the only clean wiring needed
`build_runtime` to hand back its gate runner across six call sites, and writing the function
without a caller would have manufactured the very defect the row was about. This milestone
builds it — and the cost came in **under** the estimate, because the seam did not have to be
opened.

## What was wrong

```rust
pub fn goal_begin_round(&self) -> Result<Option<String>, GoalError> {
    ...
    Ok(Some(goal_round_prompt(&goal)))
}
```

Every round sent the same "continue toward the objective" text, having never looked at
`goal.gates`. Hermes runs the gates at the turn boundary and makes a failing gate's bounded
output *the continuation prompt* (`goals.py:97-111`, `:1293-1345`) — explicitly so an
unattended loop cannot spin on a red suite. Epsilon could: a goal with `cargo test` as its
gate spent its whole round budget re-reading one sentence.

## What it does now

```rust
match begin_goal_round(&agent, runner, &cwd).await? {
    GoalRound::Stop        => { /* no goal, not continuable, or out of rounds */ }
    GoalRound::Continue    => { /* the ordinary prompt */ }
    GoalRound::GateFailed  => { /* the gate's own output */ }
}
```

Three decisions worth naming:

**The round is taken *before* the gates run.** A gate failure must cost a round exactly as a
turn does, or a red suite is retried for free forever. Hermes says it in as many words — a
quality gate consumes "the same turn budget". A gate check that ran first and then took a
round would have been cheaper to write and wrong.

**The prompt keeps Hermes' two load-bearing sentences.** *"then re-run it to confirm"* — a
round that fixes the cause without proving the gate passes has not finished. And *"If the
gate itself is wrong or cannot pass, say so clearly and stop"* — a goal whose gate can never
pass has to be able to end without being declared done. Without that the only exits are a
human or the round cap, and both are worse than a model saying so.

**The gate runs on its own task, not on the frame loop.** `drive` is `async` and could have
awaited it, but a gate is a shell command: `cargo test` on a real project is thirty seconds,
and awaiting that in the frame loop is an unresponsive terminal for the whole run. The round
arrives on a channel and starts in `start_gate_round`, so a slow gate costs thirty seconds of
nothing happening rather than thirty seconds of a dead terminal.

## The seam did not have to be opened

M253 measured the fix as "six call sites". It was not. `build_runtime` keeps its signature
and becomes a one-line delegation over a new `build_runtime_and_gate_runner`, and
`epsilon-tools` gains `builtin_registry_and_gate_runner`, which returns **the runner the
registry was just given** rather than resolving a second sandbox. The one-resolution rule
the tools crate's module header states is preserved by construction: there is only ever one
sandbox object, and it is the same one the shell tool got.

**`epsilon-tools` stayed a dev-dependency of the terminal.** The runner type crosses the
boundary as `epsilon_runtime::GoalGateRunner` — a re-export from the crate that already owns
the sandbox. Promoting tools to a real dependency of a frontend for one type would have put
a tool crate underneath a frontend, and the alternative — resolving a sandbox in the TUI —
is precisely what the header forbids.

## Verification

Battery `m254` — **GATE PASS on attempt 1: 1,826 tests across 75 binaries, 0 failed** (`audit/evidence/gate-m254.log`). Up from 1,820, one new test binary.

Six new tests drive the **real** `begin_goal_round` with a **real** runner against **real**
commands (`cargo --version` passes, `cargo definitely-not-a-subcommand` fails). A test
against a stand-in would be a test of the stand-in, which is the trap
`agents/memory/goal-gates.md` records about `goal_apply` in its own closing paragraph.

They pin: a goal with no gates sends the unchanged prompt; a failing gate's command, exit
code and output are in the round; a passing gate does **not** turn the round into a failure;
**a failing gate still spends a round**; a `None` runner continues and claims nothing
passed; and an exhausted goal stops instead of spending a round.

**Negative control:** with the gate run skipped, the two gate-specific tests fail and the
four that are about the round itself still pass — which is the shape a control should have.

**Reinstall:** yes, and done. This changes `epsilon-cli`'s terminal behaviour and the owner
tests the installed binary. `cargo install --path crates/epsilon-cli --features real
--force` into a temporary root; the demo runs clean from the installed`epsilon.exe`
(`epsilon 0.1.0`, `final reason: Completed`, exit 0). The install went to`%TEMP%\epsilon-install`
rather than over the owner's global `PATH` entry, because replacing a binary
another toolchain may own is an owner's decision and not one to make unattended -
moving it is one `Copy-Item`.

## What nothing exercises

- **The desktop window never consults gates.** It has no round driver at all yet, so this is
  a terminal-only fix; the shared function is ready for the window's driver when it exists.
- **No test drives the TUI's spawn-and-deliver path.** The six tests cover
  `begin_goal_round`; `continue_goal`'s branch that spawns it, and `start_gate_round`, are
  covered only by the existing no-gate round tests.
- **Still no end-to-end gated-goal test** through `UpdateGoalTool` — the half of M253's item 1
  that was true.
- **`workspace_fingerprint` is still never acted on**, so an unchanged workspace re-runs a
  slow gate every round. With the gate now on the round path this starts to cost something
  real.
- **No gate has ever run confined in a test** — every gate test runs `Confinement::Off`, so
  the branch that is the security property remains unexercised.