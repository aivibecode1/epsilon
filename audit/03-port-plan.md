# 03 — Pi port fidelity

**Audience:** the owner deciding what “complete port” means, and engineers writing conformance tests.
**Reference:** bundled `references/pi-0.85.1/`; current Rust working tree over `9c61af7`, 2026-09-26.
**Re-verified 2026-09-26** against the updated reference: all 25 citations repointed to `pi-0.85.1` and
re-read, 0 unresolvable. What that pass changed is recorded in the warning below.

## Scope must be explicit

The directory called `packages/agent` is `@earendil-works/pi-agent-core` (`references/pi-0.85.1/packages/agent/package.json:2–4`). The usable coding product also spans `packages/coding-agent`, `packages/ai` and `packages/tui`. The snapshot additionally contains client/server/protocol/session-backend/evaluation/telemetry packages. Porting its loop is not porting the entire product or monorepo.

**Proposed target for review:** behavioral parity with the implemented agent-core and coding-agent workflows in this snapshot, supported by a capability matrix and conformance tests, with explicitly accepted Rust safety/UI/extension deviations. This preserves the original full-port goal without pretending that TypeScript ABI compatibility is required. It is a proposal, not a unilateral scope reduction.

Pi itself contains incomplete scaffolding: `references/pi-0.85.1/packages/agent/src/harness/agent-harness.ts:347–420` rejects several operations as unavailable. Its exported types are not proof that every advertised durability feature works. Compare the actual loop and coding-agent `AgentSession`, not only that scaffold.

## Classification

- **Implemented:** the named mechanism exists on both sides; no claim of identical edge cases.
- **Partial:** concrete coverage exists, but a stated behavior/API is missing or different.
- **Missing:** targeted source searches and the inspected dispatch surface found no implementation.
- **Intentional:** supported by an ADR/milestone. Otherwise intent is **unknown**; an omission is not automatically accidental.
- All comparisons below are **source-verified**, not cross-language runtime conformance results. Existing Epsilon failures are in [02](02-defect-ledger.md).

> **WARNING (2026-09-26) — the verification above is stale for this file.** The pi reference was
> updated to `pi-0.85.1` and `references/pi-0.85.1/` **no longer exists**, so the 54 path-bearing
> citations below could not be checked until the checker was written. All 54 are now confirmed
> *checkable* (file exists, line range in bounds) — **checkable is not verified**: a version bump
> shifts line numbers, and an in-bounds citation can point at unrelated code. Every row still
> needs its cited lines read. See
> [`evidence/reference-citations-point-at-a-missing-tree.md`](evidence/reference-citations-point-at-a-missing-tree.md)
> and `python scripts/check-reference-citations.py`.
>
> **Content pass done (2026-09-26).** All 55 citations were read by three read-only subagents:
> **24 SUPPORTS, 25 PARTIAL, 2 CONTRADICTS, 4 UNFINDABLE.** Every non-SUPPORTS verdict is line
> drift or a rename, not a refutation — the findings stand, their evidence did not. Four change a
> *conclusion*: pi has 8 coding tools not 7 (added `powershell`); 23 slash commands not 22 (added
> `thinking`); 224 LaTeX symbols not 217; and the harness has 1 deferred op, not "several".
> Rows 5 and 8 are rewritten below; the other PARTIAL rows still carry their old ranges. Full
> detail: [`evidence/reference-citation-content-verification.md`](evidence/reference-citation-content-verification.md).

For compactness, pi paths below start with `references/pi-0.85.1/`; Rust paths start at the repository root. **All 25 were content-checked on 2026-09-26**; see the warning above for what that pass changed.

## Capability matrix

| Capability | Pi evidence | Rust evidence | State and intent |
|---|---|---|---|
| Two-level loop and queue drain points | `references/pi-0.85.1/packages/agent/src/agent-loop.ts:166–174,257–267` | `crates/epsilon-core/src/lib.rs:1037–1065,1208–1261` | Implemented. Steering is checked before response/after completed batch; follow-ups at stop. Neither cited path interrupts remaining batch work merely on steering arrival. |
| Queue delivery defaults | `references/pi-0.85.1/packages/agent/src/agent.ts:231–232` | `crates/epsilon-core/src/lib.rs:193–197` | **Closed (M85 `3afafd3`) — corrected 2026-09-26.** This row said "Rust drains all follow-ups by default", which stopped being true: `crates/epsilon-core/src/lib.rs` now defaults **both** queues to `OneAtATime` (4 `OneAtATime` sites), matching pi's `agent.ts:231–232`. The port-plan claim of a divergence was stale, not the code. |
| Embeddable context and next-turn callbacks | `references/pi-0.85.1/packages/agent/src/agent-loop.ts:177,232–257,288–295` | `crates/epsilon-core/src/lib.rs:1588–1665`; `crates/epsilon-contracts/src/tool.rs:182–212` | **Fulfilled — corrected 2026-09-26.** This row claimed ADR-002's next-turn/stop callbacks were "unmet documented intent". `TurnHooks` implements the seams (`crates/epsilon-contracts/src/turn.rs`) and the ADR was amended (`docs/design/epsilon-decisions.md:57–73`). Note the row's own citation was also wrong: `epsilon-decisions.md:36–40` is ADR-001's closing paragraph; **ADR-002 begins at `:44`**. Built-in guards still do not replace an SDK contract, so transform/conversion breadth stays Partial. |
| Tool scheduling and token-limit rejection | `references/pi-0.85.1/packages/agent/src/agent-loop.ts:226–232,372–400,418–425,540–548` | `crates/epsilon-core/src/exec.rs:183–256,540–606` | Intentionally different scheduler: bounded conflict groups with model-order commitment. Keep it. Explicit max-token calls rejected; production EOF fidelity still needs F03. Decision: `docs/milestones/M72-batch-planner.md:13–48`. |
| Basic coding tools | `references/pi-0.85.1/packages/coding-agent/src/core/tools/index.ts:95` (`ToolName` union, 8 names), `:96–105` (`allToolNames`) | `crates/epsilon-tools/src/lib.rs:128–158` (11 tools registered); `crates/epsilon-tools/src/shell.rs:47` | **7 of pi's 8, not "all seven".** 0.85.1's union is `read \| bash \| powershell \| edit \| write \| grep \| find \| ls` — pi added `powershell` after 0.84.2, so the "seven" here was once right and is now an undercount. **The gap is a surface difference, not a missing capability:** Epsilon has one `bash` tool that *is* PowerShell on Windows (`shell.rs:47`; `shell.rs:483` asserts `program == "powershell"`), where pi exposes `bash` and `powershell` as two separately selectable tools — so on Windows an Epsilon model cannot run `bash` at all. Epsilon's extras: `glob`, `web_fetch`, `todo`, `task`, `skill`. Schema/output/error/limit parity is partial, not established by inventory. **Corrected 2026-09-26: F01 no longer blocks confidence** — the ledger records it fixed 2026-09-17, and M95 added paged reads with an explicit partial-view notice (`crates/epsilon-tools/src/util.rs:292–352`). |
| Durable session trees and format | `references/pi-0.85.1/packages/agent/src/harness/session/types.ts:14–74`; `references/pi-0.85.1/packages/coding-agent/src/core/session-manager.ts:69–153` | `crates/epsilon-contracts/src/session_entry.rs:90–153`; `crates/epsilon-session/src/store.rs:594–749` | Partial. Pi agent-core and coding-agent themselves expose different entry schemas. Rust's retained-tail model is not proven compatible with coding-agent JSONL/custom/label/session-info entries. |
| Cross-branch navigation, branch summaries, fork | `references/pi-0.85.1/packages/coding-agent/src/core/agent-session.ts:3102–3116` (`navigateTree` doc + signature), `:3201–3215` (the `options.summarize` branch), `:3250–3270` (leaf switch + `branchWithSummary`) | `crates/epsilon-tui/src/app.rs:1721–1740` (`/tree`, an alias for `/timeline`); `crates/epsilon-tui/src/picker.rs:354–355`; `crates/epsilon-session/src/store.rs:1397` (the only production `EntryKind::BranchSummary` construction), `:1622–1635` (model-view projection) | Partial: `/tree` **lists** branches via `store.branch_entries()`; it is a viewer, not a navigator, so no `BranchSummary` is ever produced. Fork copies the file without switching. The `BranchSummary` **reader** is not dead — Epsilon shares pi's session format, so an imported pi session can contain these entries, and the projection replicates pi's truthiness guard verbatim (`entry.type === "branch_summary" && entry.summary`). **Re-verified 2026-09-26; the previous citation was wrong twice over** — it pointed at `agent-session.ts:2905–2935,2993–3068`, which is retry-backoff and bash execution, and it named the method `navigateTo` when 0.85.1 calls it **`navigateTree`** (5 real callers: `interactive-mode.ts:1889,5288`, `print-mode.ts:86`, `extensions/runner.ts:827`, `agent-controller-provider.ts:60`). pi does bill this call (`summaryUsage = result.usage`), so the capability exists and Epsilon lacks it. A missing feature, not a billing defect. |
| Compaction and pressure accounting | `references/pi-0.85.1/packages/agent/src/harness/compaction/compaction.ts:245,312,536–560` | `crates/epsilon-core/src/compact.rs:21–87,113–150`; `crates/epsilon-core/src/lib.rs:1435–1512` | Intentionally different prune/retained-tail/amended-summary procedure (`docs/design/epsilon-decisions.md:203–220`). Usage anchor and legal cuts exist; F04/F06/F07 remain correctness bugs, not accepted divergences. **Corrected 2026-09-26:** the old `88–120` is `getMessageFromEntryForCompaction` (`:90`), an entry-projection helper with nothing to do with accounting. Real locations: `:245` the threshold predicate, `:312` `const cutPoints: number[] = []`, `:536/:560/:824` the summarizer calls returning `{ text, usage }`. |
| Provider protocols and authentication | `references/pi-0.85.1/packages/ai/src/providers/all.ts:88–145`; `references/pi-0.85.1/packages/ai/src/api/openai-codex-responses.ts:45–57` | `crates/epsilon-llm/src/providers/mod.rs:88–193`; `crates/epsilon-llm/src/oauth_login.rs:139–222` | Partial. Multiple native wires/OAuth flows exist. Generic OpenAI Responses or a catalog row does not supply Codex's subscription transport. Dedicated Azure/Cloudflare/Radius/image-generation parity was not found. Exhaustive wire fidelity unverified. |
| Extension lifecycle and SDK | `references/pi-0.85.1/packages/coding-agent/src/core/extensions/types.ts:1198–1346,1381,1417–1433,1486` | `crates/epsilon-ext/src/manifest.rs:18–37`; `crates/epsilon-ext/src/host.rs:130–186,244–263` | Partial. Rust supplies process tools/events, not pi commands/shortcuts/provider registration, persistent extension entries or equivalent UI interactions. Isolation intentional under `docs/design/epsilon-decisions.md:246–263`; full SDK omission not automatically justified by isolation. |
| RPC embedding | `references/pi-0.85.1/packages/coding-agent/src/modes/rpc/rpc-types.ts:20–73` | `crates/epsilon-cli/src/rpc_mode.rs:32–68,75–120` | Partial and incompatible envelope. **The Rust column was stale and understated this row (corrected 2026-09-26):** it says "Six operations; no abort, state/history/model/compaction/session management", but `rpc_mode.rs` declares **nine** — `Prompt`, `Steer`, `Followup`, `Approvals`, `Approve`, `Ping`, **`Abort`**, **`GetState`**, `SetThinkingLevel`, plus a `#[serde(other)] Unknown` catch-all. So abort and state *are* implemented; what is genuinely missing is compaction and session management, and pi's union has 32 commands. The envelope is still incompatible and F13 is an independent correctness bug. |
| Resource loading/reload/package lifecycle | `references/pi-0.85.1/packages/coding-agent/src/core/resource-loader.ts:40–52,70–155,388`; `references/pi-0.85.1/packages/coding-agent/src/core/package-manager.ts:978–1049` | `crates/epsilon-cli/src/runtime.rs:564–608`; `crates/epsilon-tui/src/app.rs:1805–1845` | Partial loading, missing unified resource/package lifecycle. Rust discovers skill/agent/extension roots but no corresponding npm/git resource install/update/remove implementation was found. Trust detection of a file does not prove it is loaded. |
| Markdown prompt templates | `references/pi-0.85.1/packages/coding-agent/src/core/prompt-templates.ts:57–102,177–194` | `crates/epsilon-core/src/prompt.rs:88–149`; `crates/epsilon-tui/src/app.rs:1805–1845` | Missing named templates/argument expansion. Compaction templates and credential interpolation are different features. Intent unknown. |
| Skill discovery/frontmatter/collisions | `references/pi-0.85.1/packages/coding-agent/src/core/skills.ts:168–173,256–285,293–321,421–450` | `crates/epsilon-core/src/skills.rs:63–82,117–124` | Partial, and **deliberately divergent in a way the source comment used to deny.** pi keeps the **first** skill for a name and emits a collision diagnostic naming `winnerPath`/`loserPath` (`skills.ts:432–448`); Epsilon **replaces** the earlier entry, so a project skill overrides a global one. That is pinned by a test (`discovers_validates_and_shadows_by_priority_root_order` asserts `"Project override."` and `body-v2`), so it is intentional. **The collision diagnostic is not implemented** — `discover` returns `Vec<Skill>` with no channel for one, so shadowing a global skill is silent. Corrected 2026-09-26: `skills.rs:65` claimed the behaviour was "mirroring pi", which was false in both directions; the comment now states the divergence and cites W11. Rust's shallow scan/flat frontmatter also differ. |
| Configuration breadth | `references/pi-0.85.1/packages/coding-agent/src/core/settings-manager.ts:12–120` | `crates/epsilon-core/src/config.rs:20–98,148–169`; `crates/epsilon-cli/src/runtime.rs:29–39` | Partial settings; TOML/declarative config intentional (`docs/design/epsilon-decisions.md:386–402`). Queue/retry/resource/theme configuration breadth still needs explicit disposition. |
| TUI/editor/themes | `references/pi-0.85.1/packages/tui/src/index.ts:13–46,50–75,116–138` | `crates/epsilon-tui/src/lib.rs:12–30`; `crates/epsilon-tui/src/editor_ops.rs:19–73`; `crates/epsilon-tui/src/ui/theme.rs:1–25` | Partial, with useful editor primitives already implemented. Ratatui intentional (`docs/design/epsilon-decisions.md:352–354`); complete pi component, theme or key-by-key behavior parity not established. **Corrected 2026-09-26: this row cited `crates/epsilon-tui/src/ui/theme.rs:1–25`, a file that does not exist** — the theme is at `crates/epsilon-tui/src/ui/theme.rs`. |
| Image input/processing/rendering | `references/pi-0.85.1/packages/coding-agent/src/core/tools/read.ts:41–57` (image MIME option, `autoResizeImages`); `references/pi-0.85.1/packages/coding-agent/src/utils/image-process.ts:87` (`resizeImage`); `references/pi-0.85.1/packages/coding-agent/src/utils/image-resize.ts`; `references/pi-0.85.1/packages/tui/src/terminal-image.ts:68–105` | `crates/epsilon-tools/src/files.rs:110–142`; `crates/epsilon-core/src/references.rs:9–35,69–85`; `crates/epsilon-tui/src/state.rs:38–48` | Partial: read emits supported image bytes; no equivalent resize pipeline or terminal graphics established. `@` text attachment handling is not pi CLI image processing. |

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
