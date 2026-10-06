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

> **SUPERSEDED by the M17 amendment below — read that before this.**
>
> The `StreamFn` seam this ADR introduces is still the design and still the reason the
> amendment was cheap. **The `genai` transport underneath it is gone**: `genai` appears
> nowhere in `Cargo.lock`, and `epsilon-llm` speaks each provider's native HTTP/SSE wire
> directly. The title and the Decision below are the decision *as taken*, kept unedited
> because that is what an ADR log is for — but a reader who stops at line 93 has the
> wiring backwards, and this heading is one of the three files
> [`../START-HERE.md`](../START-HERE.md) tells a newcomer to read first.
>
> Verified 2026-10-04: `grep -c '^name = "genai"' Cargo.lock` → `0`.

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
design. **This amendment is what supersedes the ADR-003 heading and Decision above** — the
transport they name is no longer in the workspace. genai's concrete failure modes were
observed in practice and matched this ADR's
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

*(Amended M290: **fourteen** crates — `epsilon-runtime` (M233) and `epsilon-desktop` (M233) joined,
and the heading is now two milestones stale again. The count in the heading is deliberately not
edited, because a historical decision record that keeps being rewritten is no longer a record: the
current count is in [`../../README.md`](../../README.md) and in `AGENTS.md` §3, and this line is
the pointer to them.)*

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

## ADR-020: A native desktop IDE, rendered by egui, over the same agent

**Status: accepted; slice 1 landed (crate `epsilon-desktop`), wiring in progress.**

**The problem.** Epsilon has one terminal frontend, one headless mode and one HTTP/SSE
server. The product this project is becoming is a **desktop IDE**, and no part of this
repository could become one: a desktop app needs a window, and every window path in Rust
either embeds a web browser or means writing a renderer from scratch.

**The constraint that decides it: a real desktop application, not a web application.**
That rules out Electron, and rules out Tauri-with-an-HTML-frontend, and rules out DSH's
own route — its "desktop" is an Electron shell that spawns a Node child running the *same*
web profile and points a `BrowserWindow` at it over HTTP. We are taking DSH's *workflow*, not
its *carrier*.

> **Citation corrected 2026-10-03.** This paragraph originally cited
> `references/deepseek-harness-dsh-v0.2.0-rc.1/apps/desktop/src/main.ts:408,421`. That version
> is **not vendored** — `references/` holds only `deepseek-harness-dsh-v0.2.1-alpha.1` — so a
> reader could not open the line. The claim is unchanged; the evidence now points at the tree
> that is actually here:
> `references/deepseek-harness-dsh-v0.2.1-alpha.1/apps/desktop/src/host-process.ts:189`
> (the shell spawns a Node child running the Desktop Host),
> `apps/desktop-host/src/index.ts:103-104` (that child serves on `http://127.0.0.1:<port>` and
> reports the URL over IPC), `apps/desktop/src/main.ts:450-452` (the shell keeps it as
> `hostUrl`), and `apps/desktop/src/main.ts:399,431,662-677` (the window loads a
> `dsh-app://app/` privileged scheme whose requests are forwarded over HTTP to the child's
> loopback URL). DSH states the arrangement itself at `docs/architecture.md:53-55`. Note it is
> slightly *more* elaborate than "points a `BrowserWindow` at it over HTTP": the window loads a
> custom scheme and forwards requests. It is still a browser window over a local web profile.

**Decision: `egui` + `eframe` 0.36.2, rendering natively through winit + wgpu.**

Measured against the five serious Rust GUI toolkits before choosing:

| | Native (no webview) | Licence | Accessibility | Build-host deps | Verdict |
|---|---|---|---|---|---|
| **egui / eframe 0.36.2** | winit + wgpu | MIT **or** Apache-2.0 | **AccessKit in default features** | none — no `bindgen`/`clang` | **chosen** |
| gpui (Zed's) | yes (Blade/D3D12) | Apache-2.0 | **none in the published crate** | `bindgen`, `cbindgen`, `embed-resource` | second best; see below |
| iced 0.14 | winit + wgpu | MIT | **none** | none | last release 2025-12-07; master mid-rewrite, unreleased; ~96% one maintainer |
| Dioxus desktop | **no** — `wry` + `webview2-com` | Apache-2.0/MIT | — | — | violates the constraint outright |
| Slint 1.18 | yes | **GPL-3.0 / royalty** | AccessKit present | **`skia-safe` + `bindgen` in every desktop config** | licence contamination *and* a Skia source build |

Four reasons, in order of weight:

1. **It is the only one that clears every bar at once.** Native rendering, a licence that does
   not change this workspace's, working screen-reader support, and — measured on the resolved
   dependency tree — **no `bindgen`, `clang-sys` or `embed-resource`**. That last one is not a
   footnote on this machine: every build failure recorded in `AGENTS.md` is an MSVC toolchain
   problem (`D8050: cannot execute c1.dll`, missing `vcvars`, missing NASM), and a toolkit that
   needs clang to build would add a fifth way to hit them.
2. **The objection that motivated the search is gone.** egui rasterised text with `ab_glyph`
   and no shaping through 0.33. At 0.34 it moved to `skrifa` + `vello_cpu`; at 0.35 it added
   `harfrust`, a full HarfBuzz port, for kerning and ligatures. Code and English Markdown —
   which is all this window shows — shape correctly today.
3. **Accessibility is not optional here and almost nothing else has it.** egui exposes roles and
   labels on its widgets and egui's `Harness` test API queries *the same AccessKit tree a screen
   reader consumes*. For a product that claims to be usable, that is the difference between a
   claim and a feature. It is also why the palette in [`theme.rs`](../../crates/epsilon-desktop/src/theme.rs)
   is written against a WCAG contrast floor with a test on it, rather than chosen by eye.
4. **The test story fits this repository.** egui renders into an off-screen `egui::Context`, so
   a frame can be painted in a unit test at any size — the same discipline the terminal frontend
   uses (`render_rows`) and the one `AGENTS.md` insists on after M198.

**Known costs, stated rather than discovered later.**

- **MSRV rises from 1.88 to 1.95.** egui 0.36 declares `rust-version = "1.95"`. The CI MSRV job
  (`.github/workflows/ci.yml`) is raised with it. This is a gate change and is called out as one;
  the alternative was pinning egui ≤ 0.33 and giving up text shaping.
- **No bidi.** `epaint` still carries `// TODO(emilk): heed bidi characters` and strips the
  directional control characters. HarfBuzz does the *shaping*; UAX #9 *reordering* is not
  implemented. Correct for code and for the English Markdown this window renders; wrong for an
  RTL codebase's comments or an RTL UI string. Revisit when a user asks, not before.
- **`wgpu` over `glow`.** `glutin`'s OpenGL path fails on some Windows configurations that DX12
  does not, and this project's primary target is Windows. DX12 is present on every machine we
  support; a working surface on a machine with no usable GL driver is worth the extra crates.
- **Immediate mode.** Every frame rebuilds the widget tree, so any list that can be large — the
  file tree, the transcript — must be virtualized by hand from the first version. A repository
  with a 10k-file workspace is not a hypothetical user.
- **Text is rasterized on the CPU** (`vello_cpu`) rather than from a GPU glyph atlas. Unmeasured
  on this machine. The transcript pane's throughput is a measurement to take before the
  transcript lands, not after.

**Why not gpui, which is the better product and the worse dependency.** gpui is Apache-2.0
(the GPL attaches to the Zed application, not to the crate) and it is the only proven *native
code-editor* stack in existence — cosmic-text, Blade on D3D12, and `gpui-component`'s ready-made
desktop components. It loses on three measurable counts: the published `gpui 0.2.2` is ~11 months
stale while Zed itself shipped v1.22.0, so adopting it means tracking an unreleased API; it needs
`bindgen` + `cbindgen` + `embed-resource` on the build host; and it hard-pins `cocoa = "=0.26.0"`,
which produced a real workspace resolution conflict against another desktop target when
measured. It also has **no accessibility** in the published crate. It wins if a11y becomes
optional and a vendored git dependency becomes acceptable. Both conditions are recorded here so
the next reader does not have to re-run the search.

**What this ADR does not decide.** The desktop's *content* — file tree, editor, diff view,
terminal, task panel — is a series of slices, each with its own evidence. And what it composes
is ADR-021, not this file.

---

## ADR-021: Composition is a library crate (`epsilon-runtime`), not a binary's private function

**Status: accepted; extraction in progress.**

**The problem.** Exactly one function in this repository knows how to turn a workspace plus a
configuration into a running agent: `build_runtime` (`crates/epsilon-cli/src/main.rs:1727`), a
`pub`-less `async fn` in a **binary** crate, together with 3,199 lines of helpers in
`crates/epsilon-cli/src/runtime.rs`. The terminal UI gets it, the headless modes get it, the
HTTP server gets it and the stdio RPC frontend gets it — because they all live in the same
binary.

A second frontend in a second crate cannot. It would have to reimplement the whole composition:
skill roots, extension roots and trust gating, the permission layers, the resume ordering, the
session-lease sweep, the migration and repair passes, the system-prompt overlay, agent
definitions, and the extension event forwarder. That is the definition of the "faithful port, no
production caller" defect in its most expensive form: two copies of a rule that cannot disagree
visibly, which is this repository's D-class (see ADR-012's amendment on `global_dir`).

**Decision: move composition into a new crate, `epsilon-runtime`.**

`crates/epsilon-cli/src/runtime.rs` becomes `crates/epsilon-runtime/src/lib.rs` essentially
verbatim, and `build_runtime` moves with it as a public function. `epsilon-cli` keeps `main.rs`,
argument parsing and mode dispatch; `epsilon-desktop` calls the same `build_runtime` the
terminal does.

**Why a new crate rather than depending on `epsilon-cli` as a library.** `epsilon-cli` *is* a
library — `crates/epsilon-cli/src/lib.rs` exports `doctor`, `rpc_mode` and `runtime` — so
`epsilon-desktop` could depend on it today with no refactoring at all. That is rejected for
three reasons, in order:

1. It drags `epsilon-tui` (ratatui, crossterm), `epsilon-server` (axum) and `epsilon-mcp`
   (rmcp) into a native GUI that wants none of them, on a machine whose build budget is already
   the binding constraint.
2. The architecture law reads "no library may depend on CLI". Depending on it from a *binary*
   does not break that law's letter, and this ADR does not pretend otherwise — but it inverts its
   intent, and a rule kept only in its letter is a rule the next agent will read past.
3. It makes the second frontend a *consumer of the CLI* rather than a peer of it. The TUI does
   not depend on the CLI either — it is composed by it. The desktop should have the same shape.

**Why this is the smallest change that works.** The extraction is mechanical: 35 references to
`epsilon_cli::` exist in the workspace, 33 of them inside `epsilon-cli` itself and 2 in its own
integration tests. `runtime.rs` has no dependency the CLI does not already have, and the new
crate's manifest is the CLI's minus the frontend crates. Nothing is redesigned in the move, which
is the point — a refactor that also changes behaviour cannot be reviewed for the refactor.

**What moves, and what stays.**

| Moves to `epsilon-runtime` | Stays in `epsilon-cli` |
|---|---|
| `runtime.rs` in full: trust store, skill/extension/agent roots, extension attach and hooks, project rules → `Ruleset`, `diagnostics_for`, session-path rules | `parse_args`, `dispatch`, `Mode` |
| `build_runtime` + `RunParams` | `doctor` diagnostics *presentation* (its inputs move) |
| `session_path_for`, `resumed_model_to_use`, the lease-wait notices | the binary, `--help`, the auth ceremony |

**The one thing this does not fix, named so it is not assumed fixed.** `dispatch` builds the
`StreamFn` — retrying ∘ panic-guard ∘ direct provider, or the `--fallback` chain — inline at
`crates/epsilon-cli/src/main.rs:1390-1440`, together with custom-provider installation and the
real-vs-faux decision. That is provider composition, and it is the second half of the same
problem: the desktop needs it too, and a second copy would silently disagree about which model
the window is about to call — which is exactly the lie M233 closed in the terminal header.
It moves in the same slice as `build_runtime`, not after it.

**Amendment (M234) — the second half moved, and the refusal path is why.** `resolve_launch` now
lives in `epsilon-runtime`, taking a `LaunchRequest` and returning a `Launch` or an `Err(String)`.
Four branches of the old block were **`die()` — a process exit** — and those are the four the
desktop could not have reached even by copying the function:

| Branch | Before | After |
|---|---|---|
| a model reference that matches nothing | `die("unknown model …")` | `Err("unknown model …")` |
| a reference matching several rows | `die("ambiguous model … — matches: …")` | `Err(…)` |
| `--provider real` with no credential anywhere | `die("--provider real requested …")` | `Err(…)` |
| `--script` that will not read or parse | `die("read …" / "invalid script json …")` | `Err(…)` |

That table is the argument, and it is not about tidiness: **a refusal path that exits the
process is a refusal path a window cannot have.** The desktop needs to show the sentence and
keep running. `Launch` deliberately does not derive `Debug` — `dyn StreamFn` is not `Debug`, and
a hand-written shim would print nothing a reader could use — which is why
`an_unknown_model_is_an_error_a_frontend_can_show_not_an_exit` uses `let … else` rather than
`expect_err`.

Three properties are now frozen by tests rather than by reading the branch:

- an unknown model is an error a frontend can display, and the sentence quotes what the user
  typed;
- a faux launch resolves with no provider and no key, and says so in its notices;
- **`a named real model beside `--provider faux` still reports `offline`** — the M233 lie,
  expressed as the field that prevents it.

`LaunchRequest::for_workspace` defaults to `Lsp::Off` and `Confinement::Auto`, which is the
library-caller's default rather than the CLI's (`--lsp auto` is what the CLI passes). The
difference is recorded because a default is a policy: a library caller that does not ask for
language-server diagnostics should not spawn one.

**`epsilon-server` stays as it is.** It is a real and useful surface (an external client over
HTTP/SSE) and it is not how the desktop talks to the agent. The desktop hosts the agent
**in process**, on its own tokio runtime, sharing one `Arc<Agent>`. Going over the wire would buy
nothing here and would cost the whole session surface, which the HTTP surface does not expose.

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

---

## ADR-022: The desktop's accessibility is egui `WidgetInfo` over painter-drawn panes, and the limit is stated rather than discovered later

**Status: accepted; implementation not started.**

**The problem.** ADR-020 chose egui because it "ships AccessKit in its default features so the
app is actually reachable by a screen reader". That claim is true of the *toolkit* and false of
*this crate*: `crates/epsilon-desktop` declares no accessibility dependency of its own, and its
transcript, diff, editor, goal band, title bar, status bar and palette rows are drawn with
`egui::Painter::text` on raw rectangles. **A painter call emits no accessibility node.** The
transcript — the single most important thing in the window — is invisible to a screen reader,
and no amount of toolkit capability fixes that from the outside.

**What egui actually offers, measured in its own source.** `Response::widget_info`
(`egui-0.36.2/src/response.rs:869`) takes a closure returning `WidgetInfo`, and egui emits that
into the AccessKit tree for whichever platform adapter is running. `WidgetInfo`
(`egui-0.36.2/src/data/output.rs:563-592`) is:

```
typ: WidgetType, enabled: bool, label: Option<String>,
current_text_value, prev_text_value, selected, value,
text_selection: Option<Range<CharIndex>>, hint_text: Option<String>
```

**The limit, stated up front so it is not discovered in a conformance audit.** There is **no
role, no description, and no relations field**. egui 0.36 exposes *name, value and state* — not
the full ARIA/AccessKit role tree. So the desktop can honestly reach **accessible name, value,
state and keyboard operation**, which is what most of WCAG needs, and it **cannot** express
landmark roles or `aria-describedby`-style relations through this API alone. Recording that here
means the next person does not re-derive it, and means "WCAG 2.2 AA" is not quietly claimed in
full.

**Decision.** Attach `widget_info` to the interact response of every region a user can see or
reach, using the smallest truthful `WidgetInfo` the region supports, and make the whole thing
**testable**: egui renders into an off-screen `egui::Context`, and its `Harness` test API queries
the same tree a screen reader consumes — so an accessibility claim can be asserted in a unit
test rather than believed. This follows the discipline ADR-020 already adopted for frames.

**Contrast floors, from the primary source.** WCAG 2.2 SC 1.4.11 (Non-text Contrast) requires
**3:1** for the visual information needed to identify UI components **and their states**, against
adjacent colours, and in combination with 2.4.7 Focus Visible the focus indicator itself must
have sufficient contrast against the adjacent background. The Understanding document is explicit
that 3:1 is a threshold that **must not be rounded** — 2.999:1 does not pass.
Source: <https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html>.

That has two consequences the current palette does not yet meet. `IdeTheme::SURFACE_FLOOR` is
**1.12:1** (`crates/epsilon-desktop/src/theme.rs:67`), and it governs `border_focus`,
`selection`, `selection_focus`, `panel` and the surface bands (`:266-274`). That number is not
careless — `theme.rs:248` argues it, and the argument is sound for its own case: *"a band is a
background, and a background is read as a boundary, not as text. 4% of luminance is where a fill
stops being the page."* A fill that merely tints a region does not need 3:1.

The gap is the case that reasoning does not cover, and it is the one 1.4.11 singles out. Where
the selection fill **or the focus border is the only thing identifying which row is selected or
focused**, it is not a background — it is the state indicator, and 1.4.11 requires 3:1 against
adjacent colours. Today neither `selection_focus` nor `border_focus` is ever compared **against
its own unfocused sibling**; the tests at `theme.rs:429-457` check each against the page
background at the band's floor, which cannot distinguish "is this row selected?" from "is this
row painted?".

**What this ADR does not decide.** Whether the desktop should also convert painter-drawn regions
to real widgets. That is the larger question and the more expensive one; `widget_info` over the
painter path is the smallest change that puts name, value and state in front of a screen reader
at all. It is the right first slice and it is not the last.

**How we will know.** Per `AGENTS.md` §8, the desktop is held to WCAG 2.2 AA. That claim is only
true if a test asserts it. A conformance claim with no test behind it is the exact failure mode
this repository has already paid for twice.

### Amendment, 2026-10-03 — implemented, and the tree is readable from a test

**The hedge above is no longer needed, and it was wrong to leave it there.** `Context::enable_accesskit()`
(`egui-0.36.2/src/context.rs:3701`) turns on the per-pass state that `Context::end_pass` turns into
`FullOutput::platform_output.accesskit_update` — the literal `accesskit::TreeUpdate` the platform
adapter forwards to the OS — and `Context::run_ui` returns it. `egui::accesskit` is re-exported
(`lib.rs:434`), so reading the tree in a test needs **no new dependency**. **An accessibility claim
in this crate can therefore be asserted, not asserted-by-proxy.**

One helper, `name_region` (`app.rs`), covers five regions through `Ui::interact` with a literal
id salt and `Sense::empty()` (so it cannot steal input from the scroll area or the editor), then
`widget_info`: the **transcript** (`Agent transcript · N entries`), the **editor** (read-only, with
the path and line count, or the change under review), and the **file tree**, **subagents** and
**sessions** panes with the counts they already had. Counts are read from the same values the
pane drew with; the transcript counts *entries*, not "messages", because a `RowKind` carries tool
calls and diffs too.

Two verifications, both measured:

- **Negative control.** With `widget_info` commented out and nothing else changed, **121 passed and
  8 failed** — exactly the eight new tests. Restored: 129 passed, 0 failed.
- **Nothing is drawn.** In the same control run, `FullOutput::shapes` was **47 entries both ways,
  byte-for-byte identical**. This is metadata, not repainting.

**Still not claimed, and said so in the code as well as here:**

- **The transcript's rows have no nodes.** The region is named and counted; a screen reader cannot
  read the *conversation*. That needs real widgets or per-row names, and it remains the largest
  open gap in this ADR.
- ~~No landmark roles, no descriptions, no relations — not expressible through `WidgetInfo` in
  egui 0.36. `WidgetType::Panel` maps to AccessKit `Role::Pane`, which is a container role and not
  an ARIA landmark.~~ **CORRECTED 2026-10-04 (M301).** True of `WidgetInfo`, false of egui:
  `Context::accesskit_node_builder` is **public** (`egui-0.36.2/src/context.rs:3684-3690`) and
  `accesskit::Node::set_role` is public, so the window now publishes real landmarks — `Log` for
  the transcript, `Navigation` for the activity bar, `Banner` for the title bar, `Status` for the
  status bar and the goal band, `TreeItem` for a file row, `Splitter` for the resize handle, and
  `Link` for a "file changed" row (M303). The
  one subtlety is **ordering**: `fill_accesskit_node_from_widget_info` rewrites the role on the
  same pass, so the role must be stamped **after** the `widget_info` call. **Descriptions and
  relations remain genuinely inexpressible** — `WidgetInfo` has no field for either.
- The nodes attach **flat** to the window root, because the pane `Ui`s carry no node of their
  own. Truthful, but not a pane hierarchy.
- **No screen reader was run.** The claim is "egui emitted the node into the platform tree", not
  "a screen reader announced it".

### Second amendment, 2026-10-03 — two corrections to the first, and a second mechanism

**Correction 1: two of the "painter-only" regions were not painter-only.** The first amendment
listed the **composer** and the **activity bar** among the regions that emit nothing. They were
wrong: both are real egui widgets, and egui had already put nodes in the tree for them —
*unnamed*.

- `WidgetInfo::text_edit` (`egui-0.36.2/src/data/output.rs:697`) fills `current_text_value` and
  `hint_text` and **never `label`**. The composer node existed as a `MultilineTextInput` with a
  value, a placeholder, and nothing to call it.
- `Button::ui` publishes `WidgetInfo::selected(Button, enabled, selected, text)` where `text` is
  the **glyph** (`widgets/button.rs:391`), so the three activity-bar buttons were named `◧`, `⑂`,
  `☰`. Separately: `on_hover_text` never reaches a screen reader at all — egui 0.36 contains no
  `set_description` or `set_tooltip` call anywhere, although `accesskit::Node` has both.

**Correction 2: naming a node egui already built needs a different call.**
`Response::widget_info` **writes** the node from the `WidgetInfo` handed to it, and on a frame
where the widget was clicked or edited it also pushes an `OutputEvent` (`response.rs:869-894`,
the `output_event` branch). Using it on the composer would queue a second, value-less copy of
every keystroke onto a queue the app had already received one of.

So there are now **two** accessibility mechanisms in this crate, deliberately:

| Helper | For | Mechanism |
|---|---|---|
| `name_region` | regions we paint ourselves | `Ui::interact(..., Sense::empty())` + `Response::widget_info` |
| `name_node` | nodes egui already built | `ctx.accesskit_node_builder(id, |n| n.set_label(..))` |

`name_node` writes **one field** and leaves role, value, placeholder and `toggled` exactly as
egui set them, which is what makes it safe next to `Button::selected`.

**Regions now named:** transcript, editor, file tree, subagents, sessions (the original five),
plus **title bar** (workspace, model and figures, character-for-character what it drew), **status
bar** (no agent attached / idle / running), **activity-bar strip** and its three buttons
(`files`, `subagents`, `sessions`), **palette rows** (salted by identity, not by index, so a node
does not renumber as the query is typed), **goal band** (status, round, gate count, armed — and
deliberately *not* the objective text, which is unbounded reader text), and the **composer**
(renamed, not doubled). Tabs were left alone: they are already `egui::Button`s, so egui names them.

**Evidence.** Six new tests through the real `paint` and the AccessKit tree, each asserting the
label **changes with the state it reports** (figures appear and disappear; `no agent attached` →
`idle`; `files` → `subagents`; `not armed` → `armed`; an absent goal publishes **no node at
all**), plus no-double-node controls for the composer and the palette's search box. Negative
control: all five new `name_region` calls and both `name_node` calls commented out and nothing
else changed → **130 passed, 6 failed**, the six being exactly the new tests, with five dead-code
warnings proving the removal was total. `FullOutput::shapes` identical in both builds.

**Still open.** The palette's overlay rectangle, its "N more" note, its empty state and its hint
line are unnamed. **Landmark roles were also listed here and are no longer open — see the M301
amendment at the end of this ADR.** Descriptions and relations remain inexpressible, the nodes
attach flat, and no screen reader has been run.

### Third amendment, 2026-10-04 — transcript rows are named, and the premise was wrong

**The gap was not what it said.** The second amendment says "the transcript's rows have no nodes."
They did. Every drawn transcript row is an `egui::Label`, and `Label::ui` already calls
`response.widget_info(...)` (`egui-0.36.2/src/widgets/label.rs:282`) — so every visible row had
been putting a `Role::Label` node into the tree carrying the row's whole laid-out text as its
**value**, with `label = None`. What was missing was the **name**, not the node. A screen reader
could reach each row and get its text as a value; it could not say what kind of row it was.

**`Response::widget_info` cannot name a row at all.** `WidgetType::Label` maps to `Role::Label`
(`response.rs:936`), and on that role egui writes a `WidgetInfo`'s `label` into **`Node::value`,
not `Node::label`** (`response.rs:962-964`). Using it would overwrite the row's text *and still
leave the accessible name empty*. So the rows use **`name_node`** — the helper that writes the
one missing field and leaves role and value as egui built them. That matters beyond tidiness:
`selectable_labels` is on by default, so transcript labels are click- and drag-sensitive, and
`widget_info` would queue a duplicate `OutputEvent` on every such frame.

**Labels are `Transcript row · {kind} · {excerpt}`** — kind first because egui attaches these
flat to the window root, so a node arriving out of context has nothing else saying which region it
is in; excerpt capped at 80 characters, whitespace-folded, from the **sanitised** text the row
already draws. A six-block answer gives six distinguishable names rather than six identical ones.
The **empty state** and a pending **permission ask** are named too.

**Off-screen rows stay unnamed structurally, not by convention.** The measuring pass takes
`Label::layout_in_ui`, which registers no node at all, and every off-screen row only ever runs in
the measurement probe. That is the evidence this did not re-create the cost virtualisation removed —
and it is now in a test, not in a comment.

**Correction, 2026-10-04 (M304): this paragraph carried two numbers no build of this window can
produce, and the sentence above it claimed a test that did not exist.** It read "a test pins this:
800 rich rows → **44 lines drawn → 53 named nodes**, not 1600". Three things were wrong with it.

1. **No such test existed.** The bounding tests used 200-, 300- and 400-row transcripts of
   *one-line user rows* — a fixture that never wraps and names as `You said`, so it exercised
   neither the multi-line layout nor the assistant role the sentence is about.
2. **The numbers are arithmetically impossible under either reading.** A node is built per
   **row**; `FrameSummary::region` counts recorded **blocks** (see
   `the_agent_region_records_one_line_per_markdown_block_not_per_painted_line`). Nodes ≤ rows ≤
   recorded lines. "44 lines drawn → 53 named nodes" has *more* of the second than the first.
3. **There is no painted-line counter in the product at all**, so "44 lines drawn" could not
   have been measured from this tree even by someone who meant to.

It is replaced by the property the sentence was reaching for, pinned in
`eight_hundred_rich_rows_pay_only_for_the_rows_that_are_drawn`: with the fixture the claim names
(800 rich assistant rows, injected through `apply` so they name as `Agent said`), the published
row-name count is **identical at 200 rows and at 800** — measured 15 names either way on a
1440×900 window, against 800 rows in the transcript. Quadrupling the conversation does not
quadruplicate the frame's cost. The negative control is run: with the span slice replaced by
`for row in rows`, the test fails printing *"took the published names from 200 to 800"*.

**On the cost, stated carefully because the measurement could not settle it.** `frame_cost.rs`
reported 3.927 ms → 4.004 ms at 800 rows, but the *same* build swung between 2.64 and 4.02 across
runs, so that pair proves nothing alone. Two paired measurements settle it: back-to-back on the
mandated benchmark (3.493 control vs 4.023 named), and an interleaved four-round A/B with
AccessKit **on** — named 3.55 / 4.41 / 4.08 / 4.03 against control 4.08 / 3.74 / 4.26 / 4.34.
**Fully overlapping: the naming is below the noise floor.** The interleaved probe had to be built
because `frame_cost.rs`'s `paint_once` never calls `enable_accesskit()`, so **that benchmark
cannot see node-building at all** — a real gap in our instrument, recorded rather than papered over.

**Still open after this.** ~~There are no landmark roles~~ — **closed 2026-10-04 (M301), see the
amendment below.** Nodes still attach flat, and there are still **no descriptions and no
relations**, which `WidgetInfo` genuinely cannot express. ~~A `Changed` row's click target has
**no node at all**~~ — **also closed (M303)**, and the *reason* it was believed was wrong twice
over: `Sense::click()` **is** focusable (`egui-0.36.2/src/sense.rs:60`), and
`Context::accesskit_node_builder` is *"get or create"* (`context.rs:3674-3675` — it inserts
`Default::default()` on a vacant id), so naming a node creates it whether or not egui would have
allocated one. That row is now `Open the change · …`, roled `Link`. egui's per-galley
`Role::TextRun` selection run stays unnamed deliberately — its content *is* its value, and naming
it would double-announce; a test pins that it remains one node. And **no screen reader has been
run** at any point in this ADR.

### Fourth amendment, 2026-10-04 — roles were never the limitation, and a test was pinning it

**This ADR said, three times, that egui 0.36 cannot express an ARIA landmark. That was wrong**,
and the error had a specific shape: it is a true statement about one API (`WidgetInfo` has no role
field) generalised into a statement about the toolkit. `Context::accesskit_node_builder` is
**public** (`egui-0.36.2/src/context.rs:3684-3690`) and `accesskit::Node::set_role` is public.
`egui::Area` cannot help — its only role hook is crate-private (`containers/area.rs:649`) — but a
response already in hand is enough.

**What that cost.** Every region in the window had a *name* and no role, so a screen reader's
rotor — which is built from roles — offered no route to the transcript, which is the one region a
reader opened the application for. Names alone are what a reader searches; roles are what they
navigate.

**Decision.** `name_region` takes a `role` and stamps it **after** its own `widget_info` call,
because `fill_accesskit_node_from_widget_info` rewrites the role on the same pass and a role set
beforehand is overwritten every frame — a fix that would have looked like it worked on the frame
it was written. Roles chosen to match what each region *is*: `Log` for the transcript (a region of
past events; `Document` is for text the reader edits, and this pane is read-only), `Navigation`
for the activity bar, `Banner` for the title bar, `Status` for the status bar and the goal band,
`TreeItem` for a file row, `Splitter` for the resize handle (`Adjustment` is for a value, not a
pane's width), and — added at M303, after a review pointed out the record did not match the tree —
`Link` for a "file changed" row, whose click target opens the file.

**The part worth keeping: a test was pinning the limitation.** `a_named_region_is_a_container_
with_a_name_and_nothing_else` asserted the transcript was a `Role::Pane`, with a doc comment
saying landmarks were inexpressible. Correcting the limitation made that test fail — and the
tempting move is to widen the assertion rather than read why it failed. It now asserts the
property that was always wanted (*the transcript is reachable by role*) rather than the
implementation (*it is a pane*), so it survives the next role change and fails if the role is
dropped. **A test that documents a limitation is a test that will resist its correction.**

Also fixed here: **two `Role::Unknown` nodes**, which is the shape egui leaves behind whenever it
allocates a node for a focusable widget and nothing sets a role — one per drawn file-tree row and
the sidebar resize handle, the second of which had a full name and still announced as unknown.
`no_node_in_the_window_is_unnamed_and_unroled` asserts the property over the whole tree rather
than the two sites, because a test aimed at one node passes with the other still broken.

---

## ADR-023: `/restore` — the rollback half of a checkpoint store is a command, and the guard compares against what the harness left behind

**Status: accepted; implemented (M277).**

**Context.** `CheckpointStore::record` runs before every `write` and `edit`
(`crates/epsilon-tools/src/files.rs:356`, `:539`), so every mutation leaves a pre-image on disk.
`CheckpointStore::restore` (`crates/epsilon-tools/src/checkpoint.rs`) had **no production caller** —
every call site was inside that file's own `#[cfg(test)] mod tests`. The store grew a durable
revert record that no tool, slash command or panel could reach: the defect class `AGENTS.md` §5
names by its own name.

**Decision.**

1. **`/restore <path>` is a terminal slash command.** Parsing in `picker.rs` (`Command::Restore`,
   `COMMAND_HEADS`, `SLASH_ITEMS`), execution in `app/commands.rs::run_restore_file`. The desktop
   refuses it **by name** rather than ignoring it, so a window does not tell a reader the command
   does not exist.
2. **Two presses.** The first states what would happen and changes nothing; the second repeats the
   path and ends in `yes`. `AGENTS.md` §6 requires a confirmation before an overwrite, and a
   restore is an overwrite. The confirming line lands directly under the plan it confirms, in the
   transcript.
3. **The store is reached through `epsilon-runtime`** — `describe_checkpoint` (preview) and
   `restore_checkpoint` (apply) — never through `epsilon-tools`. `crates/epsilon-tui/Cargo.toml`
   states a rule worth keeping: *"the shipped TUI reaches the shell through
   `epsilon_core::exec::run_local`, never by naming a tool crate."* Adding a production
   `epsilon-tools` edge for one command would have inverted it. This follows the `/goal`
   precedent (ADR-021): a rule both front ends share lives in the composition crate.
4. **Path resolution belongs to the store** (`CheckpointStore::resolve`). The ledger has one
   spelling of a path; a front end that re-derived it would answer "no checkpoint" for a file that
   has one — which is a lie, not a missing feature.

**5. The restore guard compares against the POST-image, and this is the part that took a
rewrite.** `Entry::digest` holds the **pre**-image, because `record` runs before the write — which
is correct, since a checkpoint taken afterwards captures the damage. But M104's guard then
required the file to *still* hash to that pre-image, and once the write landed it never did. **The
guard was unsatisfiable on the only flow that produces checkpoints: reverting an edit refused one
hundred percent of the time.** Nobody noticed because, in M104's own words, *"No UI reaches this
yet, which is why it is a fix rather than an incident."* Wiring `/restore` up is what made it
observable, which is the argument for shipping the command rather than deleting the write path.

The fix is **not** to weaken the guard. `Entry` gains `post_image` — the hash of what the harness
itself left on disk — written by `CheckpointStore::amend_with_post_image` as an **append** after
the write lands. The ledger is append-only on purpose, so an amendment is a new entry repeating
the same `blob`, `bytes` and `existed`, differing only in carrying the post-image; because
`latest_for` reads newest-first and `resolve` goes through it, the amendment is what a `/restore`
finds. The guard now asks the one question that is answerable: *is the file still exactly what
this harness left here?*

**Consequences.** `/restore` reverts an agent-**created** file (removing it) and an **edit**
(putting the pre-image back). It refuses, leaving the file untouched, when the file has moved since
the harness wrote it — the user's own later work. Three refusal sentences now exist where there
was one, because "the user edited it" and "this entry never recorded what we left behind" are
different facts with different advice, and merging them is how a refusal becomes a mystery.

**Alternatives rejected.** *Delete the write path instead* (the roadmap's other option) — loses
the pre-image, so nothing is recoverable after a wrong edit. *Expose `CheckpointStore` to the TUI*
— breaks a written manifest rule for one command. *A confirmation overlay* — a new `App` state
field and a second input mode for a two-word grammar that reads better in the transcript, next to
the plan it confirms. *A rendered diff* — the crate's diff renderer needs both sides as text and a
checkpoint has only the "before" side; a second diff implementation would answer a question the
byte count and the checkpoint identity already answer honestly. *Weakening the guard to a
timestamp* — the reason M104 chose content over metadata is recorded beside it and still holds.

**Still open, and not pretended otherwise.** ~~`restore` on an `existed: false` entry removes the
file with no content check, because a creation has no pre-image to hash. So a user who edits an
agent-created file and then restores it loses their edit. The preview says `DELETE` in capitals and
waits for `yes`, which is the honest mitigation available without a further schema change.~~
**CLOSED by ADR-026.** The premise was half right and the conclusion wrong: a creation has no
**pre**-image, but it does have a **post**-image, and `amend_with_post_image` was skipping exactly
the entries that needed one — it refused any entry whose `blob` was `None`, and a creation is the
other way to have a `blob` of `None`. So the guard had nothing to compare against, and the deletion
path never called the guard anyway.

---


## ADR-024: A command the product parses is a command the product offers; a key hint is looked up, never written

**Status: accepted; implemented in the part that is user-visible (M279).**

**Context.** The command surface is five hand-maintained tables that nothing reconciles:
`enum Command`, `COMMAND_HEADS` + `ALIAS_HEADS`, `SLASH_ITEMS`, `palette_entries()`, and
`keybindings::DEFAULTS`, plus a sixth in the desktop — its own `SUPPORTED` list and a 38-arm
`head_of`. Two drift checks existed and both were about `parse_command`. Two lies shipped through
the gaps.

**Decision 1 — `/help` and autocomplete must offer every canonical head.**
`SLASH_ITEMS` is what `/help` prints and what the `/` autocomplete suggests. Six heads parsed,
were listed in `COMMAND_HEADS`, and appeared in neither: `theme`, `expand`, `templates`, `branch`,
`exit`, `scoped_models`. A person could type `/theme` and have it work, and had no way to learn it
existed. `ALIAS_HEADS` settles the canonical question — it holds the *alternative* spellings, so
`branch` and `exit` are canonical while `timeline`, `quit` and `q` are their aliases — and all six
are now offered, guarded by `every_offered_command_is_reachable_from_slash_items`, which was
written first and **failed on `/theme`** before anything was added.

**This changes a wire, and that is why it is written down.** `SLASH_ITEMS` feeds the RPC
`get_commands` reply (`crates/epsilon-cli/src/rpc_mode.rs`), which is a wire type under
`AGENTS.md` §2.8. **The reply grows by six entries.** The check there only asserts that every TUI
command *appears* in the list, so it goes green over the change — a test that runs in one direction
cannot see a wire getting wider. Widening it is correct: a client enumerating commands was
enumerating an incomplete set. A client that asserts an exact count would break, and the right
fix for such a client is to stop asserting a count of a surface meant to grow.

**Decision 2 — a palette key hint is resolved from the binding table, not typed.**
`PaletteEntry` carried `binding: Option<&'static str>` with hand-written literals. One of them was
**`"/login"` in the key column** — a slash command advertised where a reader looks for a chord, for
an action that has no key at all; pressing it did nothing. The next advertised `"Ctrl+M"`, which is
not the spelling `key_label` prints, so a rebind would have left it stale regardless.
`KeybindingsManager::key_text` already existed and already stated the doctrine:
*"a rebound key changes its own hint, and a binding that does not resolve produces no hint at all
rather than a dead one."* `PaletteEntry` now carries `key_id`, and `binding` is produced by that
call, so the only values that can reach the key column are chords the handler accepts.

**The guard that let it through, and its replacement.**
`a_palette_entry_that_advertises_a_key_advertises_the_bound_one` asserted `normalized != "x"` — a
blacklist of one value, which passed against one bad literal out of seven. It is replaced by
`a_palette_key_hint_is_resolved_from_the_binding_table_not_typed`, which holds three properties: an
entry with no `key_id` shows no key; an entry with a `key_id` shows exactly what the table
resolves; and no hint begins with `/`. **Negative control run** — reinstating the historical
`"/login"` hint fails that test, naming the entry and the reason.

**Consequences.** `/help`, the `/` autocomplete and the RPC command list are all six entries wider.
The palette no longer shows a key for "Connect provider", because there is not one.

**What this ADR does not do.** It does **not** move the registry into `epsilon-contracts`. The
larger unification — one `REGISTRY` table with per-frontend capability declared as data,
replacing `COMMAND_HEADS`, `ALIAS_HEADS`, `SLASH_ITEMS`, `DEFAULTS`, `SUPPORTED` and `head_of` — is
designed and recorded in [`docs/STATUS.md`](../STATUS.md) and is **not implemented**. This ADR
fixes the two lies that are user-visible today, without the 350-line move that carries the most
risk. The invariant in `AGENTS.md` §2.4 stands; this is a step toward it, not its completion.

**Amendment, M290 — the host module landed; the table is still four tables.**
`crates/epsilon-contracts/src/command.rs` now holds `SlashItem`, `SLASH_ITEMS`, `Command`,
`COMMAND_HEADS`, `ALIAS_HEADS` and `parse_command`, and `epsilon-tui/src/picker.rs` re-exports all
six, so **no call site in the workspace changed**. `epsilon-desktop/src/commands.rs` reads the
registry from `epsilon_contracts::command` directly rather than through `epsilon_tui::picker`, which
is the edge that crate's own manifest predicted (*"If that ever stops being true, the edge is one
module"*). The desktop's remaining `epsilon-tui` dependency is the *palette* and the *numbers*
(`ThemeMode`, `ui::theme`, `ui::metrics`) — colour roles and values, not command behaviour.

So the paragraph above is now narrower and more precise: the **command half** moved to the crate
below both front ends, and the **rows are still four hand-maintained tables** (`COMMAND_HEADS`,
`ALIAS_HEADS`, `SLASH_ITEMS`, `keybindings::DEFAULTS`) that merely live in one crate. One flat
`REGISTRY` of `Action` rows is still unbuilt and nothing in the product depends on it.

Two decisions carried across the move rather than re-made:

1. **`parse_command` stays a hand-written `match`.** Rewriting it as a table lookup deletes ~80
   lines and needs four arities spelled out (`/model` bare opens the picker, `/model x` sets an id,
   `/models` is both, and a bare `/login` is valid and means "ask me"), plus `scoped-models` and
   `scoped_models`. That is a behaviour change dressed as a refactor (`AGENTS.md` §5), so the arms
   moved verbatim.
2. **The guard is repaired, not relaxed.** `no_source_file_lost_a_test_since_head` reported the
   move as *four deleted tests*, because `git diff HEAD` cannot see a file git has never heard of —
   the third recorded instance of *"a check that cannot see a thing is indistinguishable from a
   check that found nothing"*. It now reads `git ls-files --others --exclude-standard -- crates/`
   and counts those files' tests as added. **A moved test is not a deleted test, and the guard now
   says so.**

---

## ADR-025: The sidebar is a control, so the window's minimum size must be able to show it

**Status: accepted; implemented (M289).**

**Context.** `Preferences { sidebar_width, sidebar_visible }` existed in
`crates/epsilon-desktop/src/layout.rs` with a documented clamp range, and was **dead state**: set to
`default()` once in `IdeApp::new` and `without_agent`, and never mutated. No drag handle, no
collapse control, no persistence — so the objective's "resizable and collapsible panes, remembered
state" was met by a struct rather than by a product.

**Decision.**

1. **A resize handle on the sidebar's right edge**, 6 points wide — exactly `GUTTER`, so it can
   never reach into a pane — using egui's own idiom (`Sense::drag()` plus `Response::drag_delta`,
   which is what `egui::containers::Resize` does). `Sense::drag()` rather than the bare
   `Sense::DRAG`, because that one word carries `FOCUSABLE`, and egui builds an AccessKit node for
   every focusable widget — which is what gives the control's accessible name somewhere to live.
2. **`Ctrl`+`B` folds and unfolds it** — the chord VS Code, Sublime Text and Atom already bind to
   this — plus `←`/`→` in 16-point steps. The keyboard route is not a convenience: a 6-point target
   is under WCAG 2.2 SC 2.5.8's 24 by a factor of four, and arrow keys are how a screen-reader
   toolkit resizes a splitter. The handle's width gap is documented in `layout.rs`, not hidden.
3. **The control lives in the title bar**, because a control that vanishes with the thing it
   controls cannot put it back. The status bar prints the keys, and **only while they work on that
   frame** — nothing while the palette is open, no arrows once the sidebar has folded. That is
   `AGENTS.md` §2.7 in its smallest form: a hint for a key that does nothing is worse than no hint.
4. **Persistence is one small JSON file per workspace**, two keys and no secret. Corrupt,
   unreadable, truncated, wrong-typed or missing-a-key all fall back to the defaults and **never
   fail the launch**; a width stored out of range is clamped on the way in.

**The per-workspace keying was measured, not chosen.** One global `desktop.json` shipped first,
then met a hostile file written into the real config directory: **nine tests went red**, because
the integration tests resolve the config directory themselves and inherited whatever the person
running them had resized. That is a test that is a coin flip, which is worse than no test. Re-keying
per workspace, and bounding the last two helpers, brought it to zero with the poison files still
on disk. Per-workspace is also the better answer: a monorepo's file tree wants a wide sidebar and a
dotfiles repo's does not.

**And the minimum window size, which is what made the rest honest.** `lib.rs` set
`with_min_inner_size([720, 480])` — **below `MEDIUM` (860)**, so at the only size a window cannot
be dragged away from, the layout folds the sidebar for narrowness. A reader who had sized it,
collapsed it or restored it would be shown a window silently disagreeing with their own choice, and
could not drag it back. **A minimum that contradicts a control is not a minimum; it is a bug waiting
to be resized into.** It is now `900×560`, and the layout's degradation thresholds are untouched —
a window can still be shrunk below this by a display change or a window manager, and the tiers
still fall away in order. This only says where the guarantee *starts*.

**A defect the tests found.** `paint` carved the layout *before* reading keys, so `Ctrl`+`B` was
answered one frame late: the frame that saw the key still drew the sidebar. The carve now happens
after every key handler, which is also what `pane_keys` needs, since a splitter drag resolves
against last frame's rects. I did not reason that out beforehand — the keyboard test failed on it.

**Consequences, stated plainly.** The new bindings are read in `app.rs` alone: they are **not** in
the command palette and **not** in `/hotkeys`, because both need a `Command` variant in the
terminal's `picker.rs`, which the shared `REGISTRY` does not yet exist to host. `Ctrl`+`B` also means
transcript page-up **in the terminal** — a different application's namespace, and a known cost of the
registry still being unbuilt.

## ADR-026: The deletion path is the most destructive thing the checkpoint store does, so it is guarded too

**Status: accepted; implemented (M298).**

**Context.** ADR-023 closed the half of the revert path that writes a pre-image back and left the
other half explicitly open: `restore` on an `existed: false` entry — reverting a **creation** —
called `remove_file` with no content check at all (`crates/epsilon-tools/src/checkpoint.rs`). The
stated mitigation was the `DELETE` preview and the typed `yes`.

That mitigation does not hold up under reading. It is a warning about the *outcome*, not about
*whose work* is in the file, and the case that matters needs no unusual timing: the agent creates
`src/new.rs`, the user types their own work into it, they ask to undo the creation, and `/restore`
deletes their work after a preview that was accurate about everything it said. `AGENTS.md` §6
requires a refusal path to leave the file untouched; this path had no refusal to leave.

The second finding is the one that made it fixable rather than merely reportable.
`amend_with_post_image` skipped any entry whose `blob` was `None`, documented as *"nothing was
captured to amend"*. That is true of a **skipped capture** (too large, unreadable) and false of a
**creation**. A creation has no pre-image to repeat *because there is no pre-image*, but the write
that follows leaves one, and that is exactly what the guard needs. So the two entries the code
treated alike are the two that need opposite handling, and the deletion guard had nothing to
compare against even once it was called.

**Decision.**

1. **The deletion path runs the same guard as the write path.** `restore`'s `existed: false` branch
   calls `check_unchanged` before `remove_file`. One question, asked in the one place where the
   answer can be acted on.
2. **A creation gets a post-image.** `amend_with_post_image` skips on `blob.is_none() && existed`
   rather than `blob.is_none()` alone, so a creation is amended and a genuinely skipped capture is
   not. `existed` is already on every entry; no new field, no schema change, no migration.
3. **The two entries now refuse for different stated reasons.** A creation with a post-image that no
   longer matches says the file changed since the harness wrote it. A creation with neither (a ledger
   written before this ADR) refuses rather than deleting, and its advice says *"delete it by hand"* —
   telling a reader to "delete it" when the file is the thing in question would be nonsense.

**Consequences.** `/restore` still reverts an untouched creation, which is the case the command
exists for; `an_untouched_created_file_is_still_removed_by_restore` exists so that a guard which
refuses everything cannot pass as a safe guard. Two tests were fixtures that lied about the product
— they recorded a creation and wrote the file without the amendment every real `write` performs, a
shape the product stopped producing at M277. They now mirror it, which is the M277c lesson applied a
second time.

**Alternatives rejected.** *Weaken the warning instead* — a louder `DELETE` does not change what the
command does. *Add a confirmation dialog* — the typed `yes` is already the confirmation, and the
problem is not that the reader was not asked. *Trust the preview* — the preview cannot see the
user's later edit either; it is generated from the entry, not the file. *Hash the creation into a
blob* — it would have to be a blob of bytes that did not exist, which is a second concept doing the
work `post_image` already does.

---

## ADR-027: A subagent inherits the ruleset and not the session's memory

**Status: accepted; implemented (M300).**

**Context.** `build_task_child` (`crates/epsilon-core/src/lib.rs:3394`) gave every subagent
`Arc::clone(&self.gate)`. That looks like the safe choice — one policy for the whole session — and
it was the unsafe one, because `RulesetGate` keeps the session's remembered decisions *inside* the
gate: `memory: SessionMemory` (`crates/epsilon-safety/src/gate.rs:47,54`) is a list of
`(capability, pattern)` pairs pushed on an `AlwaysSession` answer (`gate.rs:118`) and read back at
`gate.rs:93-102`.

So one "always allow bash" in the main loop silently authorised `bash` for **every subagent
afterwards**, with no prompt and no inheritance decision anywhere. Confirmed as a live path, not a
test artefact: `epsilon-runtime:4702` registers `TaskTool`, `exec.rs:795` injects the task host,
`delegate_task` reaches the child, and the child's own calls go through `exec.rs:452`.

The roadmap's "six lines" estimate was wrong, and the reason is worth recording:
`Authorization { decision, asked }` **cannot distinguish an allow remembered from memory** (`:99`)
**from an allow the engine's own rules produced** (`:108`) — both are `AllowOnce, asked: false`. So
the derivation cannot live where a decision is being made; it has to happen where the gate is
*handed over*.

**Decision.**

1. **`PermissionGate` gains `fn child_gate(&self) -> Option<Arc<dyn PermissionGate>> { None }`.**
   `None` means "share self", so `AutoAllow`, `AutoDeny` and every test's own gate compile
   unchanged and keep their behaviour — a gate with no session state has nothing to withhold.
2. **`RulesetGate` overrides it**: a clone of the same `engine`, the same `ui`, and a **fresh**
   `memory`. `PermissionEngine` gains `#[derive(Clone)]` — it holds a `Vec<Ruleset>` that never
   changes after construction, so a clone is a copy of a value, not shared mutable policy.
3. **`build_task_child` asks.** `self.gate.child_gate().unwrap_or_else(|| Arc::clone(&self.gate))`.

**What is inherited, and why each part.** The *ruleset* is the owner's policy — identical for every
agent, and cloned rather than rebuilt so a deny cannot be escaped by being spawned into. The *UI*
is shared because it **is** the user: a subagent's prompt goes to the same person, and a second UI
would mean a question asked where nobody is watching. The *memory* is withheld: it records
decisions made **about this conversation**, and inheriting it converts one "yes, always" into a
standing permission for agents the user was never shown.

This is opencode's rule, read from the reference rather than recalled
(`references/opencode-1.18.34/packages/opencode/src/agent/subagent-permissions.ts:21-25`): a
subagent's permissions derive from the parent's **denies**. Its filter also keeps
`external_directory` rules regardless of action and *adds* synthetic `todowrite`/`task` denies —
details Epsilon's rule does not reproduce, because Epsilon derives by construction rather than by
filtering a list.

**Consequences.** A subagent that needs the same capability asks again. That is the cost, and it is
the correct one: the alternative is a permission no human granted. Denies are unaffected —
monotonicity is in the engine, which the child shares.

**Amendment, 2026-10-04 (M303) — two consequences of the fix, found by an independent review.**

1. **A grant made on a subagent's prompt now dies with that subagent**, because the child's memory
   is fresh. Before this ADR the keypress promoted into the *shared* memory and lasted for the
   whole session. That is the fix working, but it has a cost nobody wrote down: the transcript row
   said `permission allowed (session)`, which promised a longer scope than the code provides.
   `AGENTS.md` §2.7 is exactly this — a control that does not do what it names — so the row now
   says what is true: **"allowed (for this agent, not its subagents)"**.
2. **The prompt cannot say which agent is asking.** `PermissionRequest` carries `request_id`,
   `capability`, `pattern` and `detail` (`agent_event.rs:52-61`) and **no agent identity**, while
   the child shares the parent's `ui` because that *is* the user. So a reader who answers "no" to
   "run this shell?" cannot afterwards tell whether the main agent or a subagent was stopped.
   Fixing it properly means adding an optional agent label to `PermissionRequest` — a wire type,
   so `AGENTS.md` §2.8 applies, and it wants its own decision rather than a field added here.
   **Recorded as open rather than done.**

**Alternatives rejected.** *Filter the parent's memory into the child* — needs to distinguish
remembered allows from engine allows, which `Authorization` cannot express (see above). *Pass a
ruleset into `build_task_child`* — puts a policy decision at the call site, so the next call site
would get it wrong. *Make it a required trait method* — would break every implementor for a
decision only one of them has an opinion about.

**Two failures worth recording, because the first version of the test could not have caught the
bug.** The RED test drives a **real** parent, a real `task` call and a real child, then counts the
`bash` asks reaching the UI. Getting that scenario right took three attempts, and each failure was
the fixture rather than the product: a `task` call missing its required `description`; then two
`bash` steps before the `task`, so the **parent** ran both and the child never got a turn; then a
`say` in between, which ended the run. **A test that never spawns the subagent it is about cannot
see the rule it names**, which is why the precondition assertion ("a bash call actually happened")
is in the test rather than assumed. Separately, the first version of the override was written in
the inherent `impl RulesetGate` block instead of the trait impl: it compiled, carried a full doc
comment, and was **never called**.

### Amendment, 2026-10-04 (M309) — the same door, and the hook that never reached the child

**This ADR derived the gate. It did not touch the hooks, and that was the same defect one field
over.** `build_task_child` copies the model, thinking level, skills, system overlay, agent defs,
task depth and agent brief to the child, and calls `child_gate()` for authorization — and never
called `set_tool_hooks`. Every agent starts on `NoopToolHooks`
(`epsilon-core/src/lib.rs:765`), so an extension's ADR-019 `before_tool_call`, installed on the
parent at `epsilon-runtime/src/lib.rs:1318`, reached **no subagent at all**.

What that costs is not theoretical: a hook whose whole purpose is to *vet or block* a tool call
blocked it for the top-level agent and let every child run it, silently. `AGENTS.md` §2.7 again.

**The decision, and the one place it differs from the gate.** The child now receives the parent's
hooks **by `Arc`, inherited rather than derived**:

```rust
child.set_tool_hooks(self.tool_hooks());
child.set_turn_hooks(self.turn_hooks());
```

**Inherited where the gate is derived, and the asymmetry is deliberate.** The gate needed a fresh
memory because a *session's remembered allow* is a decision about **one conversation** — sharing it
let a user's single `a` keypress authorize `bash` for every future subagent. A hook is not a
decision about a conversation: it is **operator policy**, the extension's own veto, and policy that
applies to the agent but not to the agents it spawns *is* the defect. There is nothing in a hook to
un-remember, so nothing needs deriving. Both accessors clone an `Arc`, so the cost is two refcounts
per delegated task.

**Evidence, RED first.** `a_tool_hook_on_the_parent_sees_the_childs_calls_too`
(`epsilon-safety/tests/loop_with_rules.rs`) scripts `bash` → `task` → `bash` → `say`, installs a
recording `ToolHooks` on the parent, and counts. Before the fix it printed
**`left: 1, right: 2`** with `All calls seen: ["bash", "task"]` — the parent's own two calls, and
the child's bash absent. After: `2`.

**And the fixture fooled it once too, in a way worth recording.** The first version queued a single
`AlwaysSession` answer, so `RecordingUi` denied the **`task` call itself** once the queue emptied —
the fan-out never happened and the test saw the parent's two bash calls. **`task` asks.** The
precondition now requires a successful `task` result *separately* from a bash result, because "a
bash happened" is satisfied by the parent alone, which is precisely how the sibling test in the
same file was fooled twice.

---

## ADR-028: A store stops writing when its own lease is gone, and a refusal it cannot prove is not a refusal

**Status: accepted; implemented (M312).**

**Context.** A session file has exactly one writer, enforced by `crates/epsilon-session/src/lease.rs`:
acquisition is `create_new` on `<session>.lock`, a live holder re-stamps it every
[`HEARTBEAT`], and `Drop` removes the file. That protocol is sound for *acquisition* and says
nothing about what a holder does afterwards, because nothing ever looked again. The store's field
was `_lease` — held for its `Drop`, never consulted — and `append` writes through a handle opened
`O_APPEND` at construction. So a takeover, once it happened, was invisible to the victim: it went
on interleaving lines into one JSONL stream for the rest of the session. `SessionLease::refresh()`
had **zero callers**, which is the same fact from the other side.

The gap was not hypothetical. M294 fixed the *other* half — a stolen-from holder's `Drop` no longer
deletes the thief's lock — and recorded the residual in those words. M310 then measured why the
obvious fix is not two lines:

1. Two production call sites discarded `append`'s result, so a refusal would have been a **silent
   drop** — trading an interleaving risk for data loss nobody is told about. M311 gave both a way to
   report (`AgentEvent::Notice`) *before* the store was allowed to refuse at all.
2. `lock_is_ours()` answers one `bool`, and it answers "no" for **someone else owns this** and for
   **"I could not read it right now"** alike. A check built on it refuses legitimate writes over a
   transient sharing violation, and this process's own heartbeat opens that same file for writing
   every two seconds.

**Decision.**

1. **`SessionLease::ownership()` returns `LeaseOwnership`, not a `bool`** — `Ours`, `Taken { holder }`,
   `Missing`, `Unreadable`. `lock_is_ours()` survives as a wrapper, because `Drop` genuinely does not
   need the distinction: it must not delete a file it cannot identify, and all three non-`Ours` cases
   say the same thing to it.
2. **`append` refuses on `Taken` and `Missing`; it does not refuse on `Unreadable`.** The
   discriminator is the *kind* of the I/O error, not the file: `NotFound` is evidence — only a holder
   removes a lock file, and we are a live holder, so a missing one means another acquisition existed
   and has since finished — while a present-but-unopenable lock says nothing about who owns it.
3. **`persist_migration` refuses too**, and it is the more dangerous of the two paths: a migration does
   not add a line, it rewrites the file and renames it over the top, so a store whose lease was taken
   would not interleave with the other writer, it would **replace its file** and take every entry that
   writer appended since.
4. **The check is the first statement in `append`, before the attempt counter.** One check covers every
   typed wrapper — `append_goal`, `label`, `record_turn_end`, the branch summaries — so a caller cannot
   route around it by picking a different door. And a refusal is not a write attempt: it must not spend
   the next attempt an F07 fault plan was armed for, and `appends()` must keep counting what was written.

**First refusal, not a silent re-open.** The alternative considered and rejected is to re-acquire and
carry on. Re-opening a session means acquiring a lease **another process currently holds** — the exact
thing `open` refuses — and a wait there is up to [`STALE_AFTER`]. A refusal that says "close the other
window and reopen this one" costs one turn and loses no file; the alternative either corrupts the
session or blocks the reader for thirty seconds on a write that is going to fail anyway.

**Consequences.** Two writers are now impossible to sustain *silently* rather than impossible to
sustain: the second one gets an error, a notice, and a file that is byte-for-byte what it was. A
process whose lock file was deleted out from under it by hand also stops writing — deliberately, since
it can no longer prove it is the writer, and the remedy (reopen) is immediate.

**Alternatives rejected.** *Check in `Drop` too* — `Drop` cannot report; it can only decline to delete
a file it cannot identify, which it already does. *A write lock held open for the store's lifetime
(`LockFileEx`)* — this workspace forbids `unsafe`, and a platform lock is not portable to the second
front end's process model; the file protocol stays and gains a reader. *Only refuse on `Taken`* — that
leaves the commonest shape of the defect open: a thief that has since closed leaves no lock to read,
and the victim resumes writing into a session somebody else owns.

**Evidence, RED first.** Four tests: the store refuses on a taken lock, on a vanished one, does **not**
refuse on one it cannot read this instant, and the agent loop turns that refusal into a
`Notice` and fails the run. **Negative control run** — with `refuse_if_lease_lost` stubbed to `Ok(())`
the three refusal tests fail, and the core one fails printing the whole defect as a value: the run
returns `Ok(RunOutcome { produced: [Assistant(… "an answer nothing recorded" …)], reason: Completed })`
— a completed turn, written into a session file another process owns. The fourth test stays green
under the control **by design**: it asserts the non-refusal, so it is there to catch a fix that is too
eager, and its meaning is only visible in the control's output.

---

## ADR-029: The window's command list stays two lists, because a macro cannot express one — and three verbs joined it anyway

**Status: accepted; implemented (M312).**

**Context.** `crates/epsilon-desktop/src/commands.rs` answered **3 of the 37** heads the shared
registry knows. That is honest — a head the window lacks is refused **by name**, so a reader is
told where it lives — but the window had a subagents pane and a button that opens it, a session it
can name, and a transcript it can empty, while `/tasks`, `/session` and `/clear` went on saying
*"not in this window; it works in the terminal"*. The objective asks for the desktop "to the highest
standard", and a third of the surface is thin.

The file also had a structural smell: `SUPPORTED` declared what the window runs, the arms of
`try_command` implemented it, and a hand-written list in a test held the two together.
`window_runs`, the only reader of `SUPPORTED`, has **no production caller** — it feeds
`refused_heads()` and tests.

**Decision.**

1. **Three verbs joined, each because the window already had the effect.** `/tasks` is a
   `set_sidebar_view` the activity bar's button already performs. `/session` reports the file, the
   model, the context gauge and the state — the four facts this window holds, in the terminal's
   one-line shape, because a window has no leaf and no tree to walk. `/clear` empties the view and
   keeps the file and the figures, which is exactly what the terminal's says in its own words.
   **The criterion was "the action already exists", not "the command is easy"**: `/resume` is a
   session *handover*, which `SidebarView::Sessions` still declines to do, so it stays refused by
   name.
2. **The declaration and the arms stay two lists.** A `macro_rules!` emitting both from one table is
   the right idea and **does not work**: it will not expand inside a match's arm-list, so it would
   have to write the whole function from **item** position, where its arguments are parsed as
   *items*. The measured failure is `E0424: expected value, found module self` at every arm. Two
   tests hold the pair instead — `the_window_runs_exactly_the_heads_it_declares_supported` and the
   integration guard that drives every head through the real `try_command` — and **the first of
   those two caught exactly this drift when the three verbs landed**, which is the evidence that the
   pair is held by something rather than by hope. What the macro would have prevented is a **red
   test**, not a shipped lie, and a red test is the cheaper failure.
3. **A refusal stays the default.** A head with no arm is refused by name, in the reader's own
   spelling. Growing the set remains a product decision: each name is a verb the window stops
   refusing.

**Consequences.** `/tasks`, `/session` and `/clear` work in the window — six of 37, up from three.
/new` and `/resume` still do not, by name and for stated reasons. The refusal set is 31 heads rather
than 34, and `Transcript::clear` had to reset the live-row indices as well as the rows, because
those indices are absolute and a stale one writes into the wrong row.

**Alternatives rejected.** *The macro* — measured, above. *Mapping `/resume <path>` onto
`IdeSession::resume`* — it would replace the live conversation without a confirmation the session
handover has never had, which `AGENTS.md` §6 forbids. *`/theme` in the window* — it needs a
high-contrast palette that does not exist yet, so wiring the verb now would repeat the very claim
`/theme` was just caught making.

**Amendment (full-port round 4).** `/help` joins under the same criterion: the effect already
exists — `Ctrl+P` opens the command palette, which lists every file and every command this window
answers, so it is the window's honest equivalent of the terminal's command reference. 8 of 37, up
from the seven above (plus `/compact`, `/goal` and `/theme`, which landed alongside).

**Amendment (full-port round 6).** `/new` joins under the same criterion: naming the file reuses
the runtime's own `fresh_session_file`, and switching reuses `switch_session` — the path the
sidebar's clicks already take. 9 of 37. Two things this amendment does not change: the busy refusal
(switching under a running turn would orphan its writes), and the no-agent answer (a file with no
session to switch is half a verb, so the window names what's missing).

**Amendment (full-port round 7).** `/resume` joins under the same criterion: bare opens the
sessions sidebar (the browser this window has — the same view `/sessions` shows), and a path hands
over through `switch_session`, the clicks' path. Same-path resumes say so rather than rebuilding
silently, following the click path, which skips them for the same reason. 10 of 37. The busy
refusal and the missing-file wording ("no file at ... — check the path") are the terminal's, kept
word for word so the two front ends cannot disagree about what went wrong.

**Amendment (full-port round 8).** `/fork` and `/clone` join under the same criterion: both run
through the store's `fork_branch_at` — the call the terminal and the RPC surface already share —
and differ only in the verb they report, so the window shares one helper the same way. Neither
switches: the fork is stored and named, and moving there is `/resume`'s job. 12 of 37.

---

---

## ADR-030: High contrast is a third palette, chosen by name, and it dims nothing

**Status: accepted; implemented (M312).**

**Context.** `AGENTS.md` §8 asks for **OS dark/light and high-contrast support** in both front ends.
The product had two palettes and no third. Worse, `/theme`'s description said
`switch palette: dark, light, high-contrast` while `ThemeMode` had two variants — so
`/theme high-contrast` answered **"unknown palette"**. A milestone removed the word to stop the lie
and recorded the capability as roadmap work.

**That was the right call at the time and the wrong place to stop.** The requirement is real, the
description was already right, and the fix that keeps the promise is the code rather than the
sentence.

**Decision.**

1. **A third palette, `ThemeMode::HighContrast`, in the terminal's table — so the desktop inherits
   it.** `IdeTheme::from(ThemeMode)` already adapts a terminal palette, and the desktop is the front
   end that has to *paint* the page. One table, two front ends, and the shared-tokens rule in
   `AGENTS.md` §8 honoured rather than worked around.
2. **The claim is a number, and the number is 7:1 (WCAG 2.2 SC 1.4.6, AAA) on pure black** — where
   the other two palettes hold 4.5:1 (AA). Measured for **all 21 roles**, not sampled, and every
   role is also checked against the palette it replaces, so "never worse than the default" is part
   of the promise rather than an afterthought.
3. **Bands are backgrounds and are held to the opposite property.** `surface_*` must stay near-black
   or they read as text; what they are checked for is the **text they carry**. The first version of
   the test applied the foreground floor to them and failed a role that was right.
4. **Nothing in the palette is dimmed.** `Slot::quiet` adds `Modifier::DIM`, and a measured 7:1 role
   rendered dim is not a 7:1 role — the mode's whole promise is a number, and `DIM` is a terminal's
   discretion about a number. Secondary roles are carried by **lightness** instead (`text_faint`
   `#b0b0b0` against `text` `#ffffff`), which survives a terminal that ignores attributes. The
   **semantic hues keep the dark table's slots**, so a 16-colour console still tells success from
   danger.
5. **Reached by name, not by the bare `/theme` toggle.** `flipped()` stays dark↔light. A key a user
   presses to change their mind about brightness should not sometimes land on a third thing, and
   turning the ceiling on is a decision rather than a preference.

**The control was a finding, and not the one expected.** The assumption going in was "the dark
palette misses the AAA floor on black, so this fixes it". Measured, the dark palette's body text is
**15.1:1 on black** — a dark palette is built for a dark background and pure black is its best case.
The roles that actually fail are the quiet ones: `text_faint` is **3.4:1**. That is a better
statement of what the mode is for than the one it replaced, and it is why the test checks *every*
role against both the floor and the palette it supersedes.

**Consequences.** On a 16-colour console this mode shows **one colour** for body, secondary and
tertiary text, because all three are `Color::Reset` and only lightness separates them — which a
terminal may not honour. That is the honest trade and it is stated in the table's docs and in the
test: **one legible colour beats two, one of which is illegible.** In a 24-bit window, where the
mode paints its own black page, all three are distinct.

**Alternatives rejected.** *Detect the OS high-contrast setting* — egui exposes no hook, winit's
system-theme signal is light/dark only, and this workspace forbids `unsafe`; a name the user sets is
honest where a detection that cannot see is not. *Make it a modifier over the dark palette* — the
light table's own docs reject that reasoning for the same reason: the numbers are not a transform of
each other. *Put high contrast in the bare `/theme` cycle* — see 5.

---
---

## ADR-031: One chord grammar, two binding tables, and a window that can finally be rebound

**Status:** accepted, 2026-10-05 (M315). Amends `AGENTS.md` §2.4's evidence, not its rule: the
rule was right and the tree did not satisfy it.

### The problem, measured

`AGENTS.md` §2.4 says slash commands, palette entries **and keybindings** are "declared once and
read by every front end". Commands were true — `COMMAND_HEADS` lives in `epsilon-contracts` and
both front ends read it. Palette entries were half true. **Keybindings were not true at all**:

- the terminal had a table of 24 ids in `epsilon-tui/src/keybindings.rs`;
- the window had **seventeen literal `consume_key` reads** and one hand-written
  `const SIDEBAR_TOGGLE_KEY: &str = "ctrl+b"` used in four hint strings.

So `keybindings.json` could touch the terminal and not the window. The comment above that constant
compounded it: it claimed *"the registry that would carry it does not exist yet"* and that
*"the terminal binds `ctrl+b` to page-up in its transcript"* — **both untrue**. The registry did
not exist, which was the real point; and `ctrl+b` is the terminal *composer's* readline
back-character, not a table entry at all.

### The decision

**1. The chord grammar moves to `epsilon-contracts` (`keybind.rs`), UI-neutral.** `Chord::parse` is
now the one function that decides what `"ctrl+shift+f2"` means. It is a verbatim move of the
terminal's `parse_key_id` with `KeyCode`/`KeyModifiers` replaced by `Key`/`Mods`, so that the crate
keeps no UI dependency — declaring a keyboard must not invert the dependency direction the whole
repository is built on (`design/epsilon-architecture.md` §2). Each front end converts once, at its
own input boundary.

**2. Two tables, not one: `TERMINAL` and `WINDOW`, in the same module.** This is the part worth
arguing, because it looks like the thing the ADR was supposed to prevent.

A terminal and a window are different programs. `F6` cycles a focus ring in the window and means
nothing in the terminal. `ctrl+b` moves a text cursor in the terminal's composer and folds the
window's sidebar. Collapsing both into one list would mean either two entries per id with a
per-front-end flag on every chord, or a chord that means different things in different places —
and a table like that is harder to read than two tables that are adjacent.

`AGENTS.md` asks for *"per-front-end capability declared as data rather than as a second
hand-maintained list"*, and **that is exactly what this is**: one module, one format, one grammar,
two lists, each front end reading its own. A reader looking for "what does this product bind?" finds
both in one file. A reader looking for "what does *this* front end bind?" filters one list.

**3. Ids are per *action*, not per key, and that is a real modelling change.** `Escape` means four
things in the window — close a palette, leave a pane, clear the draft, refuse an ask. There is no
single `app.interrupt` there; there are four ids, and which one answers is decided by the drawing
order, which is code. A terminal has one composer; a window stacks surfaces.

**The consequence that had to be written down:** a chord may appear against more than one id in
`WINDOW`, so the "no chord is bound to two ids" rule now holds for `TERMINAL` **and not** for
`WINDOW`. In a terminal a shared chord is a coin toss decided by handler order; in a window it is
a priority the drawing already states. The test says so rather than being quietly narrowed.

**4. `take` matches modifiers for *equality*, and that is not a detail.** egui's own `consume_key`
matches **logically** — extra Shift and Alt are ignored — which is how `Shift+Enter` came to be
swallowed by a plain `Enter` read in the composer, and M314 fixed it with a hand-written guard
scanning the event stream. **That guard is deleted.** A binding is a choice by a person, so it
means one specific set of modifiers, and the general rule replaces the special case. The test that
proved the special case still holds.

### What this costs

- `epsilon-contracts` gains a module. It is the crate `AGENTS.md` §2 calls the "stud standard", and
  this is vocabulary, not logic — which is what that crate is for.
- Two places now convert: the terminal's `KeySpec` and the window's `neutral_key`. Both are
  mechanical and both are covered by a round-trip test (`a_label_round_trips_to_the_chord_it_came_from`)
  that fails if a printed hint cannot be parsed back into the chord that produced it.
- **The window's `neutral_key` had a bug the tests caught on the first run**, and it is worth
  recording: it assumed egui has no letter keys and returned a placeholder for all of them, so
  `Ctrl+P` — the one chord every IDE uses for a command palette — never matched and four palette
  tests failed. The lesson is the repo's own: a static reading of a dependency's API is a claim,
  and only a running test is evidence.

### Alternatives rejected

- **One table with a per-front-end flag on every chord.** Rejected: it makes the common case (a
  binding both front ends share) harder to read than two adjacent lists, and it puts a conditional
  in the one place a reader looks to find out what a key does.
- **Keep the terminal's table where it is and give the window its own.** Rejected: that is the
  status quo, and it is what the matrix and the a11y survey both flagged.
- **Adopt the terminal's `keybindings.json` format but not its parser.** Rejected: two parsers are
  two answers to "what does `ctrl+shift+f2` mean", and they would eventually disagree.

**Amendment (full-port round 12).** The first two rows the tables gained after the move:
`window.session.new` (`ctrl+n`) and `window.session.fork` (`ctrl+alt+f`), read in `pane_keys`
below the palette guard and running the same `run_new_session` / `run_fork_session` the `/new`
and `/fork` heads run. Nothing about the mechanism changed — same tables, same `take`, same
`keybindings.json` — which is the point the amendment records: the first real users of the
machinery arrived through the front door, driven by real key events in tests.
