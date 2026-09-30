# Epsilon Architecture Decision Records

For contributors tracing design intent and historical amendments. This relocated
record includes pre-existing M33–M79 amendments; adopting it does not certify their
implementation claims. [Current architecture](epsilon-architecture.md) and
[the audit](../../audit/README.md) describe observed behavior. ADR-002 callback parity,
ADR-014 types-only/enforcement claims, M65 observer scope, M68 check outcomes and
M75 final-entry refusal have documented implementation drift. This relocation
changes no runtime behavior or accepted port target.

Format per ADR: **Decision** -> **Alternatives considered** -> **Why this is best for Epsilon**
(citing Phase 1-2 findings) -> **"Is this truly the best possible choice?"** honest answer.
Status: all Accepted for v1 unless noted. Superseded ADRs are kept at the bottom.

Citation key: `HA` = docs/research/harness-analysis.md, `CA` = docs/research/comparative-analysis.md.

---

## ADR-001: Language and runtime - Rust on tokio

**Decision.** Implement Epsilon in Rust (stable, edition 2024) with tokio as the async runtime;
single static binary distribution.

**Alternatives.** (a) Stay TypeScript like pi; (b) Python like Hermes; (c) Go + TS TUI like old
opencode; (d) Zig; (e) async-std or smol instead of tokio.

**Why best for Epsilon.** The task fixes Rust as the port language, but the *shape* of the Rust
usage is a decision. Evidence from Phase 1: pi's weaknesses 5-6 (runtime tax: Node startup, GC
pressure, transcript fully in RAM, unbounded parallel fan-out) and Hermes' weakness 2 (sync loop
ceiling) are runtime limitations, not design flaws - the designs wanted lower latency and real
parallelism and could not have them. Rust gives deterministic latency during token streaming (no
GC pauses - the zylos 2026 survey documents the industry migration of exactly this hot path to
Rust), cheap process isolation for extensions (ADR-014), and memory safety without a runtime tax.
tokio because it is the de-facto standard with LTS channels (verified: 1.53.x current, 1.51.x LTS
to 2027-03, MSRV 1.71) and every crate we depend on (genai, rmcp, axum, ratatui ecosystem)
targets it. Single binary removes pi's Bun-compile workarounds entirely.

**Best possible?** Yes within the port constraint. Honest caveat: Rust slows iteration speed vs
TypeScript; mitigated by keeping module boundaries identical to pi so upstream pi improvements can
be ported mechanically.

---

## ADR-002: Port pi's loop shape verbatim; improve only at named seams

**Decision.** `run_loop` replicates pi's outer/inner loop, queue drain points,
should_stop_after_turn, prepare_next_turn, and event taxonomy semantically one-to-one. Improvements
(bounded pool, doom-loop brake, sticky turn reasons) attach at defined seams.

**M85 amendment — queue defaults.** Both steering and follow-up delivery default
to `OneAtATime`, matching the locally inspected pi 0.84.2 constructor. Explicit
`All` remains available for callers requiring batched delivery. The default and
All-mode provider-context fixtures cover this narrow correction; they do not
establish callback, event-order or full queue parity. The prepare/stop callbacks
promised above remain an open commitment in the current audit.

**M89 amendment — the callbacks exist.** The four seams this ADR promised are now
implemented as `epsilon_contracts::turn::TurnHooks` (M89,
[docs/milestones/M89-turn-hooks.md](../../docs/milestones/M89-turn-hooks.md)): the loop calls
`prepare_next_turn` immediately after `TurnEnd` and `should_stop_after_turn` on the replaced
context, both before any queue polling; a stop ends the run without draining steering or
follow-ups and without the stuck-run notice (pi `agent-loop.ts:224–257`); a returned
snapshot replaces the run's context, model and thinking for its remaining provider requests
and never the session's state; `transform_context` then `convert_to_llm` run when the
provider request is built and write nothing (ADR-007); everything the loop appends to the
transcript during the run rides along into a replacement, and a committed mid-run compaction
invalidates it. The seam is **async** (`#[async_trait]`, matching `ToolHooks` — pi's callbacks
may return a Promise) and a panicking hook is contained at the call site instead of unwinding
the run. Recorded deltas from pi: a snapshot replaces the message context only (pi's
`AgentContext` can also swap `systemPrompt`/`tools`), `new_messages` excludes the run's own
prompt (pi's starts with the prompts), and hooks cannot observe run cancellation (no token is
passed). Queues, `LoopLimits` and the event taxonomy are untouched: a hook can stop a run,
not reinterpret a queue (M85 amendment stands).

**Alternatives.** (a) Adopt DSH's phase-machine-over-event-log as the core; (b) opencode's Effect-
style runner; (c) invent a new loop.

**Why best.** CA section 2: pi's loop is the cleanest of the four (~800 readable lines, pure
functions + stateful wrapper). DSH's durable inbox/phase machine is more capable but its cost is
~150 packages and a vendored DI kernel - complexity Epsilon cannot carry. Porting verbatim also
means pi's extensive behavioral documentation (types.ts contracts) transfers directly to our tests
as executable specifications.

**Best possible?** Yes. The one thing pi's loop lacks that DSH has - crash-resumable mid-run
state - is recovered differently: our JSONL log + lease + repair approach (ADR-006/010) achieves
resume-without-reinventing-the-loop.

---

## ADR-003: Provider layer wraps genai behind our own StreamFn trait

**Decision.** `epsilon-llm` defines a `StreamFn` trait with pi's never-throws failure contract; the
default implementation adapts `genai 0.6.x`; genai types never leak past this crate.

**Amendment (M10b).** The model identifier is agent configuration, not a loop constant:
`Agent::set_model_id` feeds the main stream call, the compaction summarizer, and the VOLATILE
prompt tier (previously hardcoded "faux-1" in all three). genai routes by model name and resolves
auth per family from env, so a real run is `--provider real --model <id>` with no other wiring.
The faux default keeps every prompt-hash test byte-stable.

**Amendment (M70).** The wires now sanitize what a model emits before the loop sees it
(`epsilon-llm/src/sanitize.rs`). Two shapes used to be handled by *losing* intent: a tool call whose
arguments were not valid JSON had its payload replaced with `{}` (so the tool ran with no arguments
and the model was told it forgot a field), and duplicate tool-call ids inside one message reached the
transcript unchanged (providers reject them, and the result pairing became ambiguous, so the 400
arrived one request later). Arguments are now repaired syntactically — trailing commas, truncation
from an output-limit cut, double-encoded strings, unambiguous single quotes — and when nothing can be
read the raw text is handed on, so the failure quotes what the model actually wrote. A repair that
would recover *nothing* is refused rather than returned as `{}`: an empty success is the old bug
wearing a repair's clothes. Nothing here invents a field, reorders a call, or guesses intent; the
reasoning-echo policy stays "wires never replay thinking blocks", which Anthropic and Bedrock already
documented (see their `Thinking` stripping) and which the other wires follow by construction.

**Alternatives.** (a) Hand-write every provider adapter like pi (~40 adapters); (b) use
async-openai only; (c) adopt opencode's ai-sdk-equivalent breadth via many provider crates;
(d) no abstraction, call HTTP directly everywhere.

**Why best.** Pi's hand-written adapter layer is its largest maintenance surface (packages/ai/src/
providers/*: ~40 providers x models files). Rebuilding that by hand duplicates years of protocol
quirk-fixing. genai (verified current: v0.6.5, native protocols for Anthropic/OpenAI/Gemini/xAI/
DeepSeek/OpenRouter/Ollama etc., CacheControl + ReasoningEffort support, no vendor SDK deps -
exactly pi's philosophy implemented already in Rust) gives us the layer at zero cost while our own
trait keeps the dependency swappable (the lesson of Hermes ADR anti-pattern: one SDK shape as
universal shim is brittle; ours hides the shim). async-openai alone would repeat Hermes'
impedance tax for Anthropic/Gemini.

**Best possible?** Mostly. Risk: genai prioritizes ergonomics over depth ("depth secondary" per
its README); if a needed capability (e.g., fine-grained thinking budgets) is missing, the escape
hatch is a custom StreamFn for that provider - the seam exists precisely for this.

**Amendment (M17, superseded by the pi port).** The escape hatch above became the whole
design. genai's concrete failure modes were observed in practice and matched this ADR's
risk call exactly: `AdapterKind::from_model` silently routed bare `kimi-*`/`glm-*`/
`llama-*` ids to its Ollama adapter; it invented env-var spellings (`OPEN_ROUTER_API_KEY`);
it carried no model metadata (forcing a models.dev dependency anyway). Epsilon now speaks
each provider's native HTTP/SSE wire directly - Anthropic Messages, OpenAI-compatible,
Google streamGenerateContent - behind the unchanged `StreamFn` contract, with routing
driven by a per-catalog-row `WireApi` column (pi's `model.api`). The never-fail
contract, retry classification, faux provider, and aux-call isolation all survive
untouched; only the transport underneath swapped. pi attribution and file mapping:
`docs/milestones/M17-pi-port.md`.

---

## ADR-004: Tools defined as typed structs + schemars-derived JSON Schema

**Decision.** Tool parameter types are plain Rust structs deriving Serialize/Deserialize +
JsonSchema (schemars 1.2.x); schema generation and validation share one source of truth;
descriptions load from `.txt` files at compile time.

**Alternatives.** (a) Hand-written JSON Schema dicts (Hermes); (b) schema DSL like TypeBox used
only dynamically (pi); (c) proc-macro DSL of our own.

**Why best.** HA-Hermes weakness 3 documents hand-written-schema drift; pi's TypeBox gets
correctness by defining schema-first, but then TS types must mirror it manually. Deriving both
serde behavior AND wire schema from one struct eliminates the entire drift class while keeping
static typing. schemars verified stable at 1.x generating JSON Schema 2020-12 (the version rmcp
and modern providers expect). `.txt` descriptions adopted from opencode keep prose reviewable
without touching logic (CA: opencode strength).

**Best possible?** Yes for built-ins. For extension tools, manifests carry explicit schema JSON
files (they may be authored in any language), validated at load - same guarantee, different authoring path.

---

## ADR-005: Two-layer safety: permission gates over fail-closed confinement

**Decision.** Layer 1: last-match-wins wildcard rulesets (Capability x Pattern -> Allow/Ask/Deny)
evaluated before execution, with once/session/persisted approval memory and reject-with-feedback.
Layer 2: per-call confinement policies executed through a Sandbox trait (landlock/bwrap on Linux,
sandbox-exec on macOS, job-object limits on Windows v1) failing closed when policy demands
confinement and no backend exists. Sandbox denials escalate in-band (retry with justification ->
approval prompt).

**Alternatives.** (a) Pi's none-by-design; (b) Hermes regex danger detection; (c) single-layer:
opencode gates only; (d) single-layer: DSH confinement only.

**Why best.** CA section 5 implication 2: combine the two strongest layers rather than choosing.
Evidence:
pi ships no gate at all (HA-pi W1 - genuine interactive UX gap); Hermes' regexes are spoofable
(HA-Hermes W4); opencode proves gates can be usable (once/always, corrected feedback, parser-
derived patterns); DSH proves confinement can fail closed across platforms with escalation
instead of dead ends. Parser-based command analysis (tree-sitter bash+powershell grammars) feeds
Layer 1 patterns so they match real semantics, not guesses.

**Windows honesty:** v1 ships job-object resource limits plus Layer 1 only, with an explicit
banner - shipping a fake ACL runner would be worse than documenting the gap (DSH set the standard
for cross-platform honesty; matching its Windows ACL runner is post-v1).

**Best possible?** Yes for v1 scope. Residual risk: tree-sitter parsing of arbitrary shell is
imperfect (aliases, indirection); the second layer exists precisely because Layer 1 is not
trustworthy alone.

**Amendment (M10a, Layer 2 landed).** The `Sandbox` trait exists in epsilon-safety with three
backends: `JobObjectSandbox` (Windows: per-call Job Object with per-process/job memory caps,
active-process child cap, optional CPU-rate hard cap, KILL_ON_JOB_CLOSE; breakaway refused;
grandchild survival tested live), `BwrapSandbox` (Unix: bubblewrap argv — fs/namespace isolation
only; resource caps need cgroups and are explicitly out of v1), and `NullSandbox`
(fail-closed placeholder). Policy rides each call (DSH rule); `Auto` picks the best platform
backend and refuses to run when none exists; `--sandbox off` is an explicit, banner-logged,
transcript-recorded opt-out. Confinement refusals reach the model as `[confinement]`-marked
error results (DSH denial-signature analog) instead of failing the run. **The Windows ACL
restricted-token runner remains deferred with this rationale**: it requires a service-account /
elevation story that cannot be built honestly without a dedicated security review milestone;
Job Objects already deliver the containment guarantees that matter for tool children (resource
caps + kill-on-close + no breakaway). The unsafe FFI lives in exactly one module
(`sandbox::jobobject`) under the AGENTS.md per-crate exception with documented-safety linting.

---

## ADR-006: Sessions are append-only JSONL trees + in-memory index + leases

**Decision.** Keep pi's entry model (`message`, metadata entries, `compaction`,
`branch_summary`, `custom`) with seq/parentId/timestamp; add `api_content` sidecar hashes and
typed turn-end entries. Storage stays line-delimited JSON (git-diffable, portable, greppable).
An in-memory BTreeMap index maps seq->file offset so long sessions stream lazily. Advisory lock =
turn lease serializing multi-process runners.

**Alternatives.** (a) SQLite (Hermes/opencode); (b) full event-sourcing incl. raw chunks (DSH);
(c) pi's RAM-only transcript.

**Why best.** CA: SQLite wins queryability, JSONL wins portability/git-friendliness. For a coding
agent whose sessions users may want in git, diffed, or emailed, JSONL is the better primary;
query needs (search) are served later by a derived index file, not the source of truth. DSH's
chunk-level provenance is valuable but storage-hungry; we take its *principle* ("model-visible
means logged") with tiered retention: api_content stores payload hash always, inline payload
sampled/configurable. Hermes proved cross-process correctness matters (CLI/TUI/server sharing
state) - leases cost ~100 lines and prevent a class of corruption bugs. The index fixes pi's
memory-growth weakness without changing the format.

**Best possible?** Yes. Known trade-off: concurrent-writer analytics queries are weaker than
SQLite; acceptable until a derived SQLite read-model is warranted (additive change).

---

## ADR-007: Compaction = prune, clamp tail, template summary, amend, shadow view

**Decision.** Trigger set {pressure 80% usable, overflow-retry-once, manual}. Procedure: model-free
prune of stale tool results -> protected tail clamped [2k,15k] tokens -> strict-template summary
(Objective/Important Details/Work State/Next Move/Relevant Files) -> amend prior summaries
iteratively -> persist compaction entry with retained_tail + file-op details. Model view shadows
covered ranges; human log intact. Summary calls isolated (fresh id, no cache writes) and prefix-
reusing where supported.

**Alternatives.** Any single harness's procedure verbatim.

**Why best.** Each element is the strongest variant found: DSH contributes triggers+shadowing+cache
trick, opencode the template+clamp+replay discipline, Hermes the amend-not-regenerate rule (cheaper,
more stable summaries), pi the entry format and isolation knobs. CA section 2 names DSH best-for-
correctness and opencode best-for-template; taking both is strictly better than either.

**Best possible?** Yes modulo tuning constants (thresholds configurable; defaults from the most-
tested sources: opencode clamp values, DSH pressure check placement).

**M86 amendment — acceptance, cancellation and projection replacement.** A pass now commits
only after three checks: the summarization turn ended with a non-failure stop reason (pi
`compaction.ts:670-685` rejects `stopReason === "error"` before the append); the answer carried
nonempty text (the old template fallback could replace useful context with a placeholder —
Epsilon's own reliability rule, *not* claimed pi parity: pi's inspected helper has no nonempty
check); and the durable entry appended successfully *before* runtime history is replaced (a
store-backed run reads its provider view from the store, so committing one view only would
leave runtime history and the request view disagreeing). The summarization call is
cancellation-aware at connect and while polling, with a final check immediately before the
commit (pi `agent-session.ts:1854-1881` checks its signal before appending). A
pressure-triggered failure emits a `Notice` naming the reason and suppresses further pressure
attempts for the rest of the run — each retry is a paid provider call — while overflow
recovery reports the reason and ends the run honestly. `model_view` now *replaces* the
projection an earlier compaction built instead of appending to it (pi
`session-manager.ts:418-453` selects the latest compaction on the active path and builds
context from it plus its retained entries): superseded summaries and their tails no longer
ride along in every subsequent request. Aborted turns commit their synthetic terminal message
to runtime history, the durable mirror and a `MessageEnd` event exactly like a completed turn
(pi persists assistant message-end events, `agent-session.ts:639-656`) — the visible partial
answer survives a resume instead of existing only in `RunOutcome.produced`.

---

## ADR-008: Three-tier byte-stable prompt assembly

**Decision.** STABLE (identity/tools/guidelines) / CONTEXT (project instructions + skills catalog,
hash-versioned) / VOLATILE (runtime snapshot, reminders) tiers, fixed section order, deterministic
assembly enforced by test, provider cache breakpoints at tier boundaries.

**Alternatives.** (a) pi's string concat; (b) DSH section registry only; (c) opencode per-model
prompt bodies.

**Why best.** Hermes demonstrated cache discipline as invariant (byte-stable tiers, rebuild-only-
on-compaction) - the deepest of the four (CA). DSH's registry supplies the mechanism (named slots,
no whole-string replacement fights between extensions - fixing HA-pi W4). Per-model prompt bodies
(opencode) rejected: model-id sniffing is an anti-pattern (CA anti-shortlist); capability-driven
guideline injection replaces it.

**Best possible?** Yes; the open risk is tier assignment mistakes (something volatile leaking into
STABLE poisons caches) - guarded by a lint-style unit test asserting volatility classes.

**Amendment (M31).** CONTEXT tier extended with `MEMORY.md` (global `~/.config/epsilon/MEMORY.md` plus workspace `MEMORY.md`), additive with `AGENTS.md`, hash-tagged for cache stability (Hermes `memory_provider` pattern). Deferred `webfetch` stays rule-only until plan 010 ships; `wildcards.rs` upgraded to opencode `wildcard.ts` parity (`*`→`.*`, `?`→`.`, `\`→`/`, `**` collapsed) via `regex` crate.

---

## ADR-009: Extensions are isolated OS processes speaking NDJSON-RPC

**Decision.** Third-party extensions run as child processes with manifest-declared capabilities,
per-call timeouts, crash budgets, filtered event subscriptions, and tool registration validated up
front. First-party in-process trait extensions remain internal-only.

**Alternatives.** (a) pi/opencode same-process modules; (b) WASM sandboxing; (c) DSH vm-sandboxed
self-modification.

**Why best.** HA-pi W2 and HA-opencode W-ext show same-process extension fragility is unsolved
industry-wide; only DSH sandboxes, experimentally. Rust makes OS processes nearly free (fork/exec
or spawn, ~1ms), giving memory isolation, CPU isolation, and language freedom (any language speaks
NDJSON) without a WASM toolchain dependency. Timeouts + restart budgets convert crashes into
visible degradation instead of lost runs. WASM deferred: component-model churn not worth it for v1.

**Best possible?** Yes for safety-per-effort. Trade-off: higher latency per call (~sub-ms overhead,
irrelevant next to LLM/tool latencies) and no direct TUI widget access (extensions get declarative
widget descriptors instead of components - deliberate capability reduction).

---

## ADR-010: Reliability stack - retry ledger, repairs, kill ladders, cost tracking

**Decision.** (1) Retry with decorrelated jitter honoring Retry-After, classified errors, attempts
logged durably keyed (turn, step, provider) for resume-correct accounting (DSH). (2) Crash repair:
dangling tool calls become synthetic error results on resume (DSH repair helpers). (3) Child
processes: kill-on-drop + grace->force ladder (opencode forceKillAfter lesson). (4) Sticky typed
TurnEndReason. (5) Usage-based cost tracking in integer micro-units rendered live (ZSeven-W agent-rs
pattern; avoids f64 drift). (6) Fail-all-tool-calls-on-truncation (pi, ported unchanged). (7)
Doom-loop brake at 3 identical calls -> permission ask; advisory counters at 5/8 (opencode+DSH).

**Amendment (M12).** (1) landed as the generic `Retrying` StreamFn executor — pre-stream failures
only (peek-first-event discipline keeps mid-stream semantics honest), `on_retry` hook lets callers
render notices or append durable ledger entries; the full (turn, step) keying remains open since
the adapter layer cannot see loop positions (recorded honestly rather than faked). (3)+(5) landed:
run-scoped `CancellationToken` flows from `abort_current_run()` through `ToolContext` into both
shell paths (job-tree termination on drop; Esc aborts in the TUI), and the footer renders session
cost from catalog prices (documented placeholders until a registry sync). Advisory counters at 5/8
remain unbuilt — recorded as intentionally dropped for v1: the brake already covers the failure
mode; advisories are UX sugar without a demonstrated need.

**Alternatives.** Ad-hoc per-feature reliability (status quo in most agents).

**Amendment (M62).** A run that dies mid-stream must not discard what the user already saw. Abort had
always salvaged the partial (`aborted_message`); every *failure* path dropped it — the transport read
failure, each wire's provider-error frame (OpenAI-compat, Mistral, Anthropic, OpenAI-Responses,
Bedrock), and the loop's "stream ended without a terminal frame" arm — so a dropped connection could
erase an answer that was already on screen, and the durable session a resume reads back had a hole
where that reasoning used to be. One rule now lives in one place per layer:
`HttpEventStream::failure_message(detail)` on the wire side (the M60 stall watchdog is a caller with
its own wording) and `Agent::failure_message(partial, stop, detail)` in the loop (from which
`aborted_message` is a one-line wrapper). Salvage never becomes a way to fake success: the turn is
always flagged with the real stop reason and an explaining `error_message`, and tool calls in flight
are always dropped because their arguments may be half-written JSON.

**Amendment (M61).** Session durability gets a **write lease owned by the store**. The lock
protocol (Hermes' turn-lease shape) was already specified and implemented in
`epsilon-session/src/lease.rs`, but nothing called it, so two processes could append to one
session and produce a spliced conversation that `repair_on_open` cannot fix (it heals torn tails
and dangling tool calls, not interleaving). The store — the writer — now holds the lease, which
covers every writer path by construction: the CLI open, the `--resume` repair pass,
`Agent::live_resume`, and delegated child sessions. Readers never open a store, so discovery stays
unconstrained.

Two consequences worth recording:

- **The holder heartbeats** (`HEARTBEAT` 10 s, wake-on-demand thread joined on drop). Without it,
  `STALE_AFTER` (30 s) applied to *acquisition time*, so a long-lived session looked identical to a
  crashed one and would have been handed to a second writer — the corruption the lease exists to
  prevent. Staleness now means "no heartbeat for 30 s", i.e. a dead process.
- **Same-process resumes must release first.** A store's lease is file-based, not process-aware, so
  `live_resume` onto the file it already holds detaches (dropping the handle *and* any rollback
  clone, because `Drop` only runs when the last handle goes) before reopening, and restores the
  previous handle if the reopen fails. The alternative — teaching the lease to recognise its own
  pid — would have hidden genuine same-process double-writers.

**Amendment (M136).** The lease **waits out the stale window instead of refusing on first sight.**
The M61 heartbeat above already answers the only question that matters — is anyone still there? —
and it answers it without a platform call. Having that answer, refusing immediately was strictly
worse than useless in the most common case: killing a TUI and reopening it produced "another epsilon
is already writing this session" for up to `STALE_AFTER` (30 s), because the process just killed is
precisely the one not heartbeating. The console probe hit this and had to work around it
(`audit/evidence/console-probe-2026-09-21.md`). Acquisition now polls to the stale deadline, re-reading
the lock each time so a holder that exits mid-wait is taken over from immediately rather than after
the remainder, and only reports `Held` when a heartbeat is still arriving at the deadline.

The alternative — a liveness probe of the recorded pid — was considered and **rejected**, and the
reasoning generalises: it needs a platform call and, in this workspace, `unsafe`, to learn strictly
*less* than the heartbeat already reports, while adding pid reuse as a fresh failure mode. The trade
this amendment accepts is the one worth stating plainly: **in the only case that costs anything, the
holder really is alive, and waiting is the correct behaviour** because the user is looking at a
second window that owns the session. Two consequences: `LeaseOutcome::Held` and
`StoreError::Leased` now carry `waited`, because a refusal that says how long it tried is a
different experience from one that looks like the tool is broken; and `acquire_now` is kept as the
explicit no-wait entry point for callers with their own timeout (and for tests, which must not spend
30 seconds proving a 30-second window).

**Amendment (M60).** The retry stack is now **engaged in production** — it had been
implemented, exported, and unit-tested while no production path wrapped a stream with it, so a
transient 429/503 failed the turn outright. `Agent::retrying_real_stream()` is the single
constructor both the TUI hot-swap and the CLI (direct and fallback chains) use, and scheduled
waits surface as `AgentEvent::Notice` (TUI status + transcript line, CLI stderr — stdout stays the
transcript for `-p`).

Two scope extensions, both narrowed to the same rule — *retry while nothing user-visible has been
emitted*:

- **A live stream that dies before its first delta is retryable.** The executor tracks visible
  output (`Start` does not count; it merely opens the message) and swallows the fresh attempt's
  duplicate `Start`. Once a delta has been forwarded, failures pass through: re-running would
  duplicate or contradict text the user has already read, and discarding it silently is worse than
  an honest error.
- **An idle watchdog on the stream read** (`STREAM_IDLE_LIMIT` 120 s,
  `EPSILON_STREAM_IDLE_SECS` to tune, `0` disables). Elapsed time is never the trigger — silence
  is — so long legitimate streams are untouched; a half-open socket (VPN drop, NAT timeout,
  proxy that swallowed the FIN) can no longer hang a turn forever. Its terminal message is
  salvable prose + a failed turn, and tool calls in flight are **dropped**, never executed, since
  their arguments may be half-written JSON.

**Why best.** Every element is a proven fix for an observed failure mode in Phase 1 codebases;
bundling them into one ADR because they share the session-log substrate. No alternative was found
in any reference for ledger-based resume accounting being *worse* than in-memory counters.

**Best possible?** Yes; complexity contained by implementing all seven against one log abstraction.

---

## ADR-011: Frontends - ratatui TUI default; print/rpc/serve modes share one runtime

**Decision.** Default interactive TUI on ratatui 0.30.x/crossterm (verified current). Modes:
interactive | print | rpc(stdio) | serve(axum+SSE+OpenAPI). One session-runtime beneath all.

**Alternatives.** (a) Web-first (DSH); (b) Electron desktop now; (c) Ink-style JS TUI; (d) TUI
framework alternatives (iocraft, cursive).

**Why best.** The developer audience for a coding agent lives in terminals (all four references
lead with terminal surfaces despite very different bets after that). ratatui is the maintained
Rust standard (22k+ stars, active releases June 2026). serve/rpc exist from day one because
opencode demonstrated the payoff (multi-frontend reuse via one typed API) and pi demonstrated the
cost of retrofitting modes onto a TUI-coupled core (modes/ split came late). OpenAPI-from-types
keeps the API honest.

**Best possible?** Yes for v1. Deferred: web/desktop consume the same SSE/REST - explicitly
designed-for, deliberately unbuilt (architecture doc section 14).

**Amendment (M9b, modes shipped).** interactive (ratatui TUI), print, and serve (axum SSE)
land together; rpc(stdio) stays open for the editor-integration milestone. The shared
runtime is real: CLI composes provider+tools+gate once and hands `Arc<Agent>` to every
frontend; permission asks flow through ONE mechanism (`ChannelApprovalUi`) answered by
the TUI dialog locally or REST remotely. OpenAPI emission deferred with rpc mode ? the
route types are already the single source of truth for it.

**Amendment (M12, all four modes).** rpc(stdio) ships (`--rpc`: NDJSON requests in,
AgentEvent frames out — same runtime and approval channel as serve, transport is the
only difference) and the serve API serves a hand-maintained OpenAPI 3.1 document at
`/openapi.json`, guarded by a drift test asserting spec paths == registered routes.
Codegen (utoipa) deliberately skipped while the surface is seven routes; revisit on
growth.

**Amendment (M92, correlated replies + abort/state on both wire surfaces).** The rpc line
protocol now speaks pi's reply envelope — `type:'response'`, `command`, the request's `id`
echoed on that request's reply, `success`, and `data` on success / `error` on failure — while
keeping the first wire's `ok`/`error` keys, so both generations of client read the same frame
with no translation layer (`crates/epsilon-cli/src/rpc_mode.rs`). Correlation replaces the
old arrival-order convention, which is what makes pi's own client able to consume frames at
all (it resolves only `type:'response'` frames with a matching `id`,
`rpc-client.ts:508-522,540-590`); pi's `type` tag is accepted as an alias for `op`, and a
frame carrying both is read as `op`. New ops: `abort`, `get_state`, `set_thinking_level`.
A dropped broadcast is no longer silent: rpc emits a `lagged` frame carrying the same
`{dropped:n}` object serve's SSE event does, and the reply/notice builders are unit-tested
(pi has no counterpart for the line protocol's dropped-event notice; serve's own shape is the
convention this ADR already fixed). serve gains `POST /abort` and `GET /state`, backed by the
same core calls, with `request_id` optional on prompt and abort bodies and echoed in the
202/409 JSON; both surfaces read ONE payload struct (`SessionState` in `epsilon-contracts`,
types only) so they cannot disagree about shape, while serve keeps HTTP statuses rather than
importing rpc's single string channel. The OpenAPI drift test covers both new paths (this ADR
requires spec == routes). Interop is envelope-level only: pi's command names and payload
dialects (`task`/`text`, `followup`, camel-case state keys) are not adopted, and the deferred
op families answer `unknown op: <name>` with the echoed id instead of pretending. The one
vocabulary difference that IS bridged: pi's thinking-level spelling `xhigh` is accepted as an
input alias for epsilon's canonical `x_high` (the spelling session files already persist);
outbound state keeps the canonical spelling, so a client reading `x_high` and sending back
`xhigh` is the supported loop.

---

## ADR-012: Configuration TOML, declarative only, project trust

**Decision.** Global + project config.toml layers, ${VAR} env interpolation, no code evaluation;
effective-config dump command; first-use project trust prompt recording a content hash.

**Amendment (M11, first slice).** The global layer now exists in minimal form:
`~/.config/epsilon/config.toml` with `[model] id` (written by the picker / `/model`,
read at startup under ADR-018 precedence). Reads fail open; writes are best-effort and
preserve unknown keys. M11 also fixes two conventional paths pending the full layer:
skills at `{global,.epsilon}/skills`, extension manifests at
`{global,.epsilon}/extensions/<dir>/extension.json`. Project layer, trust hash,
ruleset/extension/MCP declarations in config, and `config dump` remain scheduled for M12.

**Amendment (M12, project layer landed).** `.epsilon/config.toml` now carries the
project layer: `[model] id`, `[[rules]]` (capability/pattern/action stacked as a
"project" engine layer over defaults — last-match-wins with any-deny-wins preserved),
`[[extensions]] dir`, `[[mcp_servers]] name/command/args`. First use records a SHA-256
trust hash at `.epsilon/trust.json`; later content changes print a loud review banner
(interactive re-prompt deferred to TUI polish). Precedence for models is now:
`--model` flag > project config > global saved > default.

**Amendment (M22, interactive re-prompt landed — DIRECTION-01).** The deferred
re-prompt is real: `load_project_config` returns an explicit
`TrustDecision::{Absent, Trusted, Unchanged, Changed}` and the TUI holds ALL
submits while a changed config is unreviewed — type `trust` to accept (and
persist nothing until then: the old hash stays on disk so every run re-reports)
or `trust no` to quit. Headless modes keep the M12 banner policy. Rationale:
project rules can tighten OR loosen permissions; silently running against
changed rules was the last banner-only path in the trust story.

**Alternatives.** YAML+!!js (DSH), JSONC (opencode), YAML 1.9k-line example (Hermes), JSON (pi).

**Why best.** HA-DSH W3 flags executable config as an attack vector; Epsilon refuses the category.
TOML over JSON(C): comments matter in committed project configs; serde's toml support is mature;
less error-prone than YAML indentation for the small schemas involved. Project trust adapted from
pi's trust-manager concept with hash pinning so silently changed project rules re-prompt.

**Best possible?** Yes. Limitation accepted: no conditional logic in config at all; platform
gating handled by `[platform.windows]` table overrides instead of expressions.

---

## ADR-013: MCP client via rmcp; tools bridge into the normal registry

**Decision.** epsilon-mcp embeds rmcp 3.1.x (official Rust SDK; spec 2026-07-28 verified) with
stdio/streamable-http transports; discovered tools register under `mcp__<server>__<name>` with
availability probes cached with TTL+grace (Hermes check_fn pattern); server instructions render
into CONTEXT tier; OAuth flows supported via rmcp.

**Amendment (M8, staged delivery).** M8 ships everything EXCEPT the wire transport:
declarative server config, the `Connector` transport seam, and the registry bridge
(qualified names, untrusted-output handling) — all tested against an in-process connector
double. The rmcp-backed stdio connector lands in M9/M10 alongside real-server integration
tests. Staging keeps every shipped line tested while preserving the architecture exactly.

**Amendment (M9b, transport landed).** `StdioConnector` wraps rmcp's child-process
transport behind the unchanged M8 `Connector` seam — the staging bet pays out as zero
bridge changes. Integration tests spawn a REAL rmcp server binary (`mcp-fixture-server`)
over real stdio pipes: handshake, tools/list pagination, and a tools/call round-trip,
all offline. rmcp types stay inside `stdio.rs` per the adapter rule; streamable-http
remains future work (same seam, one more connector).

**Alternatives.** Hand-rolled client (all four references did); defer MCP post-v1.

**Why best.** MCP is table stakes proven by all four references; hand-rolling re-fights protocol
churn the official SDK absorbs (it tracked 2025-11-25 -> 2026-07-28 spec revisions). TTL+grace
probes prevent transient infra flakes from stripping toolsets mid-session (documented Hermes
incident). Naming keeps Claude-compatible conventions (DSH evidence).

**Best possible?** Yes; rmcp 3.x major-version churn is the residual risk - pinned exact versions
per supply-chain policy (ADR-016).

---

## ADR-014: Workspace = 11 crates mirroring pi's seams, with a dedicated contracts crate

*(Amended M79: twelve crates — `epsilon-lsp` joined. The heading is kept as written; the amendment at the
end of this ADR records why.)*

**Decision.** epsilon-{contracts,llm,session,core,tools,safety,ext,mcp,lsp,tui,server,cli};

*Amendment (M79):* `epsilon-lsp` joins as the twelfth member (see the ADR-014 amendment below).
dependency direction compile-enforced upward only; `contracts` holds the shared type vocabulary
and traits with zero logic and zero internal dependencies.

**Alternatives.** Single crate; pi's exact package count (types living inside the agent package,
as originally specified in v0 of this design); DSH's 150.

**Why best.** pi's three-layer discipline is its most transferable property (HA-pi S1), but pi
keeps its types inside `packages/agent` — meaning every consumer of mere *type names* pulls in
the loop machinery. Extracting `epsilon-contracts` makes the lego property real: any brick can
be compiled- and tested-in isolation against fakes, which is the foundation of the debugging
strategy (ADR-017). Splitting `epsilon-session` out of core follows the same logic — it is a
pure-logic brick with its own debug CLI. Eleven crates is still two orders of magnitude below
DSH's sprawl, and each boundary already has multiple consumers on paper.

Supersedes the original "9 crates" version of this ADR (types-in-core). The original honest
answer ("crate boundaries revisited if compile times suffer") stands.

**M89 amendment — seam defaults in contracts (narrow).** `contracts` hosts stateless,
dependency-free *defaults* on seam traits: `ToolHooks` (tool interception) and now `TurnHooks`
(pi's `AgentLoopConfig`), whose `convert_to_llm` default is the harness-visibility filter the
loop used to inline. This is the narrowly defined policy amendment the architecture law asks
for rather than silent drift: each default is a pure function of its arguments, holds no
state, adds no dependency, and is documented as an overridable contract — a host that
implements the trait replaces it outright. Validation and stateful folding stay outside the
charter (`audit/07-inconsistencies.md`); nothing else in `contracts` gains behaviour without
its own amendment.

**Best possible?** Yes for this scope. The one risk of a vocabulary crate is churn: changing a
stud-shape rebuilds every attached brick. Mitigated by the freeze policy in ADR-017.

---

## ADR-017 amendment (M137): declaring `bytes` instead of smuggling its type

**Context.** F03's fix turned `WireStream.inner` from a concrete `reqwest::Response`
into a boxed byte stream, so the chunk-framing behaviour became testable from a
`Vec<Bytes>`. The stream's item type is `bytes::Bytes`, and Rust will not let a crate
name a transitive dependency.

**Decision.** Declare `bytes = "=1.12.1"` in `[workspace.dependencies]` and in
`crates/epsilon-llm/Cargo.toml`.

**Why this and not something cleverer.** Three alternatives were tried or considered
and all are worse:

- *Inference gymnastics* (deriving the item type off `bytes_stream()`'s `impl Stream`,
  or leaving the field to be inferred at each construction site). Both were actually
  attempted during this milestone. They compile no better than naming the type,
  because the type still has to appear in the field declaration, and they cost a
  reader three lines of type theory to avoid a five-line manifest entry.
- *An ADR for a genuinely new crate.* Not applicable — this is not new code.

The decisive point is that `bytes 1.12.1` was **already in `Cargo.lock` and already
compiled into the binary** through reqwest. Declaring it adds zero compilation and
zero attack surface; it only makes an existing type nameable. The dependency policy
exists to stop unreviewed code entering the tree, and this entry is reviewed code that
was already there. Pinning it exact keeps that guarantee.

**Consequence.** Two direct dependencies where there was one, and one more line in the
lockfile's direct-dependency set. The upside is a testable streaming layer, which is
the difference between a defect that survives years (F03) and one that cannot.

---

## ADR-017: Build order and debug tooling (lego-brick methodology)

**Decision.**
1. Build dependency-sorted, one brick at a time, each shippable alone:
   contracts -> session -> llm(+faux) -> core loop -> tools -> safety -> compaction/prompts ->
   ext/mcp -> frontends.
2. Every brick gets a debug tool before it gets a consumer: replay/validate/fork bins for
   session, per-tool CLIs, rule-eval bin, echo extension fixture, mock MCP server.
3. The faux provider (scripted deterministic responses) is a first-class citizen of
   `epsilon-llm`, not test scaffolding — full-loop integration tests run offline, free, and
   millisecond-fast against it (pi's own faux-provider testing pattern).
4. Golden-file tests at the loop seam: given a scripted provider transcript, assert the exact
   AgentEvent sequence and final session file.
5. Debugging philosophy: event sourcing makes every bug reproducible by replaying a session
   JSONL through any brick; no live-only debugging.
6. Brick-earns-existence rule: split a new crate only when a third consumer appears. Freeze the
   contracts crate's public shapes at first stable tag; later changes require an ADR.

**Alternatives.** (a) Feature-complete vertical slice first (TUI early); (b) bottom-up without
debug tooling until integration; (c) DSH-style everything-parallel plugin development.

**Why best.** (a) is the refactor trap opencode and pi both paid for (frontends retrofit onto a
spine that was never proven). (b) defers all debugging to the hardest possible point. (c) is
documented anti-pattern HA-DSH W1. Ordering by dependency with fakes at each seam means the
hardest code (loop) is written against fully-tested, deterministic inputs, and frontends arrive
last when nothing upstream can masquerade as a UI bug.

**Best possible?** Yes for this team size. Honest cost: late visible progress (no pretty UI
until milestones land) — mitigated by the M5 demo bin that prints live events from a scripted
session.

---

## ADR-015: Skills follow the Agent Skills spec; catalog-in-prompt, body-on-demand

**Decision.** SKILL.md directories with frontmatter validation (name/description limits as pi),
global+project discovery respecting ignore files, catalog (name+description only) injected into
CONTEXT tier, bodies loaded via read tool; `disable-model-invocation` honored; slash-command
invocation expands templates.

**Alternatives.** Hermes hub+curator platform; opencode skill permissions only; no skills.

**Why best.** Spec compliance (shared format across pi/Hermes/opencode ecosystems) maximizes
skill reuse at zero format cost. Progressive disclosure keeps prompt cost O(skills) small
(Hermes/opencode evidence). Hub/curator machinery deferred (v1 exclusions).

**Best possible?** Yes for v1; distribution/hub is additive later because discovery is directory-based.

---

## ADR-019: Extensions participate through a request/response channel, not a shared event stream

**Status: accepted and implemented (`ece3af4`, 2026-09-28). Decides Phase 4 item 4.3 and
constrains 4.5.** The transport described here ships; the bridge from `ExtHost` to
`ToolHooks` is the next piece of 4.5 and is not in that commit.

**The problem, stated precisely.** Today `ExtHost::send_event` writes one line to a child's
stdin and returns `()` (`crates/epsilon-ext/src/host.rs:156-175`). The protocol has exactly
two host-to-child frame types, `event` and `invoke`. An extension therefore *observes*; it
cannot affect a run. The `ToolHooks` and `TurnHooks` traits in `epsilon-contracts` are the
correctly-shaped seam for pi's mutating events — `BeforeToolCallResult { block, reason,
terminate }`, result override, context transform, system-prompt replacement, compaction
cancellation — and the agent loop already calls both correctly (`exec.rs:654,782`). But
`set_tool_hooks` and `set_turn_hooks` appear **ten times in this repository and every one is
in `tests/loop_scenarios.rs`**. The seam is built, tested, and unreachable.

**Decision.** Host-to-child frames become a request/response channel. A mutating event is a
*request*: the host writes `{"type":"interpose","id":…,"event":…}` and the child answers
`{"type":"interpose_result","id":…,"decision":…}`. The host applies the answer to the
in-flight tool call. Extension-to-host direction is unchanged and stays fire-and-forget:
`event` frames remain, because a child with nothing to say must never be able to stall a run.

**The constraint that decides the shape** — and the reason this is not a three-line change.
`ExtHost::invoke` held `ChildIo`'s `Mutex<ChildIo>` for the *entire* invocation, across
write, read, and timeout, and `send_event` took the same mutex to write a single line. So
**every other user of that lock stalled behind a running tool call.**

*Correction (2026-09-28, after implementing and measuring).* This paragraph originally said
the old design would **deadlock**. It would not: the old code read the pipe itself, so a
waiting request *was* the reader, and its own reply reached it. What the single lock actually
caused was **serialisation**, which is bad in two measurable ways rather than by hanging:

1. An event forwarded during a slow tool call waited for that call. Measured against the
   pre-ADR-019 design: **9.74 s** to write one line, against a 10 s invoke timeout. The TUI's
   event forwarder is the caller, so the transcript, token counter and cost line all go quiet
   for the duration of every slow tool.
2. A reply cannot be routed to the requester that wanted it, because whoever holds the lock is
   whoever read it first. Correctness under overlap depends on the child never interleaving.

The design decision is unchanged and the fix is the same — a single long-lived reader task
owns stdout and demultiplexes by `id` into per-request oneshots, and the mutex shrinks to
writes — but the *reason* is head-of-line blocking, not deadlock. Recorded here because an ADR
that names a nonexistent failure mode will eventually be used to justify something that does
not help, and because this is the second time in one session a plausible mechanism turned out
to be wrong on contact with a measurement (`docs/history.md`, M181).

Every existing bound — `INVOKE_TIMEOUT_SECS`, `CRASH_BUDGET`, `MAX_FRAME_BYTES`,
`MAX_INVOKE_BYTES` — carries across unchanged, and the last two now bind in the reader
rather than in a caller that no longer reads.

**Where the bridge lives.** Not in `epsilon-ext`: it depends only on `epsilon-contracts`
(`Cargo.toml`), and must keep doing so. `epsilon-core` does not depend on it either.
`epsilon-cli` is the sole crate that sees both (`Cargo.toml:40-43`), and it already owns
`spawn_event_forwarder` (`runtime.rs:733`). The bridge is a `ToolHooks` implementation in
`epsilon-cli` that fans an interpose out to each attached host, and `epsilon-core` gains
nothing but a test that a *foreign* `ToolHooks` can block a call.

**Ordering, which is the point of 4.5 being one item.** The protocol must be able to carry a
reply before the seam is worth wiring. Implementing `ToolHooks`-to-`ExtHost` first would
build a bridge to a fire-and-forget channel — a seam that still cannot reach a real extension,
and a second bridge to rebuild once it can. So: bidirectional frames, then the bridge, then
`ToolHooks`.

**Failure policy, stated because it is the whole design.** An interpose that times out, is
malformed, or comes from a child over its crash budget is treated as **"no opinion"** — the
tool call proceeds. An extension may *add* friction deliberately (that is the feature); it
may never remove the harness's ability to finish a run. This is the same rule
`send_event`'s fire-and-forget design already encodes, and the reason the mutating path is
worth building on top of rather than instead of it.

**Alternatives.**

- **Keep fire-and-forget; expose blocking through the filesystem.** Rejected: it reintroduces
  the IPC-over-a-shared-file pattern the process boundary exists to avoid, and cannot express
  a decision tied to one in-flight call.
- **Have extensions import Rust and implement `ToolHooks` in-process.** Rejected: it is the
  strongest capability and the weakest isolation. A crash in extension code would take the
  harness with it, which is precisely what the 668-line adversarial protocol test suite in
  `tests/host_protocol.rs` exists to make impossible. Capability without a boundary is not
  worth the risk; a boundary with capability is.
- **A single bidirectional RPC with the host as a general server.** Rejected for now: it is
  pi's eventual shape and it is a much larger protocol (32 commands, `rpc_mode.rs` is at 9).
  Worth revisiting once interpose is proven; recorded rather than dismissed.

**Not decided here, and deliberately:** the 27 `ExtensionUIContext` methods, command /
shortcut / flag / renderer registration, message injection, and provider registration. Those
are capabilities *on top of* a channel that can answer, and scoping them is item 4.3's follow
up. Deciding them before the channel exists would be designing an API no transport can carry.

**Best possible?** Not yet, and this ADR says so rather than implying the gap is closed. It
closes the *transport* half of the largest measured port gap (pi: 153 exports / 26 areas;
`epsilon-ext`: 979 LOC, no path from a child to the agent at all). The honest summary after
this lands is "an extension can block a tool call and rewrite its result", not "the extension
SDK is complete".

---

## ADR-016: Supply-chain policy inherited from pi

**Decision.** cargo pinned exact versions for direct deps; `cargo deny` advisories+licenses CI;
no build-scripts-from-network; lockfile reviewed as code; release binaries built reproducibly where
rustc permits.

**Alternatives.** Loose semver ranges; vendoring (DSH).

**Why best.** Both pi and Hermes converged on strict pinning after ecosystem incidents (pi README
policy; Hermes pyproject comment citing its supply-chain event) - convergent evolution is strong
evidence. Vendoring rejected (HA-DSH W5: maintenance burden).

**Best possible?** Within Cargo's model, yes; `cargo vet` adoption planned when team size justifies it.

---

## ADR-018: Model selection — TUI picker over a bundled catalog, env-var auth

**Decision.** Selection precedence everywhere: `--model` flag (ephemeral) > project
`<workspace>/.epsilon/config.toml` `[model] id` (trust-gated, ADR-012) > saved
`~/.config/epsilon/config.toml` `[model] id` (persisted by the picker) > bundled default.
The TUI exposes pi-style `/models` (filter-driven dialog grouped by provider, current model
floated to top, custom-id row for unknown models) and `/model <id>`; `--list-models [provider]`
mirrors it headlessly. Catalog = generated-at-build-time static list in `epsilon-llm::catalog`
(pi's approach), NOT a remote registry: browsing works with zero keys and zero network.
Auth detection only: the picker shows whether the provider's env key is set; Epsilon never
stores credentials (ADR-012's env-interpolation rule).

**Alternatives.** opencode's models.dev registry + `auth login` credential store (better
freshness/convenience; rejected for v1: network dependency at browse time plus a
secrets-on-disk subsystem that contradicts ADR-012); Hermes' provider-plugin profiles
(right shape for exotic auth like OAuth/AWS — revisit when a provider needs them).

**Why best.** pi and opencode — the two TUI-first references — both make an interactive
picker the primary surface and keep CLI flags secondary; users expect it. Bundled catalog +
custom row covers 95% of usage (genai routes arbitrary ids by name anyway); the catalog is
data, so a registry sync later is additive.

**Best possible?** Yes for v1. Deferred: live per-provider listing (genai
`all_model_names`, auth-gated), favorites/recents memory (opencode), credential store.

**Amendment (M22, M46, M51, M52 — credentials on disk, and the project layer).** Two claims
in the decision above are no longer true, and the reasoning is worth keeping rather than
rewriting: "Epsilon never stores credentials" was correct **for v1 and is not any more**, and
the precedence line omitted the project config layer between M12 and this amendment.

*Credentials.* M22 shipped opencode's `auth` store: `epsilon auth login|list|logout` writing
`~/.config/epsilon/auth.json` (0600 where the OS supports it, atomic tempfile+rename, and a
corrupt file is refused rather than overwritten), with the picker showing a stored credential
as well as an env var — **stored wins, env falls back**, the same precedence `credentials_ready`
and live model listing use; without it the login flow set everything up except the one thing a
chat request needs. The rejection reasoning was not wrong, it was *scoped*: the alternative was
refused for v1 because a secrets-on-disk subsystem is a commitment, and ADR-012's
declarative-config rule — no `${VAR}` interpolation, no code evaluation — still holds, because
`auth.json` is **not** config: it is a data file owned by one subsystem, with its own lock, its
own permission mode and its own refusal to clobber. M46 then added the OAuth credential shape
(`StoredCredential::{Api, Oauth}` with liberal reads and canonical writes), M51 the
refresh-before-use path (exchange off the auth lock, locked re-check on persist, failures
falling through to the key path so a stale token never reaches the wire), and M52 the login
ceremonies (PKCE with a loopback listener, RFC 8628 device polling) reachable from both
`epsilon auth login <provider> --oauth [--paste]` and the TUI's `/login <provider> oauth`.
What survives of the original decision is the part that mattered: **Epsilon embeds no
credentials** — client ids are env-or-config and never another project's registration — and the
picker's auth detection is still "is a credential usable", never a value shown on screen. The
label at the top of this ADR is therefore stale: auth here is no longer env-var-only.

*Precedence.* ADR-012's own M12 amendment already recorded the model precedence as
`--model` flag > project config > global saved > default, and the CLI has implemented exactly
that since M12: `crates/epsilon-cli/src/main.rs` resolves the candidate as
`args.model_id.or_else(|| project.model_id.clone()).or_else(epsilon_core::config::load_model_id)`
(the line below its own comment naming the same order), where `project` comes from
`load_project_config`, the trust-gated `.epsilon/config.toml`. So this ADR's short chain was the
stale half, not [`docs/user/configuration.md`](../user/configuration.md), which cites this ADR
for the four-layer chain and matches the code. Corrected above; the project layer is
trust-gated, so a repository cannot silently switch the model a user runs.

**Amendment (2026-08, M17).** Catalog regenerated from a live models.dev fetch with
REAL prices (placeholder era ends; footer cost is now an estimate against public list
prices). Two genai-0.6 routing facts discovered while doing it are now encoded as
catalog invariants: (a) bare ids only where `AdapterKind::from_model` infers the right
provider — everything else carries genai's explicit `namespace::name` form
(`groq::`, `moonshot::`, `zai::`, `open_router::`; genai strips the namespace on the
wire), because bare `kimi-*`/`glm-*`/`llama-*` silently fall through to its Ollama
adapter; (b) OpenRouter's env var under genai is `OPEN_ROUTER_API_KEY`. A unit test
(`ids_either_auto_resolve_or_carry_genai_namespace`) pins rule (a). The
`provider::model` id convention itself is deliberately adapter-independent (pi and
opencode both identify models as provider+model pairs), so a future transport swap
does not strand saved configs. **Resolution (M17):** the open question above was
closed by the pi port - genai is gone, the transport is direct per-provider HTTP,
and the catalog's `wire` column now carries the routing metadata pi-style (see the
ADR-003 amendment and `docs/milestones/M17-pi-port.md`).

**Amendment (M28).** Routing gained a config-declared escape hatch:
`[provider_custom.<name>]` rows in the global config (`base_url` required,
`env_key` optional) let `<name>/<model>` ids stream to user-declared
OpenAI-compatible endpoints (LM Studio, vLLM, fresh cloud providers) on the
existing wire seam — opencode's `provider.options.baseURL`, Hermes'
`base_url_override`. Catalog ids always win over customs; keys follow the
stored-then-env precedence under the custom name. The type and its TOML
parsing live in `epsilon_llm::custom` (next to the routing that consumes it);
the OpenAI-compat wire descriptor went from `&'static str` to owned `String`
so both catalog rows and customs ride one implementation.

---

## Superseded / deferred

**Amendment (M24).** Two previously deferred items from the research pass landed:

- **MCP health probes (DIRECTION-02).** `Connector::is_healthy` with a
  default-trusting implementation; the stdio connector detects dead children
  via rmcp's `is_closed()`; the bridge probes before each call behind a 30s
  TTL, and a failed call resets the cache. Rationale: one relaxed atomic load
  per healthy call is cheap insurance against opaque transport errors; see
  `docs/milestones/M24-mcp-health-and-fallbacks.md`.
- **Provider fallback chains (DIRECTION-03).** New `epsilon_llm::fallback::FallbackProvider`
  (`--fallback <model>`, repeatable) walks candidates within a turn on
  retryable failure classes only (same classification as the retry executor),
  restores the primary next turn by construction (stateless chain head start),
  and never switches after streaming began. ADR-003's never-fail StreamFn
  contract is preserved: all-failed chains still terminate in an in-band
  Error message naming every candidate's last error.

**Amendment (M25).** The footer thinking-level indicator (a deferred item on
the README's future row) landed: `FooterStats` carries the active level and
`stats_parts` renders `think:<level>` between cost and model, hidden while
Off. The TUI mirrors the level in one `App` field written only by
`set_thinking`, the same choke point that updates the agent, so displayed
state cannot drift from executed state (key-path test pins equality).
Cosmetic-only; no loop, wire, or session-format change.

**Amendment (M26).** Per-favorite thinking levels (the other deferred item,
pi scoped-models' optional per-entry level) landed: `[picker.favorite_thinking]`
rows in the global config remember a level per starred model; F2/Shift+F2
cycling overrides the session level when remembered and inherits otherwise;
unstarring clears. Levels apply through `set_thinking`, so the M25 footer
mirror stays exact by construction.


**Amendment (M33).** Bash tool pi-parity boundaries. Two divergences from
`bash.ts` are intentional forks, not gaps: (1) non-zero exits return `Ok` with
`exit_code` in details and `terminate: false` instead of pi's thrown error —
the model receives the failure as information and decides the next step, while
loop control flow (doom-loop brake, abort) keys off `ToolFailure`, never exit
codes. (2) A 120s default timeout with a 600s cap, where pi has no default and
caps at `i32::MAX` ms (no cap in practice). Rationale: unbounded hangs in
automation; 600s stays generous for real builds. What *is* parity as of M33:
`timeout` accepted as an alias for `timeout_secs` (a pi-shaped call is honored,
never silently dropped to the default), and zero rejected as a validation
error per pi `resolveTimeoutMs`, never silently clamped.

**Amendment (M52).** OAuth *login* joins the credential store M46 defined and
M51 consumed. `epsilon-llm/src/oauth_login.rs` ports pi's login half (PKCE with
a loopback callback listener, RFC 8628 device polling, per-provider exchanges)
behind one synchronous orchestrator; the CLI runs it from
`epsilon auth login <provider> --oauth [--paste]`, the TUI from
`/login <provider> oauth` on a worker thread. Three decisions worth naming:
client ids stay **env-or-config, never embedded** (pi ships its own
per-provider registration; a supply-chain rule, not a convenience one); the
provider table only lists flows whose Epsilon wire can spend the resulting
credential (`anthropic`, `openrouter`, `xai` today); and `getrandom` + `base64`
were promoted from transitive to direct dependencies of `epsilon-llm` because
PKCE requires CSPRNG randomness and std provides none.

The same milestone fixed a feature-forwarding bug: `epsilon-cli`'s `real`
feature enabled only `epsilon-llm/real`, so `epsilon-core`'s real code — the
live provider hot-swap M32d wired for `/login` — compiled to its `false` stub
in the installed binary. `real` now forwards to every crate whose real path the
binary uses, and a compile-time marker (`epsilon_core::REAL_PATH_ENABLED`,
bound in the CLI) turns a future omission into a build error.

**Amendment (M53).** `task` delegation became a *stateful* mechanism:
`epsilon-core::Agent` keeps a bounded, session-scoped registry of child agents
(`TaskRequest` / `TaskOutcome` / `TaskSnapshot` in the contracts crate), so
`background: true` returns an id immediately and `task_id` later resumes the
same child or reports its status. Two conventions fall out of it, worth naming
because later work will build on them: (1) a background child's completion
notice is delivered as a user-shaped `<task …>` message through the existing
steering drain point **and** emitted as `AgentEvent::MessageEnd` — one
mechanism, and no message the model can act on stays invisible in the UI;
(2) in-band provider failures are task failures, so a child that died is never
reported as a completed task. Resumption is deliberately process-scoped:
children are live Agents, ids die with the session, and the unknown-id error
says exactly that.

**Amendment (M54).** Delegated-task control surface. `Agent::abort_current_run`
now cascades into `abort_delegated_tasks`, so stopping a run stops the children
it spawned (each child is its own `Agent` with its own token; without the
cascade `Esc` was a lie and background subagents kept spending tokens). The
registry became user-visible through the TUI's `/tasks` command, which renders
`TaskState` verbatim rather than re-deriving state, so the view cannot disagree
with the mechanism.

**Amendment (M55).** Provider hosts that depend on the credential

`ProviderWire::endpoint(model)` had always been enough because every
provider's host is static data. GitHub Copilot breaks that assumption: its bearer carries
the host it was minted for (`proxy-ep=<host>`), and the API endpoint is that
host with the `proxy.` label rewritten to `api.` — so the URL cannot be known
from the catalog row alone. Rather than special-case Copilot inside the
transport, the trait gained `endpoint_for(model, key)` with a default that
delegates to `endpoint`, and the two request sites prefer it. Copilot's wire
overrides it; every other wire was untouched by the change.

Related decisions in the same milestone:

- **A refresh style is an enum, not a flag.** `RefreshStyle::{JsonBody,
  FormBody}` grew `CopilotMint` — a GET with editor headers, no body, no client
  id, whose response is `{token, expires_at}` instead of a rotated pair. The
  three shapes are mutually exclusive, so the type says so and
  `refresh_request_body` returns `None` for the bodyless one.
- **The GitHub token is re-spent, never rotated.** Copilot's "refresh" mints a
  new short-lived bearer from the same long-lived GitHub token, so the stored
  pair's `refresh` survives every rotation; `store_refreshed_oauth_in` keeps
  it, and the credential's extras with it.
- **Credentials may carry provider extras.** `StoredCredential::Oauth` gained
  an optional `env` map (pi's `OAuthCredential` carries `enterpriseUrl`): the
  enterprise domain a Copilot login authenticated against must outlive the
  config that supplied it, because both the wire and every later refresh need
  it. A rotation carries extras forward — they describe the account, not the
  token pair.
- **Copilot's identity headers stay off chat requests.** pi sends
  `COPILOT_HEADERS` on the token mint and `/models` only; the chat path carries
  the bearer alone, and the port follows the reference rather than a client
  convention.

**Amendment (M56).** Delegation gained a cost lever: `TaskRequest.model` lets a
call run its child on a chosen catalog model, with precedence `agent-def model`
→ request → inherited parent model (opencode `next.model ?? parent model`, with
the operator's type configuration kept on top). Two deliberate asymmetries:

- **Strict resolution.** The picker's partial-match rule is right for humans and
  wrong for a machine-to-machine call: `resolve_task_model` accepts exact,
  `provider/id`, and case-insensitive matches only, and *offers* a partial match
  instead of using it. A delegated run that quietly switched models would be a
  cost and quality surprise nobody asked for.
- **The model travels with the registry entry**, not the run: a resumed child
  keeps its model (a `model` on a resume is ignored — documented in the tool
  description), a status report can name the model mid-flight, and every report
  surface (tool envelope, `details`, snapshot contract, TUI `/tasks`, background
  notice) renders the same effective id.

**Amendment (M57).** Session discovery is a session-brick concern, and the
frontends share one switch path.

- `epsilon-session::discover` owns "what is in this directory" (`summarize`,
  `list_sessions`, `latest_session`), with the CLI's `--continue` scan and the
  TUI's browser both built on it. Before this, the answer lived in an
  `epsilon-cli` helper and the TUI simply could not ask the question — a second
  copy would have drifted the moment either frontend changed a filter.
- Reads are **bounded by contract**: the first user message comes from the first
  64 lines, the display name from the last 8 KB, and nothing else is parsed.
  Listing stays flat in session length; a picker that stalls on history is
  broken.
- The TUI's `/resume`, `/import`, and the new browser all call
  `App::switch_session_file`, so the store swap, transcript re-hydration, prompt
  history, and session-name rehydration cannot diverge between surfaces.
- The browser's filter matches title and file name fuzzily but a session's cwd
  only literally: fuzzy subsequence over a long path matches everything, which
  is a filter in name only.

**Amendment (M58).** A delegated task's transcript is a session, and `task_id`
outlives the process that minted it.

- Children are persisted as ordinary sessions (`<parent-stem>-<task-id>.jsonl`)
  with the header's `parent_session` naming their parent. The store already
  carried that field (pi's), `Custom` entries for provider state, and crash
  repair — a second format for children would have duplicated all three.
- The child's file is created **before its first turn**, meta included: the task
  most worth resuming is the one that died mid-turn.
- `task_id` resolution gained one fallback: registry → disk (`rehydrate_task`) →
  honest error naming both places. No new API surfaces; a restarted process
  simply resolves more ids.
- **Snapshot state is derived, never guessed**: the last `TurnEnd` entry decides
  `Completed` vs `Failed`, the output is labelled `[restored from disk]`, and a
  retired model degrades to inheritance with a note instead of failing the
  restore.
- **The header's parent link is written at creation, never re-stamped** — the
  store's handle is append-only and on Windows FILE_APPEND_DATA writes land at
  EOF regardless of the seek position (M4's lesson; the first implementation
  produced two headers and a `corrupt session line #2`).

**Amendment (M59).** The TUI may hand the terminal to a child process, and the
hand-off is owned by the event loop.

- **Suspension happens between frames**, triggered by a flag
  (`App::external_editor_requested`), never from a key handler. Key handlers must
  stay pure reducers; only the loop may touch the terminal. That is what makes
  the request testable without a TTY.
- **A fresh `Terminal` replaces the old one on resume.** Re-using it would keep a
  back buffer describing a screen the child overwrote, so the repaint would skip
  exactly the rows that need redrawing. The child runs with inherited stdio
  through the platform shell (`cmd /C` on Windows, `sh -c` elsewhere) so
  multi-word commands (`code --wait`) behave as the user wrote them.
- **Editor resolution order is pi's** (`[editor] command` → `$VISUAL` →
  `$EDITOR`), but the default *key* deliberately differs: pi binds
  `ctrl+g`, which Epsilon already uses for the live-model refresh. Both
  `ctrl+o` and `alt+e` are defaults for `app.editor.external`, configurable in
  `keybindings.json` like every other binding.
- **Failure never costs the draft**: a non-zero exit, a spawn error, or a
  missing editor leaves the editor content untouched and reports why, because a
  composed prompt is the expensive part.

**Amendment (M63).** Six behaviour changes from the usability audit, each recorded here because
each alters what a user sees.

- **ADR-003 amendment — `real` is a default feature.** The network path was opt-in, so the
  documented one-line install produced a harness that could not reach a single real model while
  the docs showed the `--features real` incantation for people building from a clone. Feature
  flags are for *capabilities*, not for *whether the product works*; the offline-only build now
  needs `--no-default-features` (ADR-017's build-order rule is unaffected: the faux provider is
  still the default *runtime* provider when no credential exists).
- **ADR-011 amendment — headless runs report failure reasons.** `-p` printed
  `no final assistant text produced` for every non-clean ending, discarding
  `AssistantMessage::error_message` and the sticky `TurnEndReason`. A provider error is now
  printed on stderr with the provider's own words, partial text is kept on stdout (M62's
  salvage rule, applied at the CLI boundary), and the exit code stays non-zero. The TUI already
  behaved this way; the rule is now uniform across frontends.
- **ADR-018 amendment — model resolution is credential-aware.** A loose model reference
  (`--model deepseek-chat`) could resolve to a provider the user has no credential for while a
  sibling row for the same reference *is* callable; the run then silently degraded to faux and
  the banner named no provider. `resolve_model_for_run` prefers a callable sibling, reports the
  substitution, and when nothing can run it names the provider, its env var, the login command
  and which providers do have credentials. Exact catalog ids are still honoured verbatim — an
  explicit choice is never overridden.
- **ADR-008 amendment — descriptor order is sorted, not hash order.** The tool list feeding
  the STABLE prompt tier and every provider `tools` array came out of a `HashMap`, whose
  iteration order is seeded per process. Two runs of one session therefore sent byte-different
  prompts and no provider prompt cache could hit across invocations — the tier discipline's
  stated premise, silently false in practice. Sorting by name makes the bytes a function of the
  tool set alone.
- **ADR-005 amendment — child processes are scrubbed, and output retention keeps both ends.**
  Every spawned child now drops credential-shaped environment variables (DSH's subprocess
  rule): confinement bounds where a process may go, not what it may read, and a tool result is
  durable (context, session file, exports). Output truncation keeps head *and* tail with exact
  omission counts, because tail-only truncation deletes the first compiler error; spill files
  are created exclusively under a `0700` root and a failed spill says so instead of promising a
  locator that does not exist.
- **ADR-010 amendment — partial tool output is streamed.** `Sandbox::run` accepts an optional
  output sink; both backends drain pipes incrementally and the shell tool throttles them into
  `ToolExecutionUpdate` events (250 ms, 4 KB tail). The complete output still returns with the
  tool result — the stream is a view over a running command, never its record.

- **ADR-010 amendment (M67) — stuck runs are detected by result, not by call.** The doom-loop
  brake counts identical *calls*, which is the wrong signal: a stuck model varies its arguments
  (`rg`, then `grep`, then `find`) and gets told the same thing every time. `guards::ProgressGuard`
  signatures each tool result — error-ness plus a normalized head of the text, so whitespace, case
  and a differing tail are not mistaken for progress — and counts consecutive identical results.
  At `no_progress_threshold` (default 3) the guidance is appended to the offending result, which is
  the cheapest place to reach the model; at twice that, the wording escalates and names the way
  out; and before the next batch a stalled run raises a `no_progress` permission request on the
  doom-loop brake's own surface, asked once per batch rather than once per call. The harness never
  rewrites a successful result into an error to get attention, and `max_turns` remains the halt.
  In the same amendment: a run may no longer end on an **empty turn** — a finished turn with no
  text and no calls, or one line dominating a long reply, gets one nudge message (delivered as a
  normal message, so it is in the transcript) and then ends with an error naming the cause rather
  than a false `Completed`. `no_progress_threshold: 0` and `empty_turn_retries: 0` disable each
  guard; a zero threshold never fires, because a misconfigured guard that blocks everything is
  worse than no guard.
- **ADR-010 amendment (M68) — the harness records whether edits were verified, and says so once.**
  The agent contract asks the model to run the project's checks after editing; nothing observed
  whether it did, so "the tests pass" was unverifiable from outside. `epsilon-core/src/verify.rs`
  logs every successful `write`/`edit` with the turn it happened in and every check-like `bash`
  invocation with its exit status (a known runner plus a check sub-command, a program that *is* the
  check, or either inside a compound command — `grep -r test .` and `cargo fmt` do not count). A
  file is unverified until a **passing** check has run at or after its last change: a failing check
  changes the wording, never the verdict. At the stop point the run receives one ordinary message
  naming the files and what has not run, capped by `verification_nudges` (default 2), plus a
  `Notice` event for frontends. The run is never refused — the harness does not run builds the user
  did not ask for, and it does not need a policy for projects with no checks at all. Detection is
  best-effort and says so; the precise version reads the project's declared commands (Hermes
  `agent/verify/recipes.py`). Scope is one agent: a `task` child's edits are the child's record.
- **ADR-008 amendment (M69) — the cache discipline is measured, and its failures are named.**
  The tier discipline's stated purpose is economic, and until now the only evidence was a hit
  percentage that could not say why it fell. `epsilon-core/src/cache_stats.rs` classifies every
  provider call (cold, served, or re-billed) and attributes each miss to a model switch, a
  `prefix` that moved, an idle gap past the TTL, or — when the harness can see none of those — the
  provider re-reading a prefix it was given unchanged. `prompt::stable_fingerprint` hashes the
  STABLE tier alone: the CONTEXT and VOLATILE tiers change by design, and a fingerprint that
  covered them would report a cache break every time the date rolled over. Re-billed tokens are
  priced at the model's catalog input rate, which is exactly what those tokens cost; cache reads
  and writes are reported as tokens rather than dollars, because their rates are provider-specific
  multipliers this crate would otherwise have to invent. A wire that reports no cache traffic is
  described as silent rather than scored as 0 % hits.
- **ADR-005 amendment (M66) — content from outside the session is labelled as such.** A tool
  result is not trustworthy because a tool produced it: `webfetch`, every MCP server and every
  extension return text written by someone else, and it used to arrive at the same level as the
  user's own instructions with only a prose request not to obey it. Results from those sources are
  now wrapped in `<untrusted_tool_result tool="…">`, with every case-variant of the delimiter
  neutralized first so the content cannot close the boundary early or open a fake one; results
  under 32 bytes are left alone, and local tools are never wrapped — workspace content is the
  user's own and arrives byte-for-byte. A provider truncation marker (`has_more`, `… more items`,
  `data_preview`) in a long untrusted payload adds an incompleteness note, because a truncated
  result reads as complete and would otherwise be reported as the whole dataset. Shaping happens
  once, at message construction (prompt caching stays stable), and the session file holds the
  shaped form — what the model actually read.
- **ADR-007 amendment (M66) — the user's own turns are never paraphrased.** The summarizer
  serialized the whole transcript, so after a compaction the model's paraphrase of "do NOT touch
  the parser" was the only surviving record of it. User turns from the summarized range now ride
  into the new context verbatim, in a `<user_instructions>` block inside the summary message, and
  the summarization request is told to quote rather than restate. The cap (4 KB) is applied
  newest-first so it drops the oldest instructions rather than the most recent; a previous
  summary's own carried block is recovered and merged, so instructions survive successive
  compactions instead of evaporating on the second one. Companion to M64's `<file_ledger>`: the
  summary message now holds the model's prose followed by the harness's facts.
- **ADR-006 amendment (F10, 2026-09-17) — plan mode composes layers and refuses shell.**
  Plan mode layers project `[[rules]]` UNDER the plan ruleset instead of replacing them:
  project denies survive a research run, and the engine's monotonic deny keeps plan
  boundaries intact no matter what any later layer allows. `plan_ruleset()` denies
  `bash`/`shell` outright rather than matching subcommand patterns (`git commit*`), because
  core preflight authorizes only the command head — string prefixes cannot establish a
  read-only shell boundary, so plan mode offers no shell execution at all.

- **ADR-005 amendment (M81 / F02) — observations belong to an agent/session view.**
  Supersedes M65's shared-read rationale below. Each Agent owns an allocation-backed
  `Arc<()>` token carried in ToolContext. Context clones retain that token; new agents
  (including task children) and restored history receive fresh tokens. The string session
  label is not an observation identity. Tools retain only weak token ownership and reclaim
  retired scopes on the next log access. A successful native write/edit invalidates sibling
  observations for the same lexical path, even when mtime/length match; only the writer's
  snapshot is refreshed. Failed mutations do not refresh it. The per-path mutation queue
  remains shared. No stateful behavior is added to contracts: it carries the passive token.
  This does not close alias identity (F09), external-writer races, or read/stat races.
  Host callers must retain a token across their own tool calls, not allocate one per call.

- **ADR-005 amendment (M65, observer sharing superseded by M81) — a mutation is verified against what was read.** Permission gates
  decide *whether* a call may run and confinement decides *where* it may reach; neither notices
  that the file changed between the model's read and its write, which is how a `task` child's
  work (or the user's own editor) gets silently replaced. `epsilon-tools/src/read_log.rs` keeps
  per-path read stamps — mtime, size, whether the view was partial and whether that partialness
  was the harness's own byte cap — and `write`/`edit` verify against them inside the per-path
  mutation queue. Refused: overwriting an existing file nobody read (Hermes' write-without-read;
  DSH's `FS_NOT_OBSERVED`), writing or editing a file that moved since the last read, and
  overwriting wholesale a file that was only read in part. Allowed: creating a file, writing one
  that was read whole and has not moved, any targeted `edit`, and any file whose view was cut by
  a cap no re-read can lift — a guard that cannot be satisfied is a trap. The registry is
  process-global (the `mutation.rs` precedent), so a child's read and the parent's write are
  checked against each other; diagnoses therefore say *"since it was last read"*, never *"since
  you read it"*. It is a guard, not a lock: an external writer can still win the race between
  check and write, and the module says so.

- **ADR-005 amendment (M95) — a page is not a file: reads carry coverage, and the cap exemption narrows to the one window no read can widen.** Teaching `read` pi's offset/limit paging (`read.ts:275-323`) falsified the M65 exemption's premise outright — "no further read can widen it" is true of nothing except a single line longer than the byte budget — so the same change replaced the read stamp's `partial`/`capped` booleans with a coverage model; the over-cap exemption lives on in the coverage record's `capped` field, which `read_log::verify` consults (it was never a `Stale` variant — `Stale` has the same three cases, `Modified`/`PartialRead`/`NeverRead`, before and after). `epsilon-tools/src/util.rs::read_page` streams the file (one line plus the page resident, never the whole file), stops only on whole lines — which is what removes the old prefix cut's U+FFFD injection — and reports the file's real line count, while `read_log::Coverage` accumulates the lines covered *contiguously from line 1* across pages: a write may replace a file only when the reads reached its end (or the harness wrote it), and a gap, an out-of-order page, or a page that stopped early all leave it partial. The exemption survives only for pi's `firstLineExceedsLimit` case (a line over the budget, answered with a `sed` fallback), because no read through this tool can ever show it and a guard that cannot be satisfied is a trap. A snapshot's coverage is discarded when the file's stat moved or a sibling wrote it: the old lines described other bytes. Consequences recorded: the guard's diagnosis is unchanged, `grep`'s byte cap is now announced in its result instead of being a silent miss, and the read tool's byte budget stays epsilon's own (256 KiB per page) where pi uses 50 KiB.


- **ADR-005 amendment (M135) — an `edit` is not a `write`: coverage is carried across a targeted replacement, never promoted.** The M95 amendment made the read stamp a *coverage* model, and the rule it states is "a write may replace a file only when the reads reached its end (**or the harness wrote it**)". The parenthetical was true of `write` and quietly untrue of `edit`, because both tools recorded their success through the same `read_log::record_write`, which *sets* `complete`. So the F01 refusal could be defeated in two calls: read page 1 of a 5,500-line file, `edit` one line of it, then `write` the whole file — the licence the page never earned, handed over by a tool that was only ever allowed to change lines the model had already seen. M95 recorded the hole in its `Limits` as pre-existing and nobody closed it. The fix is a second recorder, `read_log::record_edit`, whose contract is the difference between the two tools: **`write` supplies every byte of the new content, so the file is the model's own text end to end; an `edit` supplies a fragment, so what the model has seen is what it had seen.** `record_edit` therefore refreshes the stat (so the model's own change is not later refused as somebody else's) and carries `complete`, `capped` and `read` forward while discarding the line arithmetic, because an edit can add or remove lines and a high-water mark counted in the old numbering can only over-claim coverage of the new content. Rebuilding coverage from line 1 is the same conservative direction the module already takes for an out-of-order page. A file the model only ever *edited* and never read keeps the `NeverRead` refusal rather than being softened to `PartialRead`, which is why `Snapshot` grew a `read` flag: the two sentences mean different things to a model deciding whether to read the file again. No `Stale` variant was added — `Modified`/`PartialRead`/`NeverRead` still covers every refusal — and the three diagnosis *strings* are unchanged, though not every case lands on the same one: a file the model only ever edited is now refused `NeverRead` where it was `PartialRead`. Two limits are recorded because the guarantee is narrower than the amendment's first sentence suggests. `capped` is carried forward because a guard that cannot be satisfied is a trap, and the honest price of that is that an over-cap read shows the model **no lines at all** while still licensing a whole-file overwrite — pre-existing, now pinned by `the_capped_exemption_is_a_hole_the_read_tool_cannot_cloze` rather than left unmentioned. And this guard covers the **file tools**: `bash` runs the platform shell and consults nothing here, so the F01 protection is "a model may not overwrite unseen text through `read`/`write`/`edit`", not a claim about the harness as a whole.

- **ADR-014 amendment (M79) — `epsilon-lsp` is the twelfth crate; the brick earns existence.**
  Roadmap Phase 5 item 1 asks for editor-grade feedback: language-server diagnostics on the file an edit
  just touched, which is the largest correctness lever the harness lacks. It lands as its own crate
  rather than a module inside `epsilon-tools` because it is a *client of an external protocol with its own
  process lifecycle* — spawn, handshake, keyed-by-(root, server) reuse, an in-flight spawn dedupe and a
  `broken` set — which is the same standing `epsilon-mcp` already has as a client brick, and the same reason
  it was not folded into core. It sits in the leaf tier beside `safety` (`contracts <- lsp <- tools`),
  depends only on `epsilon-contracts` and tokio/serde, and adds **no new dependency**: the JSON-RPC framing
  is thirty lines over `tokio::io` (MCP's framing belongs to rmcp, so it is mirrored with a citation rather
  than reached across a crate boundary). Tools consume it through a `DiagnosticsProvider` trait in
  `epsilon-contracts`, so every tool test runs against a fake and no test needs a language server installed.
  Two deliberate restrictions: **no auto-install paths** (a coding agent that downloads binaries is a supply
  chain), and **silent degradation** (a missing server is not an error — the edit succeeds and the harness
  says nothing, because a harness that fails an edit for want of a tool is worse than one that cannot lint).
- **ADR-014 amendment (M215) — the tool crate stops linking a language server; the policy moves to the
  frontend (A8, plan 023).** The M79 amendment above writes the direction as `contracts <- lsp <- tools`,
  and the last edge is what A8 objected to: `epsilon-tools` declared `epsilon-lsp` for one match arm in
  which the **tool registry constructed a `DiagnosticsService`**, so every consumer of the tool crate —
  its tests, and any frontend that later wants tools without diagnostics — linked a language-server
  client it had no use for. The tools never needed the crate, only the seam: `DiagnosticsProvider`
  lives in `epsilon-contracts`, and `write` and `edit` already took an
  `Option<Arc<dyn DiagnosticsProvider>>` through `with_diagnostics`. The edge is therefore removed by
  the inversion this repository now uses three times — `ToolContext::task_host`,
  `ToolContext::goal_host`, and this: **the frontend owns the policy because it owns the flag** (`Lsp`
  lives in `epsilon_cli::runtime` beside the `--lsp` parse, and `diagnostics_for(policy)` is the single
  place that turns "off/auto" into a handle and attaches `epsilon_safety::scrub_secret_env` to it),
  **and the library receives the handle**. `register_builtins_with` takes
  `Option<Arc<dyn DiagnosticsProvider>>`. No new crate, no new edge, no new trait in contracts, and no
  change to when diagnostics run: `cargo tree -p epsilon-tools -i epsilon-lsp` now reports no such
  package in that graph, and `epsilon-cli` is the only dependent left. That the behaviour is unchanged
  is what the suite size says exactly — 1,634 tests across 65 binaries, one more than M214 and no other
  difference.
- **ADR-005 amendment (M78) — the mode is per-call truth, and widening is consent.** Three rungs
  (`read-only` / `workspace-write` / `danger-full-access`), resolved once per call from the call's own
  paired arguments and the session's mode, and checked at execution time rather than declared in a
  schema — a schema is registry-global while the effective mode is per-call truth. Narrowing never needs
  approval (it buys a stronger guarantee); widening is refused with the reason and the retry hint, and
  `danger-full-access` is reachable only through the session switch a human flips. A backend handed
  "no confinement" refuses instead of running with its own limits, because "no boundary" is not a
  boundary. The approval *ask* for a widening (DSH's `approveEscalation`) is deliberately not wired yet:
  a per-call grant must come from a harness-owned channel (the model's arguments are forgeable), which
  means `ToolContext` carrying a mode type out of `epsilon-contracts` — its own ADR.
- **ADR-005 amendment (M77) — confinement reports its own completeness.** A confined outcome
  now says how completely the boundary held, and each backend declares its envelope
  (`Sandbox::capabilities()`): Windows job objects confine resources and not content, bubblewrap
  confines namespaces and not resources, the null backend confines nothing. The value type makes
  the unanswerable state unrepresentable (`from_missing(vec![])` is `Full`), a refusal stays an
  error rather than an outcome *so that* the third question — did the sandbox refuse? — is
  answered by provenance instead of by pattern-matching output text, and `epsilon doctor` prints
  the backend's claim rather than restating it. The rule this encodes: a diagnostic may not say
  more than the mechanism can support, and that includes the audit's own prose.
- **ADR-011 amendment (M76) — diagnosis is a mode, not a run.** `epsilon doctor` answers
  "why is this not working" with one report: the resolved model and how it was resolved, the
  credential's source (never its value), the sandbox backend the platform provides, the session
  file and its lease holder, project instructions and trust, the tool list, and the newest
  session's last failures — exiting non-zero when a blocker exists and naming the fixing command
  for each. Two decisions make it trustworthy rather than decorative: it runs **before the
  runtime is built** (like `auth`), because a diagnosis that needs the machinery cannot diagnose
  the machinery — it also takes no session lease and prints no startup banner over itself; and
  every line comes from the same API the run path uses (the launch's own model precedence, the
  real `builtin_registry`, the real trust lookup), so the report cannot disagree with what a run
  will do. Where the harness cannot know something it prints `unknown` — a lock file's liveness,
  an unresolvable id — rather than a plausible guess.
- **ADR-006 amendment (M75) — a session line that cannot be read says *why*, and a repair
  says which kind it was.** The store reported every unusable line as `corrupt session line #5`,
  which merged two unrelated situations: real damage, and a line written by a newer Epsilon
  carrying an entry kind this build has never seen. The second is version skew, and calling it
  corruption sends a user looking for a fault that is not there — and makes deleting a healthy
  session look like the fix. `LineDiagnosis::{NotJson, UnknownEntryType, SecondHeader,
  LegacyInCurrentFile}` now drives `StoreError::Corrupt { line, diagnosis }`, whose Display names
  the cause and the consequence (the unknown type is quoted). Refusal behaviour is unchanged —
  the file is left byte-identical, and a torn *last* line is still tolerated, because a partial
  write is expected while mid-file damage is not; that distinction is the point of the change.
  Repairs report kinds (`RepairKind::{DanglingToolCall, MissingTurnEnd}` — the two shapes this
  harness can produce; pi's twelve describe a reducer with more states, and a reason that cannot
  occur is a claim nobody can test), the synthesized entry carries `details.repair`, and the
  Agent keeps the resume's report so `/session` can list what the session was missing.
- **ADR-008 amendment (M73) — the CONTEXT tier learns the directories a run works in.** Only
  the workspace root's `AGENTS.md`/`CLAUDE.md` reached the model, so in a monorepo the model
  working in `crates/x/` guessed at conventions written down one directory away.
  `Agent::note_touched` records each validated call's directory (reusing M72's declared resource,
  so nothing is re-parsed and the planner's view cannot disagree with where the call went) and
  `prompt::load_subdirectory_hints` loads that family from the touched directories and their
  ancestors as ordinary `<project_instructions>` blocks — same tier, same hash tags, so the
  cache story is unchanged in kind. The bounds are part of the decision: at most five levels up
  (a hint governs its own subtree; further up is somebody else's project), the root excluded
  (already loaded, and repeating it would double every prompt), content-deduped (a vendored copy
  is one block), at most five files nearest-first (the closest directory governs what is being
  edited) and 4 KB each with the cut announced. Hints appear only after the work reaches the
  directory, which costs one prefix re-read per discovered directory and is asserted to be
  once-per-directory rather than per-turn.
- **ADR-008 amendment (M101) — the STABLE tier gains a user identity and an append, and the
  context-file rule becomes one file per directory.** `.epsilon/SYSTEM.md` and
  `.epsilon/APPEND_SYSTEM.md` were trust-gated and never loaded, which is the worst of both
  outcomes: the prompt implied the file took effect. `prompt::SystemOverlay` now carries both
  (`load_system_overlay{,_in}`, mirroring pi's `discoverSystemPromptFile` /
  `discoverAppendSystemPromptFile`, `resource-loader.ts:1023-1049`); the project pair shadows the
  global pair rather than adding to it, and a blank file counts as absent, both matching pi.
  `Stable` gains `custom_identity` and `append`, so the byte-stability fingerprint covers them and
  a change to either invalidates the cache prefix exactly as a context-file change does. **One
  deliberate divergence, stated because a later reader would otherwise "fix" it back:** a custom
  identity replaces the *contract* only. The generated `# Available tools` and `# Guidelines`
  sections stay, because they describe the live registry and a hand-written tool list that has
  drifted is the harness lying about itself. pi drops its own list with the default prompt
  (`system-prompt.ts:48-73`); we keep ours on purpose. Separately, `CONTEXT_FILE_CANDIDATES`
  becomes pi's five names with **first-existing-wins per directory**: the previous loop loaded
  every candidate it matched, so merely adding `AGENTS.override.md` to the list would have made
  the override *additive* and it would have stopped overriding. Ancestor directories up to the
  filesystem root are walked and reversed, so ordering is outermost-first and a nearer file still
  wins; ancestor blocks are labelled by absolute path, since a relative `../AGENTS.md` per level
  names no usable location. `.epsilon/prompts/` stays gated and unloaded — pi's `prompts/*.md` are
  user-invoked slash templates with `$1`/`$ARGUMENTS` substitution
  (`prompt-templates.ts:11-18,70-80`), not system-prompt text, and the template feature is
  unported.
- **ADR-002 amendment (M72) — the barrier is per-resource, not per-mode.** The pool's rule
  was all-or-nothing: any `Sequential` tool made the whole batch a barrier, which was safe and
  expensive — `write` and `edit` are `Sequential`, so three edits to three different files
  serialized. Tools now declare the resource they act on (`Tool::resource_key`, resolved against
  the same cwd execution receives, so planner and execution agree by construction) and
  `exec::plan_batch` groups a batch by conflict. Three decisions carry the safety: paths that are
  prefixed by one another are one resource (a `grep` over `src/` must not run beside a `write` to
  `src/lib.rs`); two readers never conflict, which keeps workspace-wide searches concurrent
  instead of silently serialized by their own shared key; and one resource means one chain in
  call order, built by union-find rather than greedy bin-packing (greedy would run a third
  same-file call beside the first). A `Sequential` tool that declares no resource — `bash`,
  `task` — keeps the old barrier, because its reach is unknown. The mode supplies the read/write
  role rather than a second vocabulary that could disagree with it.
- **ADR-002 amendment (M84) — find declares directory reach.** `find` reuses the
  search tools' directory resource key so the existing planner orders it against
  descendant writes while allowing reader concurrency. This remains lexical
  scheduling, not resolved-path containment or F09 closure.
- **ADR-004 amendment (M71) — the edit matcher's loose end is guarded, and the guard has a
  floor.** `epsilon-tools/src/edit_match.rs` grew from three stages to six (escape-normalized,
  trimmed-boundary, block-anchor) and now refuses the one failure mode a loose stage can
  produce: a match far larger than the text the model asked to replace. The added stages are
  ordered by rising risk, not by observed frequency as opencode orders them — when two stages
  would both match, the safer one wins. The block-anchor stage is the only variable-height
  stage, so it is the only one the refusal can fire on: a fixed-width window is exactly as tall
  as `old_string` by construction, which a test asserts as a property rather than by example.
  Refusals follow opencode's rule (`max(old+3, old*2)` lines or +500 characters) rather than a
  share-of-file ceiling: a 40-line block in a 100-line file is a legitimate edit, and a
  percentage rule would refuse it. `new_string` is still applied verbatim — the loose stages
  locate a region, they do not restyle the model's text — and every loose match says which
  stage matched, so the model can verify against the diff the tool returns.

- **ADR-007 amendment (M64) — the pressure trigger is measured, and the cut is legal.** The
  trigger read one `chars / 4` estimate over the whole transcript, which is a *guess* where the
  provider had already sent the answer: every response reports the true prompt size. The loop
  now anchors to that number with the message count it covered and estimates only the messages
  appended since, dropping the anchor the moment the history it describes is rewritten
  (compaction, session switch — `compact::anchored_tokens`). Separately, the retained tail is
  cut at a turn boundary, never inside a tool batch: a tail starting at a tool result has lost
  its call to the summary, and strict wires reject a request shaped that way — a failure that
  would land *after* paying for the summary call. Forward to the next boundary when one exists,
  backward when the batch runs to the end of the transcript. Both were port candidates from the
  reference mining (`audit/references/pi-portable-parts.md`, DSH `tool-pairing.ts`).

- **ADR-011 amendment (M201) — a status row's parts are whole or absent, and the header's tail is what yields.** Two rows state the harness's own operation and both were fitting themselves by cutting a *character*: at 60 columns the header printed `idle 0 ↓0`, a bare `0` where `↑0 ↓0` had been — which reads as a value, not as the loss of a label — and the footer printed `pagedown scroll forward · ct`, with `ctrl+enter` cut into a key that does not exist and abutting the working directory. Each row now budgets its own half with `layout::room_beside` (the same subtraction and the same one-column gap `side_by_side` reserves) and emits whole parts only, so what the layout does on a correct row is a backstop for a width-table disagreement rather than the mechanism that decides what a narrow terminal keeps. Which parts go is the order of importance in `Metrics::parts`: the tail yields, and the model with its `offline` qualifier is pinned — as a **field on `MetricPart`**, not as an inference from `MetricRole::Model`, because an offline session draws the model with `Warn` and a role-keyed pin stopped matching in exactly the row long enough to need it (measured: `offline` dropped, the model next, `R0 W0 CH n/a` kept). `side_by_side` lost its `whole_span` parameter and `cut_keeping_whole` with it: the position badge is emitted only when it fits, which is the same guarantee reached one step earlier and without a post-hoc cut. The footer's rule is three-valued — `key action`, `key`, or nothing — because a bare key is a real key honestly named while `ct` is a key that does not exist; a fragment is worse than an absence, since a wrong value is not noticed the way a missing one is.
- **ADR-011 amendment (M201) — one surface owns the session facts, and the first-run screen says what to do.** The welcome panel printed `model`/`context` two rows below a header already printing both, on a screen with twenty blank rows under them, at three different left edges and with the instruction styled exactly like the data above it. The header owns the model and the context, because it states them in *every* state while the panel exists only before the first message; the panel now spends its rows on the two things nothing else says — what to type, and the three prefixes the composer reads (`@`, `!`, `/`) — on one left edge, with the instruction in a body weight so it reads as a call to action rather than as another datum. The composer's placeholder wears the prompt marker (`❯`) instead of the notice bullet (`•`), which had the one glyph meaning "the system said this" marking the line the user was about to type, on every frame of every session.
- **ADR-006 amendment (M202) — a field added to a durable session type must be readable when it is absent, and a refusal quotes the decoder.** `Usage::cache_reported` was added in M194 with no serde default; `Usage` is inside every assistant message, so *every* session file written before that milestone stopped decoding, and the store's own (correct) rule — refuse a known entry kind whose body will not read — turned that into a harness that would not start in a workspace containing one. Measured, not imagined: the installed binary refused to start in this repository's own directory, and no test in the tree could see it because every fixture was built with the current shape. The rule: a field *added* to a durable type carries `#[serde(default)]` with a value that means "the writer did not report this" (`false` for `cache_reported`, which is also what an unannotated decoder produces and what the header renders as `CH n/a`); a *pre-existing* field that is missing stays a hard error, because that is damage rather than evolution. The two are told apart by the decoder's own sentence, which is why `LineDiagnosis::MalformedEntry` now carries `detail` and why the diagnosis re-decodes the line through the tagged `SessionEntry` parser: `SessionLine` is untagged by necessity — a session file mixes headers, entries and legacy shapes — and serde reports only the outermost failure for an untagged enum, so the first version of this improvement printed `data did not match any variant of untagged enum SessionLine` and named nothing. The same change makes the refusal name the **file**: a line number inside a file the reader was never told the name of is not a location.
- **ADR-006 amendment (M203) — a goal is a session-log fact, and its activation is not durable.** One long-running objective per session, ported from DSH's goal domain: every mutation appends a complete post-mutation snapshot (or a revisioned tombstone) as a new `EntryKind::Goal` entry, and the current goal is a fold over those entries in file order — the same shape `labels()` already had, and for the same reason (the log is the only authority, so a restart, a fork and a resume need no migration and "what was the objective" has one answer). Three of DSH's rules are kept verbatim because each one is a failure the naive version has: **compare-and-set** on `{id, revision}`, so a model tool call that read the goal ten calls ago cannot overwrite a change the user made in between (a `/goal` command carries no ref, because the user is looking at the goal); **process-local activation**, so a durable `active` goal does not arm a process that has not asked — the permission to work unattended must not survive a restart, since the person who granted it is not necessarily the person who opens the session next; and **one current goal**, replaceable only when the current one is terminal, which is what stops `/goal` being a command a user has to undo before it works again. Two rules are relaxed deliberately and recorded: a `Set` whose revision does not follow the previous one is applied rather than refused (refusing to derive state because a number skipped turns a writer's bug into an unreadable view, and the compare-and-set still guards every mutation), and the goal is not scoped to the active branch (navigating a branch rewinds a conversation, not an objective). `GoalError::NoStore` is Epsilon's own: DSH's goal service always has a session log, and this harness has frontends without one, where "no goal is set" would tell a user their objective is unset when nothing they type could ever set it.
- **ADR-008 amendment (M203) — the objective rides the CONTEXT tier, and no goal changes a byte.** A set goal is rendered as `<session_goal>` at the *end* of the CONTEXT tier: it changes when someone changes the goal, which is rare, so it belongs in the tier whose contract is "stable while the thing it describes is unchanged" rather than in the volatile tier, where it would move the byte boundary a provider's prompt cache keys on for a value that did not. Only the objective's words are rendered — no id, no revision, no round counter, because a number that moves every round would rewrite that prefix every round; the model has `get_goal` for those. And because the block is emitted only when a goal exists, a session without one produces a prompt byte-identical to what every earlier build produced: the STABLE fingerprint is untouched, so this feature cannot cost a cached prefix.
- **ADR-011 amendment (M204) — the frontend owns the round driver, and five conditions decide.** Continuing a goal between turns is a *frontend* behaviour, not a loop change: the TUI watches its own turn ends and calls `Agent::goal_begin_round`, which is the same division DSH draws between its goal domain and `goal-round-driver`. The loop (ADR-002) is untouched. Five conditions, each one a way the harness could otherwise do something the person in front of it did not ask for: the goal is continuable — **re-read from the log at the decision**, so a `complete` from a tool during the turn stops the loop on the same frame; nothing is in flight and no permission ask is parked; **the composer is empty**, because a user mid-sentence has not asked for another round and starting one would race their Enter (the round is held, not lost); and the run produced at least one message, because a provider failure produces none and repeating it would spend the whole budget on a loop that cannot progress. Two rules keep the driver from fighting the user: **an abort disarms** (Esc withdraws the permission to continue, leaving the durable goal intact — and the disarm hangs on the *request*, not on whether a cancel token happened to be live, which is both the honest rule and what makes the branch testable through the real key handler), and **creating or resuming a goal starts its first round**, because a goal that parked an objective and sat there is a note rather than work. The continuation is one ordinary user-role message: it is logged and echoed like a typed turn, and deliberately *not* put through the composer's submit path, which would offer it back through Up-arrow and write it to the global prompt-history file.
- **ADR-012 amendment (2026-09-29, decision only; plan 019) — one reader of `config.toml`, and parsed values cross the seam.** A12 measured the same rule written twice in crates that cannot see each other: `global_dir()` exists byte-for-byte identically in `epsilon-core/src/config.rs:37` and `epsilon-llm/src/auth.rs`, and three separate readers parse one global `config.toml` (`core::config` for `[tui]`/model/recents, `epsilon-llm::custom` for `[provider_custom]` — which is why `epsilon-llm` carries a `toml` dependency at all — and `oauth_login` for `[oauth.<provider>] client_id`). Two copies of one rule cannot disagree visibly, which is this repository's D-class: a path rule that changes in one place and not the other produces a config file one crate reads and another cannot find, with no error anywhere. **The obvious fix is blocked by law rather than by taste**: `epsilon-llm` may not depend on `epsilon-core` (the DAG runs core → llm), `epsilon-contracts` is types-and-traits-only with zero behaviour, and a new crate needs an ADR to hold one function. So the decision is the inversion this repository already uses twice — `register_builtins_with_skills` (the frontend discovers, the crate receives) and `ToolContext::task_host` (the crate asks, the owner answers): **`epsilon-core::config` is the only reader; everything a lower crate needs arrives as a value.** The credential store takes its directory as a parameter rather than resolving it; custom providers arrive as parsed rows, with `CustomProvider::from_config` (already pure) as the seam and `toml` leaving `epsilon-llm`'s manifest; OAuth client ids arrive as values; and the project-config parse moves next to the trust hash so "what the hash covers" and "what the settings are" cannot drift. No new crate, no behaviour in contracts, no config cache — a cache would add an invalidation question (`/reload` exists) that nobody has asked. Execution order, per-step evidence and limits: [`docs/plans/019-one-config-reader.md`](../plans/019-one-config-reader.md).- **ADR-D01 (deferred): WASM extension sandbox.** Revisit if process-isolation overhead ever
  matters or component model stabilizes further.
- **ADR-D02 (deferred): Windows ACL restricted-token confinement backend.** Post-v1; requires
  dedicated security review; job-object limits ship meanwhile.
- **ADR-D03 (rejected during design): adopt DSH surface-shadowing data structure literally.**
  Our compaction-entry-with-shadow-ranges achieves the same model-view rewrite over pi's simpler
  entry format; literal port would fork the session format for no user-visible gain.
