# M246 — the subagent pane's states, which were drawn and never checked

M242 shipped the tasks view and recorded, honestly, that the running / completed / failed
markers were "written and never asserted on". This is that assertion.

## A live delegation is a race, and a flaky test is worse than no test

The obvious test — start a background subagent, paint, expect `▸` — can lose: a faux child
finishes in microseconds, so the frame may arrive after it has already settled, and the test
fails for a reason that has nothing to do with the code.

So the **state machine** is tested and the **acquisition** stays where it is already covered.
`Agent::task_snapshots()` and the dispatch machinery belong to `epsilon-core`, whose own
`task_scenarios.rs` drives real delegations and asserts on their states. What the window owns
is turning a snapshot into two lines, and that was the untested part.

`IdeApp::task_lines` and `IdeApp::task_header` exist for exactly that reason: not because the
draw loop needed them, but because a rule you cannot call without a `Ui` cannot be tested
without a `Ui`.

## What is now pinned

- **A running child, a finished one and a failed one read differently.** `▸` / `✓` / `✗` are
  three glyphs, and the assertion is that the three lines differ — not that they are merely
  non-empty. A pane where every state draws the same thing is a pane where a failed
  delegation looks like a successful one.
- **Every row names the model and the turn count.** A delegation that silently ran on a
  different model from the one that was asked for is invisible unless the model is on the
  row, which is the whole reason the detail line exists.
- **A snapshot with no model prints no model.** Not `" · "` — a trailing separator is a
  value that is *missing* printed as though it were empty, which is the exact difference
  between "we don't know" and "there is nothing".
- **`1 task`, not `1 task(s)`.** Four characters, and "1 task(s), 1 running" is the kind of
  row a reader stops reading.

## Verification

**Battery `m246` — GATE PASS on attempt 1, 1,790 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m246.log`). Up from 1,787: three unit tests.

The end-to-end path is unchanged: a real subagent's state still reaches the pane through
`task_snapshots()`, and `epsilon-core`'s `task_scenarios.rs` still proves the delegation
machinery. What changed is that the *rendering* is no longer trusted.

## What nothing exercises

- **A task pane painted while a child is genuinely mid-run.** The acquisition is tested in
  core; the rendering is tested here; **the two together** — a live `Running` row appearing
  in a painted frame — is not. Making it deterministic would need a provider the test can
  hold open, which is a fixture worth building and is not built.
- **The count line against a mixed set.** `task_header` is tested with one running task; a list
  of five with two running is arithmetic the same function does and nobody has watched.
- **Long descriptions and long model ids.** Both go on one line with no truncation, so a
  264-point pane will clip them. Nothing clips them on purpose and nothing tests what happens
  when they are clipped.