# M7 — Compaction engine + three-tier byte-stable prompt builder

Date: 2026-08-23 · Status: complete

## What was built

**`epsilon-core/src/prompt.rs`** (ADR-008):
- Three tiers in fixed order: STABLE (embedded agent contract via `include_str!` + active-tool
  lines + guidelines) → CONTEXT (`AGENTS.md`/`CLAUDE.md` project instructions, first-match-wins,
  FNV content hash per block for observable cache invalidation) → VOLATILE (cwd/model/date).
- Determinism enforced: byte-stability test, tier-ordering test, stable-prefix-length test
  (CONTEXT changes never disturb the STABLE prefix bytes).

**`epsilon-core/src/compact.rs`** (ADR-007):
- Token estimator (chars/4 heuristic; images = flat 512) — stable and monotone, good enough
  for threshold math.
- Trigger #1 **pressure**: before each provider turn, `estimate_messages(history) > threshold`
  → compact. Configurable via `LoopLimits.compaction_threshold_tokens/_tail_tokens`.
- Procedure: model-free prune of old non-error tool results (tombstoned, provider-shape-safe)
  → protected tail clamp [2k,15k] → strict-template summary request with clipped tool outputs
  → **amend-mode** when a prior `<compaction_summary>` exists in the compacted range.
- Trigger #2 **overflow recovery**: a provider Error whose message matches context-window
  keywords pops the failed attempt, compacts once (`overflow_retried` flag bounds it), and
  retries the turn.
- **Durable mirror**: `Agent::attach_session_store` persists every transcript message, appends
  `Compaction` entries, and switches the provider-visible context to the store's SHADOWED
  MODEL VIEW — the human JSONL keeps everything while the model sees summaries + tail.

## Design decision made while coding

**Dedicated summarizer stream (`set_summarizer`).** The first implementation reused the main
provider for summary calls; under faux scripting that interleaved unpredictably with loop
calls, and in production it would couple aux traffic to main-traffic caching/rate-limits.
pi isolates auxiliary calls for cache reasons — we now isolate them architecturally too:
`summarizer_stream()` falls back to the main provider but tests (and M9+ model routing) inject
a dedicated one.

## Test-caught bugs

1. **Tools ran against process cwd instead of the declared workspace** (caught by the env-read
   scenario): `run_one` built `ToolContext.cwd` from `std::env::current_dir()` while gating
   used `workspace_root`. Fixed to a single source of truth.
2. **Broadcast flood**: one `MessageUpdate` per streamed event meant ~670 updates per big
   tool-call (args deltas every 6 chars), overflowing subscriber rings (1024) — receivers went
   `Lagged` and silently lost events. Fix: argument-fragment deltas fold locally without
   broadcast (zero UI value); visible text/thinking still streams live. Test drains are now
   also Lagged-tolerant.

## Deliberately deferred

- Real BPE token counting (estimator pluggable later), cacheRetention knobs on the genai
  adapter for true prefix-cache isolation of summary calls, AGENTS.md subdirectory discovery.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 63 passed across 10 suites
cargo run -p epsilon-cli                # demo unaffected, still green
```

New suites: prompt units (5) · compaction scenarios (2: pressure+persist+shadowing,
overflow-recover-once).
