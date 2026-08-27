# Epsilon Architecture

Epsilon is a **Rust port of pi-agent** (pi-0.84.2), selectively improved with the strongest ideas
from DSH, Hermes, and opencode as documented in `reports/harness-analysis.md` and
`reports/comparative-analysis.md`. This document specifies components, data flow, and module
boundaries. Every major choice is justified in `design/epsilon-decisions.md` (ADR log); each
section below links to its ADRs.

Design non-negotiables (derived from Phase 1-2 evidence):

1. Port pi's loop, session format, and tool pipeline faithfully; improve at the seams, not in the
   core (Comparative Analysis section 5).
2. Two safety layers by default: intent-level permission gates + fail-closed confinement where the
   OS supports it.
3. Prompt-cache preservation is an invariant from day one.
4. Extensions are isolated OS processes speaking a line-delimited JSON protocol.
5. Windows is a first-class target.
6. Model-visible means logged - but raw stream chunks get tiered retention.

---

## 1. System overview

```
+--------------------------------------------------------------------------+
|                              epsilon (binary)                            |
|                                                                          |
|  +------------------+     +------------------+     +------------------+  |
|  | tui (ratatui)    |     | serve (HTTP/SSE) |     | print (-p) / rpc |  |
|  | frontend         |     | + OpenAPI SDK    |     | frontends        |  |
|  +--------+---------+     +--------+---------+     +--------+---------+  |
|           |  subscribe(Event) over channel / SSE   |                     |
|           +----------------------+-----------------+                     |
|                                  v                                       |
|                        +------------------+                              |
|                        | session-runtime  |   (owns Session, Agent,      |
|                        |                  |    extension host, approvals)|
|                        +--------+---------+                              |
|                                 |                                        |
|              prompt()/steer()/followUp()/abort()/compact()               |
|                                 v                                        |
|  +---------------------------- agent-core ---------------------------+   |
|  | Agent: steering/followUp queues, event broadcast, abort token      |   |
|  | run_loop(): outer follow-up loop / inner tool+steering loop        |   |
|  |   turn: transform_context -> convert_to_llm -> StreamFn -> fold    |   |
|  |   tools: prepare -> validate -> before-gate -> execute(on_update)  |   |
|  |          -> after-hook -> ToolResultMessage                        |   |
|  +------+---------------------------------------+---------------------+   |
|         | StreamFn (never panics/throws; failures in-band)     |          |
|         v                                                      v          |
|  +----------------+   +----------------+   +--------------------+         |
|  | provider       |   | permission     |   | sandbox            |         |
|  | (direct HTTP   |   | (rulesets +    |   | (landlock/bwrap/   |         |
|  |  StreamFn impl)|   |  ask UI hook)  |   |  seatbelt/win32)   |         |
|  +----------------+   +----------------+   +--------------------+         |
|                                                                          |
|  +---------------------+  +--------------+  +------------------------+   |
|  | session-store       |  | ext-host     |  | mcp-client (rmcp)      |   |
|  | JSONL tree + index  |  | child procs  |  | external tools         |   |
|  +---------------------+  +--------------+  +------------------------+   |
+--------------------------------------------------------------------------+
```

## 2. Workspace layout (Cargo)

Rust workspace, one crate per architectural seam, mirroring pi's layer discipline so that
`epsilon-llm` cannot depend on anything above it. `epsilon-contracts` is the shared vocabulary
crate (the "stud standard" of the lego methodology, see `docs/qa/02-lego-brick-methodology.md`);
it holds types and traits only and depends on nothing internal:

| Crate | Role | Pi analogue |
|---|---|---|
| `epsilon-contracts` | Shared vocabulary: Message/ContentBlock/StreamEvent/AgentEvent/TurnEndReason/Usage, SessionEntry shapes, Tool trait. Zero logic, zero internal deps | (types split out of `packages/agent/types.ts`) |
| `epsilon-llm` | Unified multi-provider LLM API: StreamFn trait, faux provider, direct per-provider HTTP wire adapters (Anthropic / OpenAI-compat / Google, pi-style; M17), retry policy, model catalog | `packages/ai` |
| `epsilon-session` | JSONL tree session store: append/read/fork/navigate, shadow views, leases, crash repair, replay CLI | `packages/agent/src/harness/session` |
| `epsilon-core` | Agent loop, Agent state machine, steering/follow-up queues, event broadcast, tool pipeline driver; compaction engine + prompt builder attach here in later milestones | `packages/agent` |
| `epsilon-tools` | Built-in tools: read, write, edit, bash, grep, glob, ls, todo, task(subagent) — `webfetch` deferred (rule exists in `epsilon-safety`, ships next milestone) | `coding-agent/src/core/tools` |
| `epsilon-safety` | Permission rulesets + evaluation, approval service trait, command analysis, sandbox backends (Linux landlock/bubblewrap, macOS seatbelt-exec, Windows: documented best-effort) | none in pi (new) |
| `epsilon-ext` | Extension host: process spawning, line-delimited JSON-RPC protocol, manifest validation, lifecycle supervision | `core/extensions` (in-process jiti) |
| `epsilon-mcp` | MCP client via `rmcp`, tool bridging into the registry | none in pi core |
| `epsilon-tui` | Terminal frontend on ratatui + crossterm | `packages/tui` + interactive mode |
| `epsilon-server` | Optional local HTTP/SSE server exposing the same runtime to other frontends; OpenAPI spec emitted from route types | `pi-protocol`/RPC mode |
| `epsilon-cli` | The binary: arg parsing, config loading, mode dispatch, project trust, AGENTS.md discovery | `coding-agent` CLI |

Dependency direction is compile-enforced and acyclic upward:

```
contracts <- llm <- core <- {tools, safety, ext, mcp} <- {tui, server} <- cli
contracts <- session <- core
```

No crate may depend on `cli`. Every crate above `contracts` can be compiled- and tested-in
isolation against fakes of the crates below it — that property is the whole point of the
vocabulary crate.

## 3. The agent loop (port of pi's, with named improvements)

`run_loop` keeps pi's exact shape (`references/pi-0.84.2/packages/agent/src/agent-loop.ts`):

- **Outer loop**: while follow-up messages exist, keep running turns.
- **Inner loop**: while the assistant message contained tool calls or steering messages are
  pending, keep taking steps.
- Each step: drain pending messages into context -> `transform_context` (compaction hook) ->
  `convert_to_llm` (AgentMessage[] -> provider Message[]) -> call `StreamFn` -> fold streaming
  events into a partial assistant message -> execute tool calls -> append results -> emit
  `turn_end`.
- Drain points, queue modes ("all" vs "one-at-a-time"), `should_stop_after_turn`,
  `prepare_next_turn` hooks are ported verbatim in semantics.

Improvements layered on (each traced to Phase 1 findings):

1. **Bounded parallel tool pool** (DSH): parallel-safe tool calls run concurrently up to
   `max_parallel_tool_calls` (default 8); any sequential-mode call acts as a barrier; results are
   committed to the transcript in model order regardless of completion order.
2. **Fail-all-on-truncation** (pi): if stop_reason == length, every tool call in the message fails
   with an instructive re-issue error. Unchanged from pi.
3. **Doom-loop brake** (opencode): after N=3 consecutive identical (tool, canonicalized args)
   calls within a run, invoke the permission gate with a `doom_loop` ask; user can allow-once,
   allow-always, or deny-with-guidance. Advisory counters continue at 5/8 (DSH thresholds).
4. **Typed sticky turn-end reasons** (DSH): `TurnEndReason = completed | max_tokens | blocked |
   aborted | error | budget_exhausted`; `max_tokens` remains sticky against later completed steps.
5. **Cancellation**: an owned `CancellationToken` per run replaces AbortSignal; tool futures are
   selected against it; child processes receive kill-on-drop plus explicit terminate with grace
   period.

### Events

Same taxonomy as pi (`agent_start/end`, `turn_start/end`, `message_start/update/end`,
`tool_execution_start/update/end`) plus `permission_requested/decided`,
`compaction_started/finished`, `extension_event`. Events are `Arc<AgentEvent>` values broadcast
over a `tokio::sync::broadcast` channel; the TUI, server SSE, telemetry, and extension host all
subscribe. The `Agent` struct reduces events into state exactly like pi's `processEvents`.

## 4. Tools

Definition (port of pi's `AgentTool`, Rust-shaped):

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn label(&self) -> &str;
    fn description(&self) -> &str;            // prose loaded from .txt file at build time
    fn schema(&self) -> &schemars::Schema;    // JSON Schema 2020-12 via schemars derive
    fn execution(&self) -> ExecutionMode;     // Parallel | Sequential
    async fn execute(&self, call: ToolCall, ctx: ToolContext)
        -> Result<ToolResult, ToolError>;     // errors become isError results, never panics
}
```

- Schemas derive from typed parameter structs via `#[derive(JsonSchema)]` (single source of truth;
  avoids Hermes' hand-written-schema drift). Validation happens once at the gate using the same
  generated schema.
- Descriptions live in sibling `.txt` files included via `include_str!` (opencode pattern) so
  prose diffs never touch code review of logic.
- `ToolContext` carries: session id, cwd, env snapshot, cancellation token, an
  `on_update` callback for streaming partial results (throttled like pi's bash tool), a
  `PermissionSession` handle for `ask()` escalation, and a spill handle for oversized outputs.
- **Spill rule** (DSH): outputs above `spill_threshold_bytes` (default 256 KiB) are written to
  `.epsilon/spill/<hash>` and replaced by head/tail preview + locator path the model can Read.
- Built-in set at launch (deliberately close to pi's surface):
  read, write, edit, bash, grep (ripgrep-style walker), glob, ls.
  MCP tools attach dynamically under `mcp__<server>__<name>`.
  Mutation ordering is enforced by the batch executor's sequential-barrier rule
  (`ExecutionMode::Sequential` on write/edit/bash), matching pi's
  sequential-degradation semantics; there is no edit-specific diff preview or
  mutation queue.
- **Concurrency classification** (DSH): each built-in declares whether it is concurrency-safe
  (read/grep/glob/ls yes; webfetch — planned, see G-M5 — would join the read group; write/edit/bash/todo/task no).

## 5. Safety model (two layers)

Layer 1 - **Permission gates** (opencode model, simplified):

```rust
pub struct Rule { pub capability: Capability, pub pattern: GlobPattern, pub action: Action }
pub enum Capability { Bash, Read, Write, Edit, NetFetch, ExternalDir, DoomLoop }
pub enum Action { Allow, Ask, Deny }
```

- Evaluation is last-match-wins across stacked rulesets (defaults <- global config <-
  project config <- CLI flags), wildcard patterns with `$HOME`/`~/` expansion.
- Defaults mirror opencode's mature posture: reads allowed inside workspace, writes/edits allowed
  inside workspace but asked outside, `.env*` reads ask, network fetches ask first time,
  doom loops ask.
- `Ask` resolves through an `ApprovalUi` trait; TUI shows a dialog, server mode surfaces SSE +
  REST decision endpoint, print mode fails closed with instructions. Decisions support
  once / always-for-session / always-persisted (written to project config) / reject-with-feedback
  (feedback text becomes part of the tool result so the model can correct itself).

Layer 2 - **Confinement** (DSH model, honest about platform reality):

- `Sandbox` trait: `fn confine(&self, cmd: CommandSpec, policy: Policy) -> Result<Confined, SandboxUnavailable>`
  with `Policy::ReadOnly | WorkspaceWrite | FullAccess` riding **per call**.
- Linux: landlock crate (ruleset restricting fs access under workspace + read-only /dev,/usr,/bin)
  or bubblewrap when available; probe order bwrap then landlock; **fail-closed**:
  `SandboxUnavailableError` refuses unconfined execution when policy demands confinement
  (default `WorkspaceWrite` for bash in trusted projects).
- macOS: `sandbox-exec` profile wrapper (documented deprecated-but-functional caveat).
- Windows (first-class dev target, honest limitation): no in-process ACL restricted-token runner at
  v1; instead Epsilon offers job-object based limits (CPU/memory/child caps) + documents that full
  confinement requires running under a container; default policy on Windows degrades to
  Layer 1 gates with an explicit banner. This mirrors DSH's honesty while avoiding shipping a
  half-broken security story.
- **Denial signatures** (DSH): each backend maps its failure modes (EROFS/EACCES/EPERM; bwrap
  codes; job-object denials) into `SandboxDenied { capability, path? }` so the agent can
  distinguish "sandbox blocked this" from "command failed" - and the **escalation protocol**
  kicks in: the model may re-issue the same command with `escalate: "workspace-write"` plus a
  one-sentence justification, which triggers a real approval prompt instead of a dead end.
- **Command analysis**: bash/pwsh commands are parsed by tree-sitter grammars (bash + powershell)
  to extract touched paths and command heads feeding Layer 1 patterns (opencode mechanism).

## 6. Sessions (JSONL tree, ported)

File layout: `.epsilon/sessions/<session-id>.jsonl`. One JSON object per line; entry types are
pi's set verbatim (`message`, `model_change`, `thinking_level_change`, `active_tools_change`,
`compaction`, `branch_summary`, `custom`) with storage-assigned monotonic `seq`, `parentId`
(tree shape for forks), `timestamp`, plus two additions:

- `api_content` sidecar field on assistant entries (Hermes replay invariant): the exact
  request-shape payload hash + optional inline payload for the turn, so sessions replay byte-
  identically across restarts and provider migrations.
- `turn_end` entries carrying the typed `TurnEndReason`.

Writes are append-only with newline-buffered `BufWriter` + explicit flush at durability points
(turn end, tool result, interrupt); crash recovery replays trailing partial lines (last complete
line wins) and repairs dangling tool calls with synthetic error results (DSH repair helpers).

An in-memory **index** (BTreeMap<seq, offset>) gives O(log n) random access without holding the
full transcript in RAM - fixing pi's memory-growth weakness on long sessions; compaction bounds
context size independently.

Forking/navigating the tree works exactly as pi: new branch = entries whose parent chain diverges;
navigation inserts a `branch_summary` entry (LLM-summarized abandoned branch) when configured.

Multi-process safety: a lock file per session (fs4-style advisory lock) implements **turn leases**
(Hermes): a runner must hold the lease to append; other processes can read freely and are told
"busy" on write attempts.

## 7. Context management & compaction

Token accounting uses a pluggable estimator (heuristic chars/4 default, tiktoken-rs optional);
**reserved-token accounting** (opencode): `usable = context_window - reserved_output - safety_margin`.

Triggers (union of the four harnesses' best):

1. Pressure: projected tokens > 80% usable -> compact before next step (DSH pre-step check).
2. Overflow: provider returns context-window-exceeded -> prune + compact + retry once
   (DSH overflow-recovery; bounded retries).
3. Manual `/compact [instructions]`.

Compaction procedure (ordered blend):

1. Prune stale tool results beyond protected tail (model-free pass; DSH/opencode).
2. Protect tail by token budget clamped [2k, 15k] (opencode clamp).
3. Summarize middle range with a strict template (Objective / Important Details / Work State /
   Next Move / Relevant Files; opencode template) using an **isolated cache-safe call** (fresh
   session id, no cache writes - pi) whose request prefix reuses the live conversation's stable
   prefix where the provider supports prefix caching (DSH KV-preserving trick).
4. If a prior summary exists, **amend** it rather than summarize from scratch (Hermes iterative
   update).
5. Persist a `compaction` entry `{summary, tokens_before, retained_tail, details{files_read,
   files_modified}}` (pi entry shape; file-op accumulation kept).
6. Model-view rewrite only: the human transcript retains all entries (DSH surface shadowing
   principle implemented as derived view: compaction entries shadow their covered seq range).

Oversized single tool outputs bypass context entirely via spill locators (section 4).

## 8. Prompt construction (three tiers, byte-stable)

Ported from Hermes with pi's content:

```
Tier STABLE (byte-stable for session lifetime; written once, cached by providers):
  identity block + tool list (name + one-line snippet) + global guidelines
Tier CONTEXT (changes only when project files change; versioned by content hash):
  <project_instructions path="..."> blocks for AGENTS.md family files
  + skills catalog section (names + descriptions only)
Tier VOLATILE (may change per turn; placed last):
  runtime snapshot: cwd, git status summary, date/time, model/thinking level,
  reminders, todo state
```

Rules:

- Assembly is deterministic given inputs; identical inputs produce byte-identical prompts
  (enforced by a unit test hashing assembled output).
- Sections have fixed order and names; extensions register sections into named slots with
  priorities (DSH section registry) instead of replacing whole strings; conflicts are visible.
- Provider cache breakpoints applied at tier boundaries where supported (Anthropic cache_control;
  OpenAI prompt_cache_key) — set directly by our own wire adapters (M17).
- Skills catalog lists name/description only; body loads via the read tool on demand
  (progressive disclosure; Agent Skills spec compliance like pi/Hermes).

## 9. Providers (StreamFn port)

```rust
#[async_trait]
pub trait StreamFn: Send + Sync {
    /// Contract (ported verbatim from pi): NEVER returns Err for provider/model/runtime
    /// failures; failures arrive as a terminal AssistantMessage with stop_reason
    /// Error|Aborted and an error_message. Panics are caught at the boundary.
    async fn stream(&self, model: &Model, ctx: &LlmContext, opts: StreamOptions)
        -> BoxStream<'static, AssistantEvent>;
}
```

- Default implementation is pi-style **direct per-provider HTTP** (M17): a `ProviderWire`
  trait per wire protocol with three adapters — Anthropic Messages, OpenAI-compatible
  `/chat/completions`, Google `streamGenerateContent` — each mapping our types to/from that
  provider's native SSE stream (`providers/{anthropic,openai_compat,google}.rs` + `http.rs`).
  No SDK/router crate sits in between; routing reads the catalog row's `wire` column
  (`WireApi`), never model-id substrings. Custom endpoints/providers register additional
  `StreamFn`s (pi's registerProvider).
- Retry: exponential backoff with decorrelated jitter honoring Retry-After (bounded), classified
  by error kind (rate-limit/server/network/auth-nonretryable); auxiliary calls (summaries, titles)
  use fresh ids and no cache writes (pi).
- Model catalog: static generated table + models.dev refresh option (opencode), with explicit
  capability metadata (context window, max output, vision, reasoning, real prices) — **no
  model-id substring sniffing** anywhere in behavior selection (anti-pattern from opencode).
  Live listing in `real` builds is a plain authenticated GET `/models` per provider.
- Fallback chains: optional ordered fallback models activated on classified failures, restored to
  primary next turn (Hermes).

## 10. Extensions (process-isolated)

Distribution unit: a directory (or single executable/script) with `extension.json`:

```json
{
  "name": "git-safety",
  "version": "0.2.0",
  "entry": { "bin": "./ext-git-safety" },
  "permissions": ["events", "register_tool"],
  "tools": [{ "name": "pr_summary", "description_file": "pr_summary.txt", "schema_file": "pr_summary.schema.json" }]
}
```

Protocol: line-delimited JSON-RPC 2.0 over stdio. Host->ext: `event` broadcasts (same taxonomy as
AgentEvent, filtered by declared interests), `invoke_tool` requests. Ext->host:
`registered_capabilities` handshake, `tool_result`, `partial_result`, `ask_user` (routed through
ApprovalUi), `log`. Every handler call has a timeout (default 10s) and a restart budget
(3 crashes -> disabled for session, visible in footer). Tools declared in the manifest are
validated (schema file present, name-collision checks) before activation.

In-process Rust extensions remain possible via a `ExtensionProvider` trait for first-party
features, but third-party distribution defaults to processes - Rust makes process isolation cheap,
and it eliminates the same-process fragility observed in pi/opencode (Phase 1 weaknesses 2).

MCP servers integrate through the same host machinery (rmcp transports) but appear to the loop as
ordinary registered tools; MCP instructions render into the CONTEXT tier.

## 11. Frontends

- **TUI (default)**: ratatui 0.30.x + crossterm. Layout: scrolling transcript (markdown-rendered
  via pulldown-cmark to styled spans), streaming assistant block with thinking-collapse, tool
  cards (collapsed one-line by default, expandable), editor box with history/kill-ring, status
  footer (model, tokens in/out, cost estimate, context %, extension statuses). Dialog component
  backs ApprovalUi. Themes via TOML palette files. Alt-screen optional; transcript printable.
- **print mode** (`-p "task"`): one-shot run, final answer to stdout; approvals fail closed.
- **serve mode**: axum HTTP server: SSE event stream, REST for prompt/steer/approve/session ops,
  OpenAPI emitted from typed handlers; enables future web/editor frontends without new cores
  (opencode lesson).
- **rpc mode**: line-delimited JSON-RPC on stdio for embedding (editors) without TCP.

## 12. Configuration

Two declarative layers, no executable config:

- Global: `~/.config/epsilon/config.toml` - default model, thinking level, permission ruleset,
  sandbox policy defaults, extension list, theme, telemetry off-by-default toggle.
- Project: `<repo>/.epsilon/config.toml` (+ committed ruleset) - project trust recorded on first
  use (prompt listing what will apply, remembered hash).
- `${VAR}` interpolation from environment only; no code evaluation ever (anti-DSH !!js).
- CLI flags override; effective config dumpable (`epsilon config dump`) like DSH's --dump-config.

## 13. Reliability & observability

- Run-scoped cancellation tokens; kill-on-drop children with grace+force kill ladder.
- Retry ledger entries appended to session (durable, DSH) keyed `(turn, step, provider)` so
  resume recomputes attempt counts.
- Sticky typed turn-end reasons; interrupted runs resume cleanly from the log (lease released,
  dangling tool calls repaired synthetically).
- Observability, shipped honestly: the AgentEvent broadcast bus plus the append-only session
  log ARE the observability surface today — every UI and extension consumes the same stream,
  and any bug reproduces by replaying JSONL (ADR-017). The `tracing` crate / OTLP export
  named in earlier drafts of this section is future work, not shipped code.
- Cost tracking: usage accumulated per session from provider usage events (u64 micro-cent
  accumulator to avoid float drift - ZSeven-W agent-rs pattern), rendered in footer.
- Repeat-call advisory guard and doom-loop brake (sections 3/5).

## 14. What we deliberately do NOT build in v1

- No web/desktop GUI (server API enables them later).
- No messaging-platform gateways (Hermes territory).
- No cron/scheduler, kanban, goals, workflows, multi-agent teams (DSH/Hermes enterprise features).
- No self-modifying plugin toolset (DSH cordis_define analog) - revisit post-1.0 with heavy
  sandboxing.
- No built-in LSP integration (deferred; diagnostics-after-edit via formatters only if trivially
  available).
- No image-generation/vision input beyond passing images to multimodal models.

Each exclusion keeps v1 shippable and auditable; all are additive later because the seams
(extension host, tool registry, server API) already exist.

## 15. Related documents

- `design/epsilon-decisions.md` - ADR log justifying every major decision above.
- `design/epsilon-skills.md` - bundled/referenced/excluded skill catalog with justifications.
- `crates/epsilon-core/assets/agent-contract.md` - the agent behavior contract Epsilon embeds
  into its CONTEXT tier at runtime (include_str! lands with the M7 prompt builder).
- Root `AGENTS.md` - development rules for working on THIS repository.
- `reports/harness-analysis.md`, `reports/comparative-analysis.md` - evidentiary foundation.
- README "Credits and references" - which harness each idea came from, with links.

---

Verification notes (researched during design, see ADRs for citations):

- tokio 1.53.x current / 1.51.x LTS until 2027-03, MSRV 1.71 (tokio releases, crates.io, July 2026).
- ratatui 0.30.2 current (June 2026 release notes).
- rmcp 3.1.4 official MCP Rust SDK implementing spec 2026-07-28 (crates.io, Aug 2026).
- Workspace MSRV is 1.88 (ratatui 0.30 and rmcp 3.1 both declare it; M9b raised the
  original 1.85 plan). CI runs an explicit 1.88 toolchain check.
- landlock 0.4.7 (crates.io, July 2026); Landlock ABI v8+ supports all-thread enforcement.
- portable-pty (wezterm) provides cross-platform PTY incl. ConPTY.
- schemars 1.2.x generates JSON Schema 2020-12 from derives (crates.io, July 2026).
- genai 0.6.5 served as the multi-provider transport through M16; the M17 pi-port
  replaced it with direct per-provider HTTP (`reqwest =0.13.4`, already in-tree via
  rmcp), removing its silent id-routing and env-var spelling failure classes entirely.
