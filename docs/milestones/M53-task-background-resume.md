# M53 — `task`: background runs, resumption, completion notices

M50 gave `task` an isolated child host and left two parameters honest-but-dead:
`background: true` was refused ("not yet supported") and `task_id` answered
*"resumption deferred; started fresh"*. This milestone wires both. Delegated
children now live in a per-session registry: a background child returns its id
immediately and reports back as its own message, and `task_id` continues the
same child (or reports its status) instead of silently inventing a new one.

## What landed

1. **Contract** (`epsilon-contracts/src/tool.rs`): `TaskRequest` (prompt,
   description, subagent_type, background, resume), `TaskOutcome`
   (`Completed(TaskHostResult)` | `Running(TaskSnapshot)`), `TaskSnapshot`
   (id, description, subagent_type, state, output, turns, usage) and
   `TaskState` (`Running` | `Completed` | `Failed`). `TaskHost` grew
   `task_snapshot` / `task_ids` (defaulted) so the tool layer can poll and
   resume without knowing anything about the loop.
2. **Core registry** (`epsilon-core::Agent`): `build_task_child` (M50's
   routing, extracted), a bounded registry (`Vec<TaskEntry>`, cap 16 with
   oldest-finished eviction, since a long session must not accumulate live
   children), and `delegate_task` with four paths:
   - fresh + foreground → build, await, record, envelope (task id exposed in
     `details`);
   - fresh + background → register, `tokio::spawn` the run, return `Running`;
   - `task_id` + prompt → run the *same* child again (its transcript is
     intact), turns accumulate;
   - `task_id` + empty prompt → status report, never a run.
3. **Completion notices.** A finished background child sends a user-shaped
   `<task … state="completed">` message into a channel that `drain_steering`
   merges with the steering queue — so the notice reaches the model at the
   next turn, or at the next run's start when the session is idle. The same
   drain point emits `AgentEvent::MessageEnd` for the notice: previously
   harness-injected messages were pushed to history but never emitted, which
   meant the model could act on a message the user never saw in the UI.
4. **Tool** (`epsilon-tools/src/task.rs`): honours `background` and `task_id`
   for real, allows a status probe with no prompt/description, returns the
   task id in `details`, and surfaces unknown ids naming what *is* tracked.
   The description file now teaches background + resume instead of hiding
   them; the old "deferred" rejection is gone.
5. **In-band child failures are failures.** A subagent that dies on a provider
   error still returns `Ok` from `run()` (the never-fail contract delivers the
   error as a message), so `Agent::task_run_failure` maps a produced assistant
   message with a failure stop reason to `TaskState::Failed` and a telling
   `Err` — a dead child must not look like a completed task.

## Key decisions

- **Session-scoped registry, not durable children.** Resumption is a property
  of a live child Agent; ids die with the process and the unknown-id error says
  so. Making children durable would mean a second session-format decision
  (child transcripts in the parent's JSONL or their own file) — deliberately
  later.
- **One notification mechanism.** The notice travels the steering queue and
  uses the `<task>` envelope the tool already speaks; no new event variant, no
  polling requirement for the model. The alternative (a `task_status` tool the
  model must remember to call) was rejected: opencode/DSH push, they don't
  poll.
- **Status probe instead of a second tool.** `task_id` with no prompt is the
  natural reading of the existing schema, and it keeps the toolset small.
- **The registry caps finished tasks, never running ones.** Eviction picks the
  oldest non-running entry, so a background child cannot be dropped mid-flight.
- **Probe fields are optional in the schema, validated for work.** Making
  `description`/`prompt` `serde(default)` is what lets the probe deserialize;
  the tool still requires both for anything that asks for work.

## Gotchas

- **The failure path looked like success.** The first draft marked every child
  whose `run()` returned `Ok` as `Completed` — including the ones that ended
  with an in-band provider error. The failure scenario test caught it; the fix
  is `task_run_failure` above.
- **A host double that always completes hides the probe path.** The first
  tools-side test asserted the probe envelope against a double that answered
  `Completed`, so the assertion failed on text that came from the wrong
  branch. The double now answers probes with a scripted snapshot.
- **`Missing field description`** turned a perfectly good probe call into a
  deserialize error until the schema fields became optional (see decisions).

## Verification

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets`: zero diagnostics.
- `cargo test -p epsilon-core --test task_scenarios`: 6 new scenarios pass —
  background start → settle → notice in the parent's produced messages *and*
  in the event stream, resume accumulating turns on one registry entry,
  status probe without a run, unknown-id error listing tracked ids, resuming a
  running task refused (SlowProvider), child provider failure → `Failed`.
- `cargo test -p epsilon-core`: 16 loop scenarios still green with the new
  host signature (`loop_scenarios` migrated to `TaskRequest`).
- `cargo test -p epsilon-tools`: 39 lib + 11 integration green, including the
  new background/probe/resume/unknown-id tool tests.
- `cargo test --workspace`: 408 passed, 0 failed (399 before this milestone).
- `cargo test -p epsilon-llm --features real`: 112 passed.
- `cargo run -q -p epsilon-cli -- --demo`: completes.

## Follow-ups

- Durable children (ids surviving a restart) and a `/tasks` TUI view over
  `Agent::task_snapshots()`.
- A per-delegation *provider* override: agent defs already override the model
  (M50); wiring a different stream fn per child would let cheap models run
  scouts while the parent stays on the strong one.
