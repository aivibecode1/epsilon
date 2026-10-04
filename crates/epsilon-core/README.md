# epsilon-core

## What this crate is

The agent loop. `Agent` owns the transcript, the steering and follow-up queues, the event
bus and the tool registry; one run turns a prompt into messages, tool batches and a
terminal reason. Around it sit prompt assembly, compaction, the parallel tool pool, the
run guards and the global config reader. It does not own the tools (`epsilon-tools`), the
permission decision (`epsilon-safety`) or composition (`epsilon-runtime`).

## How it connects

- **Depends on:** `epsilon-contracts`, `epsilon-llm`, `epsilon-session`.
- **Depended on by:** `epsilon-tools`, `epsilon-safety`, `epsilon-mcp`, and through those
  `epsilon-runtime`, `epsilon-tui`, `epsilon-server`, `epsilon-desktop`, `epsilon-cli`.
- Re-exports `SessionStore`, `StoreError` and the search types (`src/lib.rs:140`), so a
  front end need not depend on `epsilon-session` directly.

## Entry points

- `Agent` (`src/lib.rs:520`), built by `Agent::new` (`:672`). What a caller calls: `run`
  (`:1839`), `abort_current_run` (`:1902`), `steer` (`:1055`), `subscribe` (`:1050`),
  `attach_session_store` (`:1124`), `compact_now` (`:2722`), `live_resume` (`:939`).
- `LoopLimits` (`:203`) with `for_model` (`:269`); `RunOutcome` (`:280`); `RunError` (`:483`).
- `ToolRegistry` (`src/registry.rs:12`) and the `PermissionGate` seam (`src/gate.rs:51`).
- `prompt` — the three-tier byte-stable system prompt: `Stable` (`src/prompt.rs:31`),
  `ContextTier` (`:53`), `Volatile` (`:77`), `build` (`:125`), `AGENT_CONTRACT` (`:18`).
- `exec` — the bounded parallel tool pool: `run_local` (`src/exec.rs:275`), `BatchOutcome`
  (`:27`), `fail_all_truncated` (`:61`). `compact` — `estimate_messages`
  (`src/compact.rs:17`), `UsageAnchor` (`:30`), `prune_old_tool_results` (`:361`).
- `config` — `global_dir` (`src/config.rs:140`), `custom_providers_in` (`:47`). Also
  `skills`, `agents`, `paths` (workspace-boundary resolution), `references`, `untrusted`,
  `guards`, `verify`, `cache_stats`, `ignore`, `walk`, plus `testing`.

## How to run and test it

No binary of its own — `epsilon-cli`, `epsilon-tui` and `epsilon-desktop` are what run it.
`tests/` holds `loop_scenarios.rs`, `compaction_scenarios.rs`, `queue_conformance.rs`,
`task_scenarios.rs`, `guard_scenarios.rs`, `system_overlay.rs` and others.

```sh
cargo check -p epsilon-core
cargo test -p epsilon-core
```

## Things worth knowing

- `default = ["real"]` forwards to `epsilon-llm/real` (`Cargo.toml:16`), so a per-crate
  `cargo test -p epsilon-core` exercises the same build as `--workspace`.
- One run at a time: `run` returns `RunError::AlreadyRunning` rather than queueing, because
  concurrent runs are a caller bug (`src/lib.rs:1837`). A `RunGuard` clears the flag on
  every exit path, including early failures.
- Everything the loop does is observable as `AgentEvent`s on a broadcast channel
  (`src/lib.rs:1050`). Front ends reduce that stream; they do not poll internals.
- Tool results commit in *model* order even when execution finishes out of order, so a
  transcript replays identically (`src/exec.rs:9`). A `MaxTokens` message fails every call
  in the batch rather than executing truncated JSON (`:59`).
- `ToolRegistry::descriptors` sorts by name on purpose: the same bytes every run is what
  lets a provider prompt cache hit at all (`src/registry.rs:35`).
- A panic in a hook or a delegated task is contained and reported, not propagated; an
  uncontained unwind would skip the cleanup that makes the agent runnable again
  (`src/lib.rs:416`).