# M50 — pi-parity: agent-type routing for `task` (`agents.ts` + opencode `Agent.get`)

Eighteenth port batch against the 2026-09-03 parity audit (tools findings —
the `subagent_type` gap). `TaskArgs` accepted a `subagent_type` that nothing
read: every child got the full registry, the parent model, and no role prompt
regardless of what the model asked for. Pi's subagent example extension
(`agents.ts` + `agents/*.md`: scout/planner/reviewer/worker) and opencode's
native agents (`general`/`explore` + `describeTask` list) both route by type —
now Epsilon does too.

## What landed

1. **Discovery** (`epsilon-core/src/agents.rs`, new): `AgentDef`
   (`name`/`description`/`tools`/`model`/`system_prompt`/`source`) mirrors pi's
   `AgentConfig`; `discover_agents(user_roots, project_roots, trusted)` mirrors
   `discoverAgents` (user roots always, project roots only when trusted — pi's
   `agentScope: "user"` default maps onto the M39 trust lattice);
   `tools: read, bash` string AND `tools: [read, bash]` list spellings (both in
   pi's sample defs); missing name/description → skip, one bad file never takes
   down the directory; later roots shadow earlier by name; sorted output.
   `describe_task` mirrors opencode's `describeTask` (`- name: description`
   lines, empty when no defs).
2. **Routing** (`Agent::run_task`): `default`/empty = no def (full registry,
   inherited model, no brief — exactly the old behavior). A NAMED type must
   resolve or the failure names what's available (opencode `Unknown agent
   type` verbatim shape). Resolved defs: tool subset via `registry.subset`
   (shared Arcs, unknown names skipped — advisory data must not break
   delegation), `model` override else inherit (pi: omitted model inherits),
   body → `agent_brief` rendered FIRST in the child's CONTEXT tier
   (`<agent_role>`, prefix-stable for cache discipline).
3. **Depth cap** (`task_depth`, opencode `subagent_depth` parity, default 1):
   children stamp depth+1; a child at the cap refuses with the depth message
   instead of recursing the loop. The refusal travels the existing
   `Err → ToolFailure` path so the model sees WHY (no new envelope shape).
4. **Composition**: `agent_user_roots()` / `agent_project_roots(ws, trusted)`
   in `runtime.rs` (same gate pattern as skills/extensions); `build_runtime`
   discovers + `set_agents` + `agents: …` startup line; TUI `/reload`
   re-discovers (trust bit = `!trust_review_pending`) and reports counts.
5. **Tests**: 3 agents-unit (parse/sort/shadow/trust-gate/describe) + 3
   loop-scenario (unknown-names-available, tools+brief+model routing with
   faux-recorded system prompt + tool payload, depth-0 still runs) + 2
   task-tool (type passes verbatim to host, host failure surfaces) + 1
   runtime (roots gating). Existing suites untouched and green.

## Key decisions

- Brief goes in CONTEXT (`<agent_role>`), NOT prepended to the user message:
  first draft merged role+task into one user text (cache-hostile, and pi
  keeps instructions vs input on separate channels — its agent body rides the
  subprocess system prompt, not the task string). Caught by test-first: the
  recorded system prompt didn't contain the brief.
- `default` never resolves a def — even if someone writes a `default.md`, the
  sentinel keeps its old meaning (full child). Named types MUST resolve;
  silent fallback to full-tools would run the WRONG configuration.
- `subset` shares tool Arcs (no clones, no re-registration); the child's
  `task` tool still delegates through ITS host at depth+1, so the cap is
  enforced by construction, not by stripping `task` from children.
- No permission-derivation port (opencode's `deriveSubagentSessionPermission`):
  Epsilon's gate is per-call interactive, not a session ruleset — children
  already re-ask through the shared gate. Recorded as a fork, not a gap.
- No parallel/chain modes, no workflow prompts (`/implement`), no 50KB caps:
  pi's extension surface beyond single-delegation stays out (same deferral
  posture as M47/M48/M49).

## Gotchas

- `agents.rs` tests didn't run until `pub mod agents` was declared — the file
  sat uncompiled while lib tests passed around it. New module = check
  `--list` shows its tests, not just green suites.
- `Option<&[String]>` vs `Option<&[&str]>`: assert with owned
  `"x".to_string()` slices, not string-literal slices.
- `task_depth` as a plain `usize` field can't be stamped post-construction
  through `&Arc<Agent>` — needs `Mutex<usize>` like every other mutable slot.
- The patch tool's lint runs edition-2015: `&&`-let-chains in NEW code trip
  it even though the crate is edition-2024 (use `and_then` or nested `if`).
- Windows linker lock (LNK1104) on `loop_scenarios` retest under DEFAULT
  features: stale hashed exe blocks relink in EVERY target dir tried. Fixed
  by building with `--features real` (fresh hash `2308e67de90ba78c`, links
  and runs in the real `target/`).

## Verification

- `cargo test -p epsilon-core --lib`: 25 passed (+3 agents).
- `cargo test -p epsilon-tools --lib task::`: 6 passed (+2).
- `cargo test --test loop_scenarios -p epsilon-core` (full, `--features real`
  build in real `target/`): 16 passed (+3 M50 routing tests).
- `cargo test -p epsilon-cli` (incl. new roots-gate test): 11 passed.
- `cargo test -p epsilon-tui --lib`: 130 passed.
- fmt clean, workspace clippy zero.
