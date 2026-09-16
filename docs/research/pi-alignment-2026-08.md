# Pi Alignment Report — Structural Side-by-Side (2026-08)

> Historical research, not current product status. See the [archive index](README.md)
> and [verified audit](../../audit/README.md) before relying on completion or safety claims.

Second alignment pass against pi-0.84.2 (first was the pre-build design audit). Method:
every claim below was verified by reading both trees (`references/pi-0.84.2/packages/*`
vs `crates/*`) during this pass — not carried forward from earlier notes.

## 1. Layer-by-layer correspondence

| pi layer | Epsilon analogue | Verdict |
|---|---|---|
| `packages/ai` — hand-written per-provider HTTP/SSE adapters, generated catalog, retry | `epsilon-llm` — `providers/{anthropic,openai_compat,google}.rs`, `http.rs` SSE decoder, catalog v2 from models.dev, `retry.rs` | **Aligned** (post-M17; previously diverged via genai — that debt is closed) |
| `packages/agent/src/agent-loop.ts` — pure two-level loop | `epsilon-core/src/lib.rs` — same outer/inner shape, drain modes, sticky reasons | **Aligned** |
| `agent.ts` stateful wrapper + event reduction | `Agent` struct + broadcast bus; frontends reduce events | **Aligned** |
| `harness/session` — JSONL tree store | `epsilon-session` — JSONL tree + offset index + leases + repair | **Aligned** (index and leases are deliberate improvements over pi) |
| `harness/compaction` | `epsilon-core/src/compact.rs` | **Aligned**, plus opencode/Hermes enhancements (template, clamp, amend) |
| `harness/skills.ts` | `epsilon-core/src/skills.rs` | **Aligned** (Agent Skills spec both sides) |
| `harness/system-prompt.ts` | `epsilon-core/src/prompt.rs` | **Aligned in content**; Epsilon uses three byte-stable tiers (Hermes discipline) instead of string concat — deliberate ADR-008 improvement |
| `harness/tools/{read,write,edit,bash}` + file-mutation queue | `epsilon-tools` read/write/edit/bash | **Mostly aligned**; see gap G4 |
| `coding-agent/src/core/tools/{grep,find,ls}` | `epsilon-tools` grep/glob/ls | **Aligned** (find ≈ glob) |
| `packages/tui` — own TUI lib (components, kill-ring, undo, autocomplete, fuzzy) | `epsilon-tui` on ratatui — editor ops, kill-ring, undo rail, slash autocomplete, palette, fuzzy matcher | **Aligned at behavior level** (M15–M17); different rendering substrate by design |
| `modes/{interactive,rpc,print}` | TUI / `--rpc` / `-p` / `--serve` | **Aligned**; serve mode is additive beyond pi |
| `packages/protocol` + `client` (RPC wire) | covered by NDJSON frames in server/rpc modes | **Aligned informally** — see gap G5 |
| `extensions` (jiti same-process, wide API incl. UI takeover) | `epsilon-ext` process-isolated NDJSON-RPC | **Deliberately divergent** (ADR-009): isolation chosen over API breadth; documented trade-off |
| `env/nodejs.ts` (runtime env probing for tools) | none | Gap **G6** (small) |
| `search/scanning.ts` — search across stored sessions | none | Gap **G1** (small, real) |
| `evals` package — scenario eval suite | none (golden-file loop tests only) | Gap **G2** (deferred with rationale below) |
| `telemetry` package (memory/noop sinks) | none (no tracing crate either) | Gap **G3** (see honesty note below) |
| OAuth flows (Anthropic/Copilot/Codex/Kimi/xAI), Bedrock, images API | none — env-var keys only | Documented v1 exclusion; unchanged |

## 2. Gaps found this pass

### G1 — Session search (close: cheap, real user value)
pi lets frontends scan stored sessions (paginated, abortable, typed hits). Epsilon's
JSONL is greppable externally but nothing in-crate searches it. Plan: a
`SessionStore::search(query, limit)` returning `(seq, snippet)` pairs over message
entries — ~80 lines plus tests, mirrors pi's scanning contract without its async
iterator machinery.

### G2 — Eval scenarios (defer)
pi's evals run scripted prompts against live models. Our equivalent discipline
(golden-file transcripts vs the faux provider) already covers regression safety;
paid-API evals violate the testing philosophy (AGENTS.md). Revisit only alongside an
offline eval-harness milestone.

### G3 — Telemetry/tracing (fix the DOCS first)
The architecture doc promises "structured tracing spans (tracing crate), opt-in OTLP"
and §13 reliability claims — **no tracing crate exists in the workspace**; observability
today = event broadcast + session log. Honest resolution: correct the architecture doc
to describe shipped reality (event bus as the observability surface; tracing crate
remains future work), rather than shipping a token integration to satisfy an old
paragraph. Recorded here so the deferral is written, not silent.

### G4 — Edit-tool mutation queue (correct the docs)
Architecture §4 claims edit has "diff preview + file-mutation queue serialization".
Code truth: serialization comes from the executor's sequential-barrier rule
(`ExecutionMode::Sequential`), which covers edit/write/bash uniformly; there is no
edit-specific diff preview. Docs corrected in this pass; behavior matches pi's
sequential-degradation semantics already.

### G5 — Protocol crate split (defer, documented)
pi factors its RPC wire into `pi-protocol`. Epsilon embeds frame shapes in
server/cli code with drift-guard tests. At seven routes the split buys nothing;
revisit if a second consumer of the wire appears (brick-earns-existence).

### G6 — Env probing module (absorb, don't port)
pi's `harness/env` centralizes runtime detection (node version etc.). Epsilon's
equivalent concerns are tiny and already localized (catalog env-key checks, sandbox
availability probes). No action.

## 3. Conclusion

Post-M17, Epsilon tracks pi's architecture layer-for-layer at the semantics level,
with three deliberate divergences (Rust crates vs TS packages, process-isolated
extensions, tiered prompts) each carrying an ADR. Remaining actionable gap: **G1
session search** (implemented in this pass). Everything else ships as a documented
deferral or a documentation correction.
