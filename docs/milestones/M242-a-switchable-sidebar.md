# M242 — the sidebar is switchable, and it shows the agent's own subagents

The activity bar drew one glyph for "there is an activity bar" and the sidebar showed a file
tree, which is one view pretending to be a workspace.

## Two views, because two is what an agent IDE needs

**Files** and **tasks**. One pane holding both would put a task list under a thousand paths,
and a reader who wanted to know whether anything was still running would scroll past the whole
repository to find out.

**Not** a third view for sessions, and the reason is worth recording: a session list that does
not let you *switch* is a list of files with extra steps. Switching is a lease handover —
detach the store, open the other one, reattach — and a "Sessions" view that looks switchable
and is not would be the kind of control this repository keeps refusing to ship. It will be
added when it can do the thing a reader expects.

## The tasks view reads from the agent, not from a copy

`Agent::task_snapshots()` is the registry itself. A copy in the window would go stale the
moment a child finished — which is precisely when a reader is looking at it.

Each row carries the **model and the turn count**, not just a description. Those are the two
things a reader cannot work out for themselves, and one of them is a real failure mode: a
delegation that silently ran on a different model from the one that was asked for is exactly
the sort of thing you only notice afterwards.

## An empty pane says what is missing

With no agent it says *no agent attached*; with an agent and no subagents it says *none yet —
the model starts them with the `task` tool*. A list that says only "no tasks" reads as
"subagents are broken", which is the state this repository keeps having to rule out.

## The bar is widgets, not paint

The first version painted a glyph on the activity bar, which was a **picture of a control**.
It is now buttons, so it takes a click — and it records a hotspot per view, which is what let
`the_activity_bar_switches_the_sidebar_and_the_change_is_on_screen` press a real rectangle
through egui's hit testing and then assert on what the *next frame drew*. A test that only
checked the enum would have passed against a window whose buttons were never wired.

## Verification

**Battery `m242` — GATE PASS on attempt 1, 1,781 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m242.log`). Up from 1,779: two window tests.

## What nothing exercises

- **A subagent that is actually running.** The pane reads `task_snapshots()`, and no test
  drives a delegation to see a `running` row with a turn count. The running/completed/failed
  markers are written and never asserted on.
- **Resizing with the bar present.** The bar draws inside `IdeLayout::activity`, and the layout
  tests cover the geometry; what is untested is a click landing correctly after a resize.
- **Keyboard access to the bar.** The buttons are focusable in egui, and nothing here or in
  the desktop uses tab order or a mnemonic yet — the accessibility story for the IDE is
  AccessKit reaching the transcript and the composer, and the bar has not been checked.