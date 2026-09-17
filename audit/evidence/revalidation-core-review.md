# Core/session/RPC pi fidelity revalidation

## Scope and evidence discipline

Read-only source review, 2026-09-17. Reference is the **bundled** `references/pi-0.84.2`, not upstream. Epsilon HEAD was verified as `80b32eb1e76a2fa9084ad1e69b300c0800186cf4`. Read and confirmed `AGENTS.md`, `agents/memory/README.md`, relevant `next-hardening.md`, and `.agents/skills/improve/references/audit-playbook.md` sections **1, 4, 8 and Finding format**. `audit/02-defect-ledger.md` and `audit/03-port-plan.md` were treated as allegations and checked against implementation.

**No builds, tests, installs, runtime probes, commits, or source edits were performed.** Acceptance scenarios below are proposed, not executed conformance results. Only this report was authored. The existing dirty tree and parent's build were left untouched. Read-only Git/source inspection is the evidence; this is not a green-baseline or complete compatibility certification.

Paths are repository-relative; `pi/` below abbreviates `references/pi-0.84.2/`. Unqualified Rust line references are working-tree lines. `HEAD:` explicitly denotes `git show HEAD:path` line numbers. Git diff confirmed that core `lib.rs`, `compact.rs`, session `store.rs`, and contracts `tool.rs`/`session_entry.rs` have no HEAD-to-working-tree changes relevant to these findings. RPC differs only in shared `@`-reference message construction; it does not repair admission or expand the protocol. TUI differs substantially and is distinguished below.

## Findings (8)

### [CORE-01] Restore the one-at-a-time follow-up default

- **Classification:** confirmed — parity gap; intent not established.
- **Evidence:** `pi/packages/agent/src/agent.ts:231–232` constructs both queues with `one-at-a-time` by default. `crates/epsilon-core/src/lib.rs:190–197` defaults steering to `OneAtATime` but follow-ups to `All`; `:133–136` implements those genuinely different drains, and `:1262–1267` consumes follow-ups at the stop boundary. Identical at HEAD.
- **Impact:** two queued follow-ups enter the same next model request in Epsilon, instead of letting the first complete before the second is delivered. This changes task ordering and context, not just configuration spelling.
- **Acceptance:** queue two distinguishable follow-ups before the original run settles; with defaults, capture provider contexts and require the second to remain absent until work triggered by the first would stop. Also characterize explicit `All` and steering priority.
- **Effort:** S. **Risk:** LOW — changed default may affect existing embeddings expecting batch delivery.
- **Confidence:** HIGH, source-confirmed; runtime conformance unexecuted.
- **Fix sketch:** use pi's default or explicitly approve/document a divergence; keep both explicit modes and add assembled-loop context assertions.

### [CORE-02] Implement the promised next-turn and context callback seams

- **Classification:** confirmed — missing SDK behavior and documented-intent mismatch.
- **Evidence:** `pi/packages/agent/src/agent-loop.ts:232–257` awaits `prepareNextTurn`, applies its context/model/thinking snapshot, then evaluates `shouldStopAfterTurn` before queue polling; `:288–295` invokes transform then conversion at the LLM boundary. `crates/epsilon-core/src/lib.rs:1202–1267` goes from TurnEnd into built-in guard/queue handling without those callbacks; `:1598–1611,1666–1671` selects history then uses fixed `Self::convert_to_llm`. `crates/epsilon-contracts/src/tool.rs:182–204` exposes tool interception, not these callbacks. `docs/design/epsilon-decisions.md:44–48` explicitly promises prepare/stop parity. Targeted Rust searches found neither callback name.
- **Impact:** an embedder cannot atomically replace next-turn context/model/thinking at pi's seam, perform configurable context conversion, or veto the next turn before queued work is consumed. Tool hooks and built-in nudges are not equivalents.
- **Acceptance:** after one tool turn, prepare a replacement context/model/thinking snapshot; assert the next request uses it. A stop callback must observe the prepared context and prevent that request and follow-up consumption. Assert transform-before-conversion ordering without rewriting durable history.
- **Effort:** M–L. **Risk:** MED — callback ordering, ownership and reentrancy touch the loop contract.
- **Confidence:** HIGH for absent seams in the inspected implementation; no executed cross-language fixture.
- **Fix sketch:** add explicit object-safe callback contracts at matching boundaries, including cancellation/error policy. Preserve the accepted bounded scheduler and built-in guards; define their ordering rather than silently substituting them.

### [CORE-03] Replace superseded summaries in the durable model projection

- **Classification:** confirmed — correctness defect; F04 remains current.
- **Evidence:** `pi/packages/coding-agent/src/core/session-manager.ts:418–453` selects the latest compaction on the active path and builds context from it plus its retained entries. `crates/epsilon-session/src/store.rs:717–749` pushes every compaction summary/tail into `out` and clears only `pending`. `crates/epsilon-core/src/lib.rs:1490–1518` replaces runtime history with only the new summary/tail, but `:1598–1608` prefers the store projection for actual provider requests. Same at HEAD.
- **Impact:** a second compaction yields older summary/tail plus newer summary/tail in the durable projection, while runtime history has only the newer pair. Superseded material reappears after compaction and reopen; physical log retention itself is not the defect.
- **Acceptance:** with an attached store, compact twice using distinguishable summaries/tails; compare exact provider context with the intended latest projection, reopen, and compare again. Include sibling branches; retain the entire human log.
- **Effort:** M. **Risk:** MED — branch-local cut semantics and migration expectations need fixtures.
- **Confidence:** HIGH, direct projection trace.
- **Fix sketch:** make a later compaction replace the superseded model projection rather than append to it, preserving Epsilon's accepted retained-tail format. Do not delete historical log entries to solve a projection bug.

### [CORE-04] Reject failed summarization and check cancellation before committing

- **Classification:** confirmed — correctness/parity defect; F06 remains current, with an empty-answer qualification.
- **Evidence:** implemented pi coding-agent summarization rejects `stopReason === "error"` at `pi/packages/coding-agent/src/core/compaction/compaction.ts:670–685`; its caller passes a signal and checks cancellation before append at `pi/packages/coding-agent/src/core/agent-session.ts:1854–1881`. Epsilon accepts any `Done` without testing stop reason or selecting on cancellation at `crates/epsilon-core/src/lib.rs:1474–1484`, then commits at `:1502–1518`. `crates/epsilon-core/src/compact.rs:241–248` substitutes a template for empty text. Same at HEAD.
- **Impact:** a failed `Done` may replace useful context with failure prose or a placeholder. Cancellation during summary connect/polling does not prevent a subsequent commit. Early EOF is already rejected and should not be reported as accepted.
- **Acceptance:** error/aborted Done, empty answer, early EOF, cancelled connection and cancelled stream must leave pre-compaction memory/store context unchanged and produce a truthful outcome; a successful summary must still commit at a legal cut.
- **Effort:** M. **Risk:** MED — preserve good context and consistent terminal events across all exits.
- **Confidence:** HIGH for error/cancellation control flow. Empty-result preservation is an Epsilon reliability requirement, **not established pi parity**: pi's inspected helper also returns extracted text without a nonempty check.
- **Fix sketch:** validate the summary outcome and cancellation immediately before durable/model replacement; make generation cancellation-aware and surface failure rather than replacing context with a template.

### [CORE-05] Persist locally synthesized abort output and close its message lifecycle

- **Classification:** confirmed — correctness/parity defect; F17 remains current.
- **Evidence:** `pi/packages/agent/src/agent-loop.ts:346–370` retains terminal responses (including errors) and emits `message_end`; `pi/packages/coding-agent/src/core/agent-session.ts:639–656` persists assistant message-end events. Epsilon's local cancellation arms return directly at `crates/epsilon-core/src/lib.rs:1686–1709`; normal Done persists/emits at `:1735–1739`, and EOF salvage at `:1778–1788`. `crates/epsilon-core/tests/loop_scenarios.rs:775–790` checks returned partial text, not attached-store/reopened history. Same at HEAD.
- **Impact:** visible partial text can exist in RunOutcome while missing from runtime and durable history, and subscribers do not receive its normal MessageEnd. A resumed conversation loses the interrupted answer.
- **Acceptance:** abort during connect and after a controlled text delta with a real attached store; assert one terminal assistant message, one MessageEnd, no incomplete call execution, and the same partial/error record after reopen.
- **Effort:** S–M. **Risk:** MED — avoid double persistence when provider Done races cancellation; account for durable append failure.
- **Confidence:** HIGH, direct return-vs-commit trace.
- **Fix sketch:** route synthetic cancellation through the common terminal-message commit/lifecycle path, retaining the existing removal of unfinished tool calls.

### [RPC-01] Make prompt acknowledgment reflect actual admission

- **Classification:** confirmed — correctness/parity defect; RPC portion of F13 remains current.
- **Evidence:** `pi/packages/coding-agent/src/modes/rpc/rpc-mode.ts:394–415` emits success only through prompt preflight success and returns correlated preflight errors. `crates/epsilon-cli/src/rpc_mode.rs:77–85,117–120` always passes the busy check, spawns a run, discards its result, and replies `started`. `crates/epsilon-core/src/lib.rs:906–909` rejects a concurrent run before event emission. HEAD has the same logic at `HEAD:crates/epsilon-cli/src/rpc_mode.rs:82–90,122–125`; the working-tree delta only changes message construction.
- **Impact:** a second request while a run is active receives success for rejected work, with no rejection event to settle it. The helper comment claiming AlreadyRunning is reported through events is contradicted by the early return.
- **Acceptance:** hold one admitted run active and submit another; exactly one admission succeeds, the other receives attributable rejection. Assert no false success or orphaned waiter, including preflight failure. No timing-only busy-check substitution.
- **Effort:** M. **Risk:** MED — request/event ordering and admission release need explicit ownership.
- **Confidence:** HIGH, direct dispatch trace; concurrency scenario not executed.
- **Fix sketch:** reserve admission atomically before success and carry preflight/run rejection back to the requester. Do not replace the constant helper with a racy check-then-spawn alone.

### [RPC-02] Expose a correlated protocol and implemented session controls

- **Classification:** confirmed — incompatible/missing public surface; acceptance of divergence unknown.
- **Evidence:** pi defines ID-bearing `type` requests and response envelopes at `pi/packages/coding-agent/src/modes/rpc/rpc-types.ts:20–73,115–124`. These are not just scaffold declarations: `pi/packages/coding-agent/src/modes/rpc/rpc-mode.ts:428–461,468–488,517–538` implements abort, new session, state, model, queue-mode and compaction operations. Epsilon's exhaustive `Op` enum at `crates/epsilon-cli/src/rpc_mode.rs:16–35` contains only prompt/steer/followup/approvals/approve/ping using `op`; replies at `:123–131` have no command/request correlation. Enum identical at HEAD; HEAD reply helpers begin at `:128`.
- **Impact:** a pi RPC client cannot use Epsilon unchanged; an editor cannot query state, change model, compact or abort through this transport even though relevant core facilities exist. Serialization-by-arrival is not pi's ID contract.
- **Acceptance:** protocol fixtures cover request IDs, success/error envelopes and interleaved asynchronous events; exercise abort/state/model/queue-mode/compaction/session operations against the composed runtime. Unsupported operations must fail explicitly.
- **Effort:** L. **Risk:** MED — backward compatibility for current Epsilon clients and lifecycle operations.
- **Confidence:** HIGH for listed dispatch/schema differences; exhaustive RPC parity remains unverified.
- **Fix sketch:** choose a pi-compatible facade or an explicitly versioned Epsilon protocol, implement the selected operation matrix and retain existing permission operations as extensions.

### [SESSION-01] Complete all-tree navigation, branch-summary replay and fork replacement

- **Classification:** confirmed — incomplete workflow, with significant HEAD/working-tree distinction.
- **Evidence:** `pi/packages/coding-agent/src/core/agent-session.ts:2925–2935,3045–3078` resolves arbitrary tree entries, can attach a branch summary at the target and rebuilds agent context; `pi/packages/coding-agent/src/core/session-manager.ts:401–403` projects branch summaries. Actual fork implementation is in **`pi/packages/coding-agent/src/core/agent-session-runtime.ts:262–286,317–332`**, selecting a cut and replacing the runtime, not the old plan's implied AgentSession method. Epsilon WT `/tree` uses only `branch_entries()` at `crates/epsilon-tui/src/app.rs:1594–1608`; `crates/epsilon-session/src/store.rs:724–745` ignores BranchSummary, whose only Rust occurrence is its declaration at `crates/epsilon-contracts/src/session_entry.rs:132–138`. WT fork only copies the full file at `crates/epsilon-tui/src/app.rs:1446–1477`.
- **HEAD versus WT:** HEAD has **no `/tree` parser arm** (`HEAD:crates/epsilon-tui/src/picker.rs:344–350`); its timeline is an active-branch notice listing (`HEAD:crates/epsilon-tui/src/app.rs:846–858`), not the WT picker. Fork is already copy-without-switch in HEAD (`HEAD:crates/epsilon-tui/src/app.rs:911–942`). Store `navigate_to` can select an arbitrary indexed entry (`crates/epsilon-session/src/store.rs:697–707`); do not report the underlying store as incapable of cross-branch selection.
- **Impact:** after branching, the frontend cannot choose an abandoned sibling through the tree picker; branch-summary vocabulary has no producer/replay behavior; fork keeps writing the original session and copies unrelated branches instead of forking at a selected node.
- **Acceptance:** create two branches, select either sibling from the all-tree UI, optionally summarize abandoned work and verify it appears exactly once after reopen. Fork at a chosen user boundary; assert fresh identity, selected ancestry only, restored editor text and subsequent writes to the new session.
- **Effort:** L. **Risk:** HIGH — durable identity, selection semantics, context rebuilding and runtime switching must agree.
- **Confidence:** HIGH for inspected paths/absence of BranchSummary consumers; assembled frontend acceptance unexecuted.
- **Fix sketch:** expose all indexed nodes to tree selection; implement branch-summary creation/projection and a transactional fork/switch workflow. Preserve historical branches instead of deleting them.

## Corrections and limits

- The core **two-level loop and queue drain points are implemented** on both sides (`pi/packages/agent/src/agent-loop.ts:166–174,259–267`; Rust `lib.rs:1043–1049,1214,1262–1267`). Neither inspected scheduler interrupts the remaining batch merely because steering arrives. Do not turn this into a missing-loop finding.
- Both explicit token-limit paths refuse the calls (`pi/.../agent-loop.ts:208–214`; Rust `lib.rs:1131–1136`). Provider EOF fidelity is a separate review; no new finding is inferred from the scheduler here.
- The old plan's unqualified “current tree picker” description is **stale for HEAD**, although accurate for the inherited WT. Its cited fork location on the pi side needs updating to `agent-session-runtime.ts`.
- Error/cancellation rejection is supported by the implemented pi coding-agent flow, not merely the agent-harness scaffold. Conversely, pi empty-summary rejection was **not** established and must not be claimed.
- Durable append failure behavior (F07), schema/tail repair (F08), pressure-accounting equivalence, awaitable event-subscriber parity, and complete session-file interchange remain **unverified in this bounded report**. No absence of findings here closes those items.
- Operational read hiccups (one mistyped workdir, one unsupported regex search and one nonexistent guessed filename) were corrected by read-only Git/source reads; none required changing the tree or starting a process under review.
