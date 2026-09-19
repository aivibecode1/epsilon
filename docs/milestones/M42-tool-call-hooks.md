# M42 — pi-parity: tool-call hooks (`beforeToolCall` / `afterToolCall`)

Tenth port batch against the 2026-09-03 parity audit (harness findings —
interception). Epsilon had no interception point around tool execution:
extensions could register tools and gate permissions, but couldn't observe,
veto, or rewrite calls — the three behaviors pi's `AgentLoopConfig` hooks
exist for (redaction, policy veto, summarization).

## What landed

1. **Contract** (`epsilon-contracts/src/tool.rs`): `ToolHooks` trait
   (object-safe, `Send + Sync`) with `before_tool_call` /
   `after_tool_call`, plus `BeforeToolCallContext/Result`,
   `AfterToolCallContext/Result`, and `NoopToolHooks`. Hooks return
   `Result<_, String>`: `Err` blocks (before) or replaces with an error
   result (after) — pi surfaces hook errors as error results, never loop
   panics. All types re-exported from the crate root.
2. **Pipeline** (`epsilon-core/src/exec.rs`): the before-hook runs after
   validation + permission gate (denied calls never reach hooks — they carry
   `blocked_result`), before execution; `block` becomes an error result with
   the hook's reason, `terminate` rides in `details` for the batch-all-agree
   rule. The after-hook merges field-by-field (`None` keeps, no deep merge —
   pi's exact semantics, including the `terminate` override path). Both run
   per-call, inline, on both the sequential and parallel paths (they live in
   `run_one`, which both paths share).
3. **Agent** (`epsilon-core/src/lib.rs`): `tool_hooks: RwLock<Arc<dyn
   ToolHooks>>` (default no-op), `set_tool_hooks` for hosts, `tool_hooks()`
   clone per call so a host can swap mid-run.
4. **Tests**: 3 new loop scenarios (observe names+args, block vetoes with the
   hook's reason while the run still completes, after-hook rewrites content
   without flipping `is_error`).

## Key decisions

- Pi passes the full `assistantMessage`, raw `toolCall`, and `AgentContext`
  to hooks; ours passes call id + name + validated args (+ the settled
  result for after). Deliberate narrowing: the full context would drag
  `AgentContext` (provider auth, model lists) across the contracts boundary
  the wrong way (dependency law: contracts takes no internal deps). A host
  that needs more can close over it — hooks are trait objects, not bare
  functions, precisely so they can carry state.
- No abort-signal plumbing on hooks (pi passes `AbortSignal`): our pipeline
  checks cancellation at batch boundaries and tools get the token via
  `ToolContext`; hooks are expected to be fast and non-blocking (documented
  on the trait). If a hook needs cancellation it can take its own token when
  constructed.
- `validated_args` stored alongside `invocation.args` (same value, intent
  alias): keeps hook call sites reading intent, not plumbing.

## Gotchas

- First draft used `catch_unwind` on the hook future (`catch_unwind_fut` —
  doesn't exist; invented API). Replaced with `Result<_, String>` returns:
  simpler, no new dependency, and matches how tools already report failure
  (`ToolFailure` → error result).
- `ToolCallId` is `type ToolCallId = String` — no `.into()` needed; removed
  two spurious conversions.
- One `PreparedCall` construction site (doom-loop denial) missed the new
  `validated_args` field (E0063) — the python bulk-replace covered only 3 of
  4 sites; fixed the fourth by hand.

## Verification

- `cargo test -p epsilon-core --test loop_scenarios`: 13 passed (3 new).
- `cargo fmt --check` clean, workspace clippy zero warnings.
