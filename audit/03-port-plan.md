# 03 — Pi port fidelity

**Audience:** the owner deciding what “complete port” means, and engineers writing conformance tests.
**Reference:** bundled `references/pi-0.84.2/`; current Rust working tree over `e200815`, 2026-09-16.

## Scope must be explicit

The directory called `packages/agent` is `@earendil-works/pi-agent-core` (`references/pi-0.84.2/packages/agent/package.json:2–4`). The usable coding product also spans `packages/coding-agent`, `packages/ai` and `packages/tui`. The snapshot additionally contains client/server/protocol/session-backend/evaluation/telemetry packages. Porting its loop is not porting the entire product or monorepo.

**Proposed target for review:** behavioral parity with the implemented agent-core and coding-agent workflows in this snapshot, supported by a capability matrix and conformance tests, with explicitly accepted Rust safety/UI/extension deviations. This preserves the original full-port goal without pretending that TypeScript ABI compatibility is required. It is a proposal, not a unilateral scope reduction.

Pi itself contains incomplete scaffolding: `references/pi-0.84.2/packages/agent/src/harness/agent-harness.ts:347–420` rejects several operations as unavailable. Its exported types are not proof that every advertised durability feature works. Compare the actual loop and coding-agent `AgentSession`, not only that scaffold.

## Classification

- **Implemented:** the named mechanism exists on both sides; no claim of identical edge cases.
- **Partial:** concrete coverage exists, but a stated behavior/API is missing or different.
- **Missing:** targeted source searches and the inspected dispatch surface found no implementation.
- **Intentional:** supported by an ADR/milestone. Otherwise intent is **unknown**; an omission is not automatically accidental.
- All comparisons below are **source-verified**, not cross-language runtime conformance results. Existing Epsilon failures are in [02](02-defect-ledger.md).

> **WARNING (2026-09-26) — the verification above is stale for this file.** The pi reference was
> updated to `pi-0.85.1` and `references/pi-0.84.2/` **no longer exists**, so the 54 path-bearing
> citations below could not be checked until the checker was written. All 54 are now confirmed
> *checkable* (file exists, line range in bounds) — **checkable is not verified**: a version bump
> shifts line numbers, and an in-bounds citation can point at unrelated code. Every row still
> needs its cited lines read. See
> [`evidence/reference-citations-point-at-a-missing-tree.md`](evidence/reference-citations-point-at-a-missing-tree.md)
> and `python scripts/check-reference-citations.py`.

For compactness, pi paths below start with `references/pi-0.84.2/`; Rust paths start at the repository root.

## Capability matrix

| Capability | Pi evidence | Rust evidence | State and intent |
|---|---|---|---|
| Two-level loop and queue drain points | `references/pi-0.84.2/packages/agent/src/agent-loop.ts:166–174,259–267` | `crates/epsilon-core/src/lib.rs:1037–1065,1208–1261` | Implemented. Steering is checked before response/after completed batch; follow-ups at stop. Neither cited path interrupts remaining batch work merely on steering arrival. |
| Queue delivery defaults | `references/pi-0.84.2/packages/agent/src/agent.ts:231–232` | `crates/epsilon-core/src/lib.rs:193–197` | Partial: pi defaults both to one-at-a-time; Rust drains all follow-ups by default. Intent unknown; test explicit modes and document choice. |
| Embeddable context and next-turn callbacks | `references/pi-0.84.2/packages/agent/src/agent-loop.ts:232–257,288–295` | `crates/epsilon-core/src/lib.rs:1588–1665`; `crates/epsilon-contracts/src/tool.rs:182–212` | Partial: Rust tool hooks exist, but not the corresponding configurable transform/conversion/prepare/stop hooks. Built-in guards do not replace an SDK contract. ADR-002 explicitly promises next-turn/stop callback parity (`docs/design/epsilon-decisions.md:36–40`): this is unmet documented intent, not merely unknown scope. |
| Tool scheduling and token-limit rejection | `references/pi-0.84.2/packages/agent/src/agent-loop.ts:208–214,418–425,540–548` | `crates/epsilon-core/src/exec.rs:183–256,540–606` | Intentionally different scheduler: bounded conflict groups with model-order commitment. Keep it. Explicit max-token calls rejected; production EOF fidelity still needs F03. Decision: `docs/milestones/M72-batch-planner.md:13–48`. |
| Basic coding tools | `references/pi-0.84.2/packages/coding-agent/src/core/tools/index.ts:83–84,186–195` | `crates/epsilon-tools/src/lib.rs:109–138` | All seven pi tool names implemented, plus extras. Schema/output/error/limit parity is partial, not established by inventory; large-file safety defect F01 blocks confidence. |
| Durable session trees and format | `references/pi-0.84.2/packages/agent/src/harness/session/types.ts:14–74`; `references/pi-0.84.2/packages/coding-agent/src/core/session-manager.ts:69–153` | `crates/epsilon-contracts/src/session_entry.rs:90–153`; `crates/epsilon-session/src/store.rs:594–749` | Partial. Pi agent-core and coding-agent themselves expose different entry schemas. Rust's retained-tail model is not proven compatible with coding-agent JSONL/custom/label/session-info entries. |
| Cross-branch navigation, branch summaries, fork | `references/pi-0.84.2/packages/coding-agent/src/core/agent-session.ts:2905–2935,2993–3068` | `crates/epsilon-tui/src/app.rs:1446–1477,1594–1608`; `crates/epsilon-session/src/store.rs:724–745` | Partial: current tree picker is active-branch-only; fork copies the file without switching. **Re-verified 2026-09-26 against `pi-0.85.1`:** the producer is `AgentSession.navigateTo(targetId, { summarize, ... })` (`agent-session.ts:3103-3112`), and it has **no caller in pi's own source** — a repo-wide `navigateTo` grep in `packages/` returns 5 hits, all an unrelated cursor option in `tui/src/components/settings-list.ts`. pi bills this call (`summaryUsage = result.usage`, `agent-session.ts:3205-3225`, passed as the 5th arg to `branchWithSummary`), so the capability exists but is unreferenced there too. Epsilon has **no** `generate_branch_summary` and **no** `rewind|navigate_back|edit_previous|switch_branch` path in tui/cli/core, and all 7 `branch_with_summary` call sites are tests. **The stale half of this row is corrected:** model-view projection does **not** ignore `BranchSummary` — `store.rs:1622-1635` replicates pi's truthiness guard verbatim (`entry.type === "branch_summary" && entry.summary`), which matters because Epsilon shares pi's session file format and an imported pi session can contain these entries. So this is a read path with **no writer yet**: a missing feature, not dead code, and not a billing defect (F22 row 6 withdrawn). Intent still unknown for whether Epsilon wants tree navigation at all. |
| Compaction and pressure accounting | `references/pi-0.84.2/packages/agent/src/harness/compaction/compaction.ts:88–120` | `crates/epsilon-core/src/compact.rs:21–87,113–150`; `crates/epsilon-core/src/lib.rs:1435–1512` | Intentionally different prune/retained-tail/amended-summary procedure (`docs/design/epsilon-decisions.md:203–220`). Usage anchor and legal cuts exist; F04/F06/F07 remain correctness bugs, not accepted divergences. |
| Provider protocols and authentication | `references/pi-0.84.2/packages/ai/src/providers/all.ts:88–145`; `references/pi-0.84.2/packages/ai/src/api/openai-codex-responses.ts:45–57` | `crates/epsilon-llm/src/providers/mod.rs:88–193`; `crates/epsilon-llm/src/oauth_login.rs:139–222` | Partial. Multiple native wires/OAuth flows exist. Generic OpenAI Responses or a catalog row does not supply Codex's subscription transport. Dedicated Azure/Cloudflare/Radius/image-generation parity was not found. Exhaustive wire fidelity unverified. |
| Extension lifecycle and SDK | `references/pi-0.84.2/packages/coding-agent/src/core/extensions/types.ts:1198–1346,1417–1433` | `crates/epsilon-ext/src/manifest.rs:18–37`; `crates/epsilon-ext/src/host.rs:130–186,244–263` | Partial. Rust supplies process tools/events, not pi commands/shortcuts/provider registration, persistent extension entries or equivalent UI interactions. Isolation intentional under `docs/design/epsilon-decisions.md:246–263`; full SDK omission not automatically justified by isolation. |
| RPC embedding | `references/pi-0.84.2/packages/coding-agent/src/modes/rpc/rpc-types.ts:20–73` | `crates/epsilon-cli/src/rpc_mode.rs:16–35,75–120` | Partial and incompatible envelope. Six operations; no abort, state/history/model/compaction/session management or request ID. F13 is an independent correctness bug. |
| Resource loading/reload/package lifecycle | `references/pi-0.84.2/packages/coding-agent/src/core/resource-loader.ts:39–50,70–155`; `references/pi-0.84.2/packages/coding-agent/src/core/package-manager.ts:978–1049` | `crates/epsilon-cli/src/runtime.rs:564–608`; `crates/epsilon-tui/src/app.rs:1805–1845` | Partial loading, missing unified resource/package lifecycle. Rust discovers skill/agent/extension roots but no corresponding npm/git resource install/update/remove implementation was found. Trust detection of a file does not prove it is loaded. |
| Markdown prompt templates | `references/pi-0.84.2/packages/coding-agent/src/core/prompt-templates.ts:57–102,177–194` | `crates/epsilon-core/src/prompt.rs:88–149`; `crates/epsilon-tui/src/app.rs:1805–1845` | Missing named templates/argument expansion. Compaction templates and credential interpolation are different features. Intent unknown. |
| Skill discovery/frontmatter/collisions | `references/pi-0.84.2/packages/coding-agent/src/core/skills.ts:168–173,256–285,399–432` | `crates/epsilon-core/src/skills.rs:14–56,63–126` | Partial. Rust's shallow scan/flat frontmatter differ; later-root replacement differs from pi first-name retention with diagnostics. Comments claiming exact collision parity are not evidence of it. |
| Configuration breadth | `references/pi-0.84.2/packages/coding-agent/src/core/settings-manager.ts:12–120` | `crates/epsilon-core/src/config.rs:20–98,148–169`; `crates/epsilon-cli/src/runtime.rs:29–39` | Partial settings; TOML/declarative config intentional (`docs/design/epsilon-decisions.md:386–402`). Queue/retry/resource/theme configuration breadth still needs explicit disposition. |
| TUI/editor/themes | `references/pi-0.84.2/packages/tui/src/index.ts:13–46,116–138` | `crates/epsilon-tui/src/lib.rs:12–30`; `crates/epsilon-tui/src/editor_ops.rs:19–73`; `crates/epsilon-tui/src/theme.rs:1–25` | Partial, with useful editor primitives already implemented. Ratatui intentional (`docs/design/epsilon-decisions.md:352–354`); complete pi component, theme or key-by-key behavior parity not established. |
| Image input/processing/rendering | `references/pi-0.85.1/packages/coding-agent/src/core/tools/read.ts:41–57` (image MIME option, `autoResizeImages`); `references/pi-0.85.1/packages/coding-agent/src/utils/image-process.ts:87` (`resizeImage`); `references/pi-0.85.1/packages/coding-agent/src/utils/image-resize.ts`; `references/pi-0.84.2/packages/tui/src/terminal-image.ts:68–105` | `crates/epsilon-tools/src/files.rs:110–142`; `crates/epsilon-core/src/references.rs:9–35,69–85`; `crates/epsilon-tui/src/state.rs:38–48` | Partial: read emits supported image bytes; no equivalent resize pipeline or terminal graphics established. `@` text attachment handling is not pi CLI image processing. |

## Intentional versus accidental

**Confirmed design choices:** Rust/ratatui rendering, process-isolated extensions, declarative TOML, explicit permission/confinement layers, bounded conflict-aware scheduling, amended structured compaction. Preserve these unless the owner explicitly revises their ADRs.

**Implementation defects contradicting their own goals:** capped edit writeback, observer sharing, repeated-summary accumulation, passing-check misclassification and lost abort persistence. These have source traces in 02. They are not reasons to copy pi wholesale; repair Epsilon's own contracts.

**Documented commitment not fulfilled:** ADR-002 promises next-turn/stop callback semantics. Reaffirm that contract or amend the ADR explicitly; the reason for omission is unknown, not the stated target.

**Unknown intent:** smaller RPC surface, absent template/package functionality, queue-default mismatch, shallow skill parsing and incomplete branch workflows. Treat these as pending scope decisions. Do not label every absent feature “deliberately deferred” without a recorded decision.

## How to measure completion

1. Freeze this reference snapshot and enumerate public operations and user workflows, including schemas, defaults and error behavior.
2. For each row, mark **conformant**, **accepted divergence**, **partial**, or **not supported**, with an owner and a test link. An accepted divergence remains visible; it does not become invisible “100% parity.”
3. Build deterministic scenario fixtures: queues, truncated calls, two compactions/resume, branch navigation, resource precedence, extension cancellation, and RPC request/event ordering.
4. Test Epsilon's assembled runtime, not only helpers. Where possible run equivalent fixtures against implemented pi code; do not test unavailable scaffold methods as if they were complete.
5. Inventory provider protocol/auth/options separately from catalog rows. Use local fixtures for framing/error/refresh and explicitly approved live calls for final compatibility.
6. Define whether importing existing pi sessions/extensions is required. If yes, budget schema translation and an isolated SDK bridge explicitly; neither is supplied by a familiar command name.

## Parity work after reliability hardening

| Workstream | Rough effort | Dependencies / stop condition |
|---|---|---|
| Core callback/default/schema conformance inventory | 3–6 days | Stop if target means entire pi monorepo rather than agent-core/coding-agent; rescope first |
| Correlated RPC, abort/state/model/history/compaction | 4–8 days | F13 and stable session APIs; pi-compatible facade versus versioned Epsilon protocol decision |
| True new/fork/all-tree navigation and branch-summary replay | 6–12 days | F04/F07/F08, durable identity and branch fixtures |
| Local resources/templates/richer skill parsing/settings | 10–20 days | Explicit precedence/trust/reload contract; do local resources before installs |
| Bounded npm/git resource package management | 5–10 additional days | Local resource model and trusted install policy; not required for initial reliable supervised coding |
| Substantial isolated extension SDK subset | 20–40 days | Protocol/lifecycle and resource foundation; arbitrary in-process pi components are outside estimate |
| Missing native provider/auth tranche | 10–25 days after 2–4-day inventory | F03/F16 and real transport fixtures; credentials/terms/platform access can block validation |
| Image/theme/editor parity | Requires separate scenario inventory | Defer graphics until bounded image input and essential keyboard/session workflows are reliable |

These are planning ranges, not an additive promise of total completion. A reliable supervised coding release can precede full pi parity; it must not be marketed as a completed port. See [04](04-roadmap.md) for the actual implementation order.
