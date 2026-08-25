# Epsilon Architecture Decision Records

Format per ADR: **Decision** -> **Alternatives considered** -> **Why this is best for Epsilon**
(citing Phase 1-2 findings) -> **"Is this truly the best possible choice?"** honest answer.
Status: all Accepted for v1 unless noted. Superseded ADRs are kept at the bottom.

Citation key: `HA` = reports/harness-analysis.md, `CA` = reports/comparative-analysis.md.

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

**Decision.** epsilon-{contracts,llm,session,core,tools,safety,ext,mcp,tui,server,cli};
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

**Best possible?** Yes for this scope. The one risk of a vocabulary crate is churn: changing a
stud-shape rebuilds every attached brick. Mitigated by the freeze policy in ADR-017.

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

**Decision.** Selection precedence everywhere: `--model` flag (ephemeral) > saved
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

- **ADR-D01 (deferred): WASM extension sandbox.** Revisit if process-isolation overhead ever
  matters or component model stabilizes further.
- **ADR-D02 (deferred): Windows ACL restricted-token confinement backend.** Post-v1; requires
  dedicated security review; job-object limits ship meanwhile.
- **ADR-D03 (rejected during design): adopt DSH surface-shadowing data structure literally.**
  Our compaction-entry-with-shadow-ranges achieves the same model-view rewrite over pi's simpler
  entry format; literal port would fork the session format for no user-visible gain.
