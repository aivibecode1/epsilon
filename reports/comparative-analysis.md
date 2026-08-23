# Comparative Analysis — Phase 2

Side-by-side comparison of the four audited harnesses across the Phase 1 dimensions, followed by
"who does what best" and the reuse shortlist feeding Epsilon's design. Evidence anchors refer to
sections of `harness-analysis.md` and the source files cited there.

## 1. Side-by-side comparison

| Dimension | pi (0.84.2) | DSH (0.1.1-rc.2) | Hermes (2026.8.19) | opencode (1.18.21) |
|---|---|---|---|---|
| Language / runtime | TypeScript, Node/Bun; ESM-only erasable syntax; Bun-compiled binary option | TypeScript, Node >=22; vendored Cordis kernel | Python 3.11-3.13, exact-pinned deps; Node only for Ink TUI/dashboard | TypeScript on Bun 1.3.14; Effect v4 beta; bun:sqlite (+node fallback) |
| Repo shape | Clean 3-layer monorepo (ai -> agent -> coding-agent) + tui/telemetry/protocol | ~150-package plugin framework on DI kernel; profiles/bundles/patches | God-app root scripts + modularized agent/ core (~149k LOC) | 32-package monorepo mid v1->v2 migration; server/client split |
| Agent loop | Pure two-level loop fn (steering inner / follow-up outer); events out; Agent class reduces state | Phase state machine over durable event log; durable two-lane inbox | Synchronous while-loop w/ budget + retry sub-loop; forwarders from god-files | Effect `while(true)` in session/prompt.ts; one runner per session |
| Streaming | EventStream folded into partial message updates; failures encoded as stopReason | assistant/chunk events persisted then assembled; abort commits interrupted partial | stream chunks consumed synchronously; single-writer gate across fallbacks | AI SDK events mapped to persisted parts; SSE header/chunk watchdogs |
| Tool definition | TypeBox schema + execute(id,args,signal,onUpdate); prepareArguments shim | Schemastery -> JSON Schema; execute(args, ToolRunContext); timeoutMs; isConcurrencySafe | Hand-written JSON-schema dicts; AST import discovery; check_fn availability probes | Effect Schema decoders (zod at plugin boundary); .txt description files; OTEL spans |
| Tool execution | Parallel preflight + concurrent run; batch-wide sequential degradation; no concurrency cap; fail-all-on-truncation | Bounded rolling pool; exclusive barriers; model-order commit; synthetic results for skipped calls | Concurrent/sequential/segmented modes per assistant message | Sequential through processor; doom-loop brake after 3 identical calls |
| Permissions | None by design (containerize instead) | pre-execute waterfall allow/deny/ask + monotonic deny-only guards; approval service fails closed | Regex danger patterns + contextvar approval state + LLM smart approvals; YOLO frozen at import | Last-match-wins wildcard rulesets ask/deny/allow; once/always; .env & external-dir asks; tree-sitter parsed patterns |
| OS sandboxing | None (docs point to containers/microVMs) | bwrap/Landlock/Seatbelt/Windows-ACL; fail-closed; per-call policy; denial signatures; in-band escalation | Spawn-per-call envs: local/docker/ssh/modal/daytona/singularity; no syscall confinement | None default (codemode experiment; cloud containers) |
| Memory/context | JSONL tree entries incl compaction{summary,tokensBefore,retainedTail}; branch summaries; cache-isolated summary calls | Event log + surface shadowing = non-destructive model-view rewrite; pressure + overflow-retry triggers; spill locators | SQLite WAL (sessions/messages/api_content sidecar/FTS5); prune -> head/tail protect -> summarize -> iterate | SQLite+Drizzle; compaction agent w/ strict template; prune min 20k, tail clamp 2k-15k; overflow replay |
| Prompt build | String assembly conditioned on active tools; <project_instructions> blocks; skills section | Section registry (identity/persona/tool guidance), strict {{vars}}, supersede-on-change runtime snapshot | Three byte-stable tiers stable/context/volatile; rebuilt only on compression; persisted verbatim | Per-model .txt bodies + env/instructions/mcp/skills blocks; model-id sniffing |
| Config format | JSON settings + CLI flags | YAML cordis.yml rows; profiles/bundles/patches; !!js expressions | cli-config.yaml (1,914-line example); exact-pin supply policy | JSONC global+project merge; ${VAR} substitution; remote URLs |
| Extensibility | jiti-loaded same-process TS extensions; wide typed API incl. TUI takeover; skills spec | Plugins-as-packages; Claude Code/Codex hook bridges; MCP client; ACP; vm-sandboxed self-modification (opt-in) | Python plugins (dirs + pip entry points); hooks around tools/LLM; skills hub + progressive disclosure; MCP client&server | Plugin Hooks interface (chat.params, permission.ask, tool.execute.*, shell.env); MCP full client w/ OAuth; markdown agents |
| Reliability | StreamFn never-throws contract; truncated-toolcall failing | Durable retry ledger; sticky typed turn-end reasons; repeat-tool advisories; repair helpers | Retry sub-loop; stale-stream breakers; cross-process turn leases; ephemeral scaffolding flags excluded from persistence | 5 retries exponential+jitter honoring Retry-After; orphaned-tool cleanup on resume; forceKillAfter |
| UX surface | Own differential-rendering TUI lib; RPC mode for editors; print mode; HTML export | Web app primary (~37 React pkgs); thin CLI; ACP | Rich REPL; Ink TUI over stdio JSON-RPC; ~20 messaging gateways | OpenTUI/SolidJS TUI; Electron desktop; web; share links; themes |
| Subagents/tasks | Not built-in (extension territory) | subagent spawn/fork packages; workflow/goal/plan/schedule subsystems | delegate_task; kanban board; cronjobs | task tool (general/explore agents); experimental background jobs |
| Undo/time-travel | Session tree forks/navigate (logical undo) | Log replay (full audit) | Session restore from SQLite | Shadow-git snapshots + revert/unrevert (file-level undo) |

## 2. Who does what best, and why

- **Loop design: pi.** Pure-function loop + event reduction + precisely specified steering/
  follow-up queues is the cleanest core. DSH's phase machine is more capable (durable inbox,
  resumable) but far heavier; Hermes' sync loop and opencode's Effect loop are harder to reason
  about than pi's ~800 readable lines.
- **Safety architecture: DSH**, by a wide margin - real confinement on all three OS families,
  fail-closed semantics, per-call policy, and escalation that turns sandbox denials into approval
  prompts rather than dead ends.
- **Permission UX: opencode** - declarative last-match-wins rules, once/always memory,
  corrected-input rejection feedback, parser-derived patterns. Defaults (.env asks, external dirs
  ask, doom loops ask) show product maturity.
- **Cache discipline: Hermes** (byte-stable tiers + api_content sidecar + verbatim restore);
  pi's cacheRetention isolation for auxiliary calls is a close second in elegance; DSH's
  cache-preserving summarization prefix is the best single trick.
- **Compaction: DSH** for correctness (shadowing keeps human transcript intact; overflow recovery
  doubles as error recovery), **opencode** for template discipline (strict summary schema +
  protected-tail clamp + mid-turn replay). pi is simplest-complete.
- **Session storage: tie** between pi (JSONL tree: portable, diffable, fork-first) and DSH (event
  log with chunk provenance). SQLite wins queryability (Hermes FTS5, opencode Drizzle); JSONL wins
  portability/git-friendliness. Epsilon can combine: JSONL durable log + derived indexes.
- **Extensibility breadth: pi** (one coherent typed API spanning events/tools/UI/session); **DSH**
  for safety (vm sandbox, opt-in self-modification) and interop (hook bridges); **opencode** for
  ecosystem pragmatics (npm plugin deps).
- **Provider layer: pi** - hand-written adapters over raw HTTP/SSE + generated catalog beats both
  "one SDK to rule them all" (Hermes pays impedance tax in delta handling) and "AI SDK plus twenty
  provider packages" (opencode's heavy surface).
- **Reliability engineering culture: Hermes.** Every fix carries its incident story (SQLite WAL bug
  pinned in Docker; encoding lint born from Windows regressions; GHSA-driven approval identity).
  Lesson for Epsilon: record *why*, not just what.
- **Multi-process coordination: Hermes** (turn leases) - the only harness solving concurrent
  frontends sharing one state store safely.
- **Undo: opencode** (shadow git) is genuinely useful day-to-day; pi's tree forks cover logical
  retry better than file revert.

## 3. Best ideas worth reusing (candidate list for Epsilon)

**From pi**
- Two-level loop with steering/follow-up queues and drain modes.
- StreamFn never-throws failure contract (failures encoded in-band as stopReason).
- Typed AgentEvent stream consumed by reducer-style UIs.
- Five-stage tool pipeline: prepare -> validate -> before-gate -> execute(onUpdate) -> after-hook.
- Append-only JSONL session tree with typed metadata entries; branch summaries on navigation.
- Fail-all-tool-calls-on-truncation rule.
- Cache-isolated auxiliary LLM calls (fresh session id, no cache writes).
- Agent Skills spec loader (SKILL.md frontmatter, progressive disclosure).

**From DSH**
- Monotonic deny-only guard stage in the tool pipeline.
- OS-confinement seam: per-call policy, fail-closed defaults, denial signatures, in-band
  escalation protocol (retry once with justification -> real approval prompt).
- Surface-shadowing compaction (model view rewritten; human log intact).
- Bounded parallel tool pool with per-call concurrency classification.
- Spill-to-disk locators for oversized outputs instead of truncating blindly.
- Durable steering inbox + durable retry ledger keyed (turn, step, provider).
- Repeat-call advisory guard (loop breaker) at escalating thresholds.
- Section registry for system prompts with cache-friendly runtime snapshots.
- Typed sticky turn-end reasons (max-tokens wins over later completed steps).

**From Hermes**
- Three-tier byte-stable prompt construction (stable/context/volatile).
- api_content sidecar: store exactly what was sent each turn; restore verbatim across restarts.
- Session-turn leases for multi-process safety over one state store.
- TTL + failure-grace availability probes for optional tools (check_fn pattern).
- Iterative rolling summary update: amend the previous summary rather than re-summarize all.
- Ephemeral scaffolding flags excluded from persistence (no replay of internal recovery context).
- Error-classifier-driven provider fallback chains with per-turn primary restoration.
- Incident-driven documentation culture.

**From opencode**
- Last-match-wins wildcard permission rulesets with ask/deny/allow + once/always memory.
- Parser-based shell analysis (tree-sitter bash+powershell) feeding permission patterns.
- Structured compaction template + protected-tail token clamp + mid-turn overflow replay.
- Reserved-token context accounting (usable = limit - reserved).
- Task-queue items inside the loop (subtask, compaction), one per iteration.
- Doom-loop brake tied into the permission gate.
- .txt description files separating tool prose from tool code.
- OpenAPI-generated SDK over a local server for multi-frontend reuse.
- Shadow-VCS snapshots for file-level undo.

## 4. Reuse anti-shortlist (examined and rejected)

- Cordis-style everything-is-a-plugin kernel (DSH): maintenance cost outweighs flexibility at
  Epsilon's scope; a fixed set of trait seams achieves most of the value at a fraction of the cost.
- Vendoring frameworks (DSH vendor/) or patching beta dependencies (opencode effect@4-beta):
  unacceptable supply-chain posture.
- Regex dangerous-command detection as primary safety (Hermes): superseded by parser-based
  analysis plus OS confinement.
- One vendor SDK as universal wire shim (Hermes openai SDK): adapters should speak native wire
  formats to avoid impedance special-cases.
- Model-id substring sniffing for behavior selection (opencode system.ts): use explicit capability
  metadata from the model catalog instead.
- Persisting every raw streaming chunk unconditionally (DSH): tiered retention instead - chunks
  are provenance gold but a storage liability.
- Arbitrary JS expressions in config files (DSH !!js): config must stay declarative data.
- Same-process dynamic-import extension loading without isolation guarantees (pi jiti, opencode
  plugins): Epsilon will use process-isolated extensions from day one (Rust makes this cheap).

## 5. Implications carried into the Epsilon design

1. Port pi's loop/session/tool-pipeline skeleton faithfully - it is the best-tested core - then
   layer DSH/opencode safety and Hermes cache discipline onto it rather than inventing new shapes.
2. Safety gets two layers by default: intent-level permission gates (opencode model) and
   fail-closed confinement where the platform supports it (DSH model), wired together so a
   sandbox denial can escalate into an approval prompt instead of failing the task.
3. Prompt-cache preservation is an invariant from day one: stable section ordering, byte-stable
   assembly, auxiliary calls isolated from the live cache prefix.
4. The extension boundary is an OS process speaking a line-delimited JSON protocol (Rust gives
   cheap isolation); in-process Rust plugins remain possible via a trait object registry but are
   not the default distribution mechanism.
5. Windows is first-class (the development environment is Windows): ConPTY, path handling, ACL-
   based confinement feasibility documented honestly, no POSIX-only assumptions.
6. Everything that reaches the model is reconstructable from the session log ("model-visible means
   logged", adopted from DSH) but raw chunk events get tiered retention to bound storage.
