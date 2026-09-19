# M58 — Durable child sessions

M53 made delegation stateful and M56 gave it a cost lever, but the state lived
in memory: the task registry dies with the process, so a `task_id` that appears
in a persisted transcript no longer resolved after a restart. The harness was
durable about the parent's conversation and amnesiac about its children's.

Now a delegated task's transcript is a session like any other.

## What landed

**Children get their own session file.** `Agent::attach_child_session` (core)
writes `<parent-stem>-<task-id>.jsonl` next to the parent's session file, with:

- a header whose `parent_session` names the parent session
  (`SessionStore::open_child_in`, new),
- a `task_meta` custom entry (`description`, `subagent_type`, `model`) written
  *before* the child runs, so a crash mid-turn still leaves a resumable task,
- then the child's ordinary message + `TurnEnd` entries, written by the same
  `persist` path the parent uses.

No parent store attached (tests, embedding callers)? Nothing is written and
children behave exactly as before — the fallback is the old behavior, not an
error.

**`task_id` rehydrates from disk.** `Agent::rehydrate_task` runs when a resume
names an id the registry does not know: it finds the child file by id suffix,
opens it (with `repair_on_open`, exactly like a live resume), reads the meta and
the model view, rebuilds a child through the ordinary `build_task_child`
routing, restores the history, attaches the store, and registers the entry so
every existing surface — status probes, further resumes, `/tasks`, notices —
works unchanged. The registry's `turns` counter keeps accumulating across the
restart, which is what makes "the same child" true rather than metaphorical.

**States come back honest.** The rehydrated snapshot derives its state from the
file's last `TurnEnd` entry (`Error`/`Aborted`/`BudgetExhausted`/`Blocked` →
`Failed`, otherwise `Completed`) and its output from the last assistant text,
labelled `[restored from disk]`. A child whose model left the catalog in the
meantime still restores: the rebuild falls back to the inherited model and says
so in the report, because losing the transcript would be worse than losing the
model choice.

**The browser knows them.** M57's `/sessions` rows show `· task child` for a
session with a parent link, so the delegated transcripts that now appear in the
directory read as a family rather than as clutter.

## Key decisions

- **A child's transcript is a session, not a sidecar.** The store already had
  `parent_session` in its header (pi's field, unused until now), `Custom`
  entries for provider-specific state (the `/name` pattern), and a repair path.
  Inventing a second format for children would have duplicated all three.
- **Persist at spawn, not at completion.** A task that dies mid-turn is exactly
  the one worth resuming; writing the file first means there is always
  something to resume.
- **Rehydrate behind the same `TaskHost` surface.** `delegate_task` gained one
  fallback branch, so the tool, the TUI, and the tests see no new API — a
  restarted process simply has more ids that resolve.
- **The error stays honest when nothing is on disk**: "no such task is tracked
  or on disk (ids come from a `task` call's result)" names both places, because
  a typo and a lost task are different problems.

## Gotchas

- **The Windows append-handle trap, again (M4's lesson).** The first
  implementation re-stamped the header after `open_in` to add the parent link;
  on Windows a write through a `FILE_APPEND_DATA` handle lands at EOF no matter
  where the cursor was seeked, so the file got *two* headers and every read
  after that reported `corrupt session line #2`. Caught by the two new
  scenario tests, fixed by making the parent link part of the creation write
  (`open_with_parent`) instead of a rewrite.
- Children are files: a session with many delegations leaves many small
  transcripts in `.epsilon/sessions`. That is the price of durability with no
  second store; the browser's parent link is what keeps them legible.

## Verification

- `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets`
  zero diagnostics.
- `cargo test --workspace`: **436 passed, 0 failed** (431 before: +3 core
  scenarios, +1 session-store test, +1 picker test). `--features real` lane:
  125 passed, 0 failed.
- New coverage: a session-brick test (creation stamps the parent link; an
  existing child is never re-parented; a plain `open_in` stays unlinked), three
  core scenarios (the child file exists with the parent link + meta + its own
  turn; a task id resumes in a *fresh process* — status probe first, then a
  continued run whose turns accumulate; unknown ids still fail naming both
  places), and a picker test (child rows are flagged).
- The demo task still runs without a store attached (the fallback path).
