# M54 — task abort cascade + the `/tasks` view

M53 made delegated children long-lived: a `background: true` child runs detached
and reports back as its own message. Two gaps came with that, both about
control rather than capability:

1. **A runaway child could not be stopped.** Each child is its own `Agent` with
   its own run token, and `abort_current_run` only cancelled the *parent's*
   token — so `Esc` in the TUI, `/abort`, or any other abort left a background
   subagent spending tokens until it finished on its own. A foreground child
   was in the same position: the parent stopped waiting, the child kept going.
2. **The registry was invisible.** `Agent::task_snapshots()` existed for tests
   but no frontend could show it, so "what is still running?" had no answer
   short of watching the transcript.

## What landed

1. **Cascading abort** (`epsilon-core::Agent`): `abort_current_run` now calls
   the new `abort_delegated_tasks()` after cancelling its own token. That walks
   the registry, cancels every child still in `TaskState::Running`, and returns
   how many were asked to stop. Finished tasks are untouched — their output is
   the point of keeping them. The cascade is depth-bounded by construction
   (children at the depth cap cannot delegate further), so it cannot recurse
   into a tree.
2. **Aborted children read as aborted**: `Agent::task_run_failure` maps
   `StopReason::Aborted` to the message *"subagent run aborted"* instead of the
   generic `Debug` fallback, so the registry (and the completion notice) say
   what actually happened rather than `Aborted`.
3. **`/tasks`** (TUI): lists every tracked task as
   `task <id> · <state> · <description> · <turns> turn(s) · <first output line>`,
   then a `N task(s) · M running` summary. An empty registry explains how tasks
   appear instead of printing nothing. `/help` and the slash autocomplete pick
   it up from `SLASH_ITEMS` automatically.

## Key decisions

- **Abort cascades, it does not orphan.** The alternative (children outlive the
  parent's abort) is defensible for a "fire and forget" feature, but it makes
  `Esc` a lie: the user pressed stop and tokens kept burning. A frontend that
  wants children to survive can call `abort_current_run` on the parent from a
  path that skips the cascade — that has to be a deliberate choice, not the
  default.
- **No new event or keybinding.** `/tasks` is a command, not a pane: the
  registry is small, read-only, and the transcript is already where the
  notices land. A live pane can come later if it earns its place.
- **State names come from the registry, not from the UI.** The TUI maps
  `TaskState` to words; it never re-derives "running" from anything else, so
  the view cannot disagree with the mechanism.

## Gotchas

- The TUI test starts a real background child through the loop's host, and the
  test agent's faux provider has no script — so the child legitimately ends
  `failed`. The assertion accepts any of the three states and pins the parts
  that matter (id, description, turns, summary line): asserting `running`
  specifically would have been a race.
- `abort_current_run` returns whether the *parent* had a live token; the count
  of children asked to stop is a separate value. The test asserts both, because
  conflating them would make "abort did nothing" indistinguishable from "no
  children were running".

## Verification

- `cargo test -p epsilon-core --test task_scenarios`: 7 scenarios pass
  (`aborting_the_parent_stops_running_children` added: SlowProvider child →
  parent abort → child settles `failed` with an "aborted" reason → a second
  sweep asks nobody).
- `cargo test -p epsilon-tui --lib`: 135 pass (`/tasks` empty + populated).
- `cargo fmt --check` clean, `clippy --workspace --all-targets` zero
  diagnostics, `cargo test --workspace`: 410 passed, 0 failed (408 before this
  milestone).
- `cargo run -q -p epsilon-cli -- --demo`: completes.

## Follow-ups

- `/tasks abort <id>` (or a keybinding in a tasks pane) if users want to stop
  one child without stopping the run.
- A restart-surviving child session would need the registry to persist; today
  ids and children are process-scoped by design (M53).
