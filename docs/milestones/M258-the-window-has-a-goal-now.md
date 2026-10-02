# M258 — the window has a goal now

`epsilon-desktop` had **no notion of a goal at all**. A grep for `goal` across the crate
returned exactly one hit, and it was a doc comment: `layout.rs:143` promised *"the bottom bar:
goal phase, pending approvals, key hints"* — a bar that was never built.

So a reader who set a goal, or drove an unattended one, watched the window behave exactly as
if it did not exist. The terminal has had `/goal`, gates, rounds and gate-aware continuation
since M240–M257; the window had none of it. For a product whose whole thesis is **one**
application serving both a CLI and an IDE, that is the largest asymmetry left in it — and it
is the same shape this window has produced five other ways: *the work is happening, somewhere
the reader cannot see.*

## What is on screen

A **goal band** above the transcript, where DSH puts its goal strip, because a goal is the
reason the conversation below it is happening. It carries only what a reader needs to decide
whether to leave it running:

- the **phase** and the **round budget** — an unattended goal's only budget, so a reader who
  cannot see it has no way of knowing whether to walk away;
- the **gate count**, because "no gates" and "a gate passed" are different claims;
- the **objective**, shortened with an ellipsis rather than clipped, per the rule every list
  in this window has followed since M247;
- **whether it will continue on its own**, in words — `not armed — it will not continue`. An
  active goal that is stopped looks exactly like a finished one from across a room.

With no goal there is no band, so the transcript gets the space back.

## What happens behind it

A round driver, with the same shape as the terminal's and the same reason:

- **Off the paint thread.** A gate is a shell command — `cargo test` is thirty seconds — and
  awaiting that in `paint` would freeze the window, which is the exact failure this window
  has already produced five other ways. The round is spawned and its answer arrives on a
  channel the next frames drain.
- **`tokio::spawn` panics outside a runtime.** The product's window runs inside one, so this
  is not the product path — but a paint with no runtime is also a test, and a round driver
  that takes the window down rather than skipping a round is the wrong trade in both. The
  driver checks `Handle::try_current()` and gives the round back.
- **The gate runner is the one the agent already has.** `IdeSession::start` now calls
  `build_runtime_and_gate_runner` instead of `build_runtime`, so the runner shares the
  agent's sandbox. Resolving a second one in the window is the thing
  `epsilon-tools`' module header rules out: two confinement answers in one session.
- A gate that fails becomes **two** things the reader can see — the gate's own sentence
  (`goal gate failed: ...`) and the round text carrying its output. A round that said only
  "continuing" would be read as progress.

## Two things the tests caught

**`tokio::spawn` outside a runtime**, which the first three new tests hit immediately. Fixed
as above; the panic would have been a crash on a headless paint.

**A freshly created goal is *armed*.** The unarmed test asserted otherwise and failed. That is
DSH's design: arming happens on creation, and it is **resume** that disarms — a goal that was
armed when the process died comes back disarmed so the model cannot auto-continue it until a
human re-arms (`packages/goal/goal/src/types.ts:72`). The test now disarms explicitly, which
is the state a resumed goal is actually in. The first version of the test would have passed
on a build that never drew the state line at all.

## Verification

Battery `m258` — **GATE PASS on attempt 1: 1,842 tests across 76 binaries, 0 failed**
(`audit/evidence/gate-m258.log`). Up from 1,837. `epsilon-desktop` is 104 unit + 26 window.

**Negative control:** with the band suppressed, the two display tests fail and the two
*absence* tests still pass — which is the shape a control should have, since "no goal draws
no band" must survive the band being removed.

**Reinstall:** none. `epsilon-cli` and `epsilon-tui` are untouched; this is the desktop
binary, which the owner launches directly.

## What nothing exercises

- **Still nothing after tessellation** (A25): this window cannot be seen on this machine, so
  the band's position, its two-line height and whether it reads at a glance are unmeasured
  against a real screen. Its *content* is asserted; its appearance is a hypothesis.
- **The round driver's spawn path is not driven end to end.** The tests drive the *answer*
  half (`start_goal_round`) and the band's *display*; no test starts a round from an armed
  goal, waits for the gate check, and observes the round arriving. That path is two lines
  long and untested end to end.
- **The desktop does not run goal rounds while its window is hidden** in any meaningful
  sense: `pump` runs from `App::logic`, so a round *can* start in the background, but nothing
  observes it, and the band only refreshes when the window paints.
- **No `/goal` command in the window.** The palette reaches four things (M251); there is no
  way to create, pause or clear a goal from the desktop, only to watch one. That is the next
  slice, and it wants the command table rather than another hand-written palette entry.