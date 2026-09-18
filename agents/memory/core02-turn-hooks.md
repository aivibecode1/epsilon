# CORE-02 — turn hooks (pi AgentLoopConfig parity)

## State
- **Implemented, reviewed twice, blockers closed, final battery running** (2026-09-18).
  Nothing committed yet.
- Review records: `audit/evidence/m89-review.md` (first review, two blockers),
  re-review deleg_5e7ac311 (no open blockers, all items closed, residuals recorded).
  Negative control: `audit/evidence/m89-negative-control.log` (my probes A and B failed for
  the right reasons; lib.rs restore verified by sha256 `66bd8f0402b8d986…`).
- Final candidate: `C:/Users/Hp/AppData/Local/Temp/epsilon-m89e-candidate` (= HEAD df5d3be +
  `crates/epsilon-contracts/src/turn.rs`, the two-line `crates/epsilon-contracts/src/lib.rs`
  [the working-tree copy carries another work item's re-exports — only two lines belong to this
  slice], `crates/epsilon-core/src/lib.rs`, `crates/epsilon-core/tests/loop_scenarios.rs`).
  Battery label `m89e`, `CARGO_PROFILE_TEST_DEBUG=1` (AV-dodge, disclosed),
  `CARGO_BUILD_JOBS=1`, external target `epsilon-core02-target`.
- My own focused verification on these bytes: fmt clean, clippy exit 0, 28/28 loop scenarios.
- Extra fix beyond the re-review's list: `attach_session_store`/`detach_session_store` now bump
  `history_epoch` (a provider-view source swap invalidates a run-scoped replacement); recorded
  as not-regression-tested in the milestone limits.

## The two review blockers (both fixed)
1. **Frozen replacement**: the loop never appended its own assistant messages/tool results to
   an installed replacement → the provider stopped seeing its own output (pi appends to
   `currentContext.messages`). Fixed via `Agent::append_to_turn_context` wired into the
   assistant push, tool results, steering/nudge drain, and the overflow-discard pop.
2. **Stale replacement after compaction**: a committed mid-run compaction left the
   pre-compaction replacement winning, defeating the recovery and breaking the M86/ADR-007
   store-view invariant. Fixed with `history_epoch` (bumped by `compact_now`/`restore_history`
   and by store attach/detach) checked before each request; per-run model/thinking survive.

Also fixed from the review: hook panics contained (`contain_hook`: `pin!` + `poll_fn` +
`catch_unwind`, never re-polls a panicked future); all four seams made `#[async_trait]` like
`ToolHooks` (the sync design's "ADR-014 forbids async_trait" justification was **factually
wrong** — contracts already depends on async-trait); one shared `try_context_view()`; the
usage anchor is no longer indexed through a replacement; five test-hardening items.

## Next safe actions
- On battery PASS: fill the M89 milestone verification numbers, add the `docs/history.md` row
  (test total = previous 751 + contracts' 2 turn tests + loop_scenarios' 3 extra… confirm from
  the gate log), copy the final gate log into `audit/evidence/`, then commit with the
  two-line index surgery (`git hash-object -w` the candidate's `contracts/lib.rs` +
  `git update-index --cacheinfo`) so the sibling work item's re-exports stay uncommitted.
- Then: cleanup (candidate + `epsilon-core02-target` + the two negctl copies; keep
  `epsilon-m89d-candidate`-side evidence files already copied into the repo), record the
  cleanup log, retire this note, update `agents/memory/README.md` and `port-revalidation.md`.
- No reinstall needed: no in-tree frontend installs turn hooks, so the shipped CLI's behaviour
  is unchanged (state that reasoning in the milestone doc).
- Ledger IDs live in `audit/evidence/revalidation-core-review.md:23–31` (the F-ID ledger is
  `audit/02`); roadmap keeps "SDK callbacks" open at `audit/04-roadmap.md:157`.
- ADR-002 (`docs/design/epsilon-decisions.md:44–55`) already names `should_stop_after_turn`
  and `prepare_next_turn` as one-to-one obligations; its M85 amendment records them as an
  open commitment — this slice fulfils law, not new invention.

## What the slice is (smallest faithful)
- `crates/epsilon-contracts/src/turn.rs`: object-safe `TurnHooks` with no-op defaults —
  `transform_context`, `convert_to_llm`, `prepare_next_turn` (→ `TurnUpdate{context, model,
  thinking}`, `NextTurnContext`), `should_stop_after_turn`. Mirrors `ToolHooks`
  (`crates/epsilon-contracts/src/tool.rs:182–212`), re-exported like `lib.rs:29–33`.
- `crates/epsilon-core/src/lib.rs`: `RwLock<Arc<dyn TurnHooks>>` beside `tool_hooks` (:315),
  setter beside :715–727; invoke prepare then stop immediately after `TurnEnd`/stuck notice
  (:1257–1267) and **before** `drain_steering` (:1269); a true stop exits through `agent_end`
  without draining follow-ups (:1317–1323), matching pi `agent-loop.ts:248–257`; transform →
  convert at the request-build boundary (:1743–1755 → :1813), pure reads only (ADR-007).
- Tests in `crates/epsilon-core/tests/loop_scenarios.rs` using the `RecordingHooks` pattern
  (:361–405): (a) prepare replaces context/model/thinking and request #2 reflects it while
  history is unchanged; (b) `should_stop_after_turn` + a queued follow-up ⇒ exactly one
  request; (c) transform output feeds the recorded request and `is_llm_visible` still filters.

## Constraints
- ADR-014 (contracts churn): additive only, stateless trait. ADR-007: hooks never write the
  store. ADR-002/ADR-006: hooks must not replace queue drains or LoopLimits defaults
  (`OneAtATime`, :195–199) — document the guard-vs-hook order in the milestone doc.
- No new dependencies, no `unsafe`, no new `#[allow]`; rustdoc on new public items.

## Open questions from the brief
- Whether pi wires these hooks at the coding-agent/session layer (only `packages/agent` inspected).
- pi's runtime behavior if a hook throws (only doc contracts read).
- Tool-hook invocation sites agent-loop.ts:418–425,540–548 cited from the port plan, not re-read.
- Ledger line numbers have drifted; current-tree numbers are the ones in the brief.
