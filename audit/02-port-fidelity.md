# 02 — Port fidelity

The original goal was a complete, faithful port of `pi-agent` to Rust. This document
answers the only question that matters: **how much of pi is actually here, and how
faithful is it?**

The short answer is that the *spine* is faithfully ported and the *product surface* is
not, and that the difference is larger than the repository's own documentation
currently admits.

## Method

The reference is `references/pi-0.85.1`, a TypeScript monorepo. I compared Epsilon's
implementation against it file by file, citing both sides. Every row below names the
evidence on each side, so any claim can be checked rather than believed.

Gross size is comparable and therefore misleading: pi is ~137,870 lines of TypeScript
across the relevant packages, Epsilon is 107,837 lines of Rust. The *distribution* is
the story — pi spends 70k on `coding-agent` (`interactive-mode.ts` alone is 6,592 lines),
Epsilon spends 28k on the TUI and has no equivalent of the interactive layer.

## What is faithfully ported

These are real ports, with divergences that are deliberate and ADR-backed.

| Capability | pi | Epsilon | Note |
|---|---|---|---|
| Two-level loop (steering inner, follow-up outer) | `agent-loop.ts:167-174,257-267` | `core/src/lib.rs:1452-1453,1783,1832-1846` | Ported |
| Queue drain defaults | `agent.ts:231-232` (OneAtATime ×2) | `lib.rs:196,200` | Ported exactly |
| Abort and cancellation | `agent-loop.ts:215,476,514,636,655` | `lib.rs:1356-1396` | Ported, and richer — token plus `abort_armed` plus delegated cascade |
| Stop reasons (error / aborted / length) | `agent-loop.ts:215,231` | `lib.rs:1551,1588,1611,2256` | Ported |
| Turn and next-turn hooks | `agent-loop.ts:177,232-257,288-295` | `contracts/src/turn.rs`, `lib.rs:980` | Ported |
| Tool-call iteration, truncation, max-token rejection | `agent-loop.ts:226-232,379-425` | `exec.rs:183-256,540-606` | Ported with a different scheduler (M72, deliberate) |
| Parallel tool pool + doom-loop brake | `agent-loop.ts:487-548` | `exec.rs`, `lib.rs:1963` | Ported and extended |
| Durable JSONL session tree | `harness/session/types.ts:14-74` | `contracts/src/session_entry.rs:90-153`, `session/src/store.rs` | Ported, shared format, `pi_import` reconciles |
| Fork to a new session | `harness/session/fork.ts:30,70` | `store.rs:1486-1530` | Ported — ancestry only, fresh identity |
| Compaction and pressure accounting | `harness/compaction/compaction.ts:157-161,245,312` | `compact.rs:12,100-109`, `lib.rs:1907-1917` | Ported; reserve is the same 16,384 |
| Token estimation | `ai/utils/estimate.ts:17,37-45,114` | `compact.rs:12-109` | Equivalent approach, not a line-port |
| Native provider wires | `ai/providers/*` | `llm/src/providers/mod.rs` — anthropic, bedrock, google, vertex, mistral, openai-compat, openai-responses, github-copilot | Ported |

The agent loop is the part a reader should trust most, and it earns that. The queue
semantics match to the level of both defaults being `OneAtATime`; the cancellation
model is a superset of pi's; the compaction reserve is numerically identical.

## What is missing

| Capability | pi evidence | Epsilon evidence | Status |
|---|---|---|---|
| **Branch navigation** (move the leaf to any ancestor, re-enter) | `agent-session.ts:3116` `navigateTree` | **Ported and wired** — `session/src/store.rs:1329` `navigate_to`, called by the timeline picker on Enter at `tui/src/app/keys.rs:495` | **Corrected 2026-09-28** (was "Missing") |
| **Branch summary in production** | `agent-session.ts:3116-3270`, 5 callers, bills usage | `store.rs:1372-1400` exists — **only tests call it** | **Store primitive ported; nothing wires it** |
| **Extension SDK** | `coding-agent/core/extensions/types.ts:1198-1346` — 157 exports: event bus, `addCommand`, `registerProvider`, `setStatus`, `newSession` | `ext/src/manifest.rs:18-37`, `host.rs:130-263` — process tools and declared events | **Subset only** |
| **Tool-call interception** (block, and rewrite the result) | `runner.ts:30975` `emitToolCall` (returns on first `block`), `runner.ts:29468-30490` `emitToolResult` (folds each handler into a running event, returns `undefined` if unmodified) | `cli/src/runtime.rs` `ExtensionToolHooks` implements both `ToolHooks` arms, wired by `install_extension_hooks` in `main.rs` | **Ported** (`e9d5b94`, `ece3af4`, `9f73a5f`, this slice) |
| **Turn-level hooks** (reshape context, swap model, end the run) | `runner.ts` `turn_end` + `shouldStopAfterTurn`; `agent-loop.ts:248-257` ends the run without another request | `cli/src/runtime.rs` `ExtensionTurnHooks` implements all four `TurnHooks` methods, wired by `install_extension_turn_hooks` in `main.rs` | **Partial — see the divergence below** |
| **Tool-call argument mutation** | `types.ts` `ToolCallEvent`: "`event.input` is mutable. Mutate it in place to patch tool arguments before execution... **No re-validation is performed after mutation.**" | `BeforeToolCallContext.args` is passed by value and never returned; a decision carries only `block`/`reason`/`terminate` | **Missing, and deliberately so — see below** |
| **`terminate` early-exit hint** | `types.ts` `ToolCallEventResult.terminate`: honoured only when *every* blocked call in a batch sets it | carried on both `BeforeToolCallResult` and `AfterToolCallResult`; `exec.rs:788` encodes it in `details` | **Ported; the all-blocked rule is the loop's, not the bridge's** |
| **Prompt templates** (`/name` expansion) | `core/prompt-templates.ts` — 4 exported functions, full bash-style substitution grammar | `contracts/src/prompt_template.rs` — the same grammar plus `${1}`/`${@}`, expanded in the TUI at `app.rs:submit_editor` **before** command parsing | **Ported and wired** (`3902461`, `d9efe9f`) |
| **Package / resource lifecycle** (npm, git install) | `package-manager.ts:978-1049` (2,699 lines), `package-manager-cli.ts` (1,102) | root discovery only: `cli/src/runtime.rs:564-608` | **Missing** |
| **RPC breadth** | `modes/rpc/rpc-types.ts:20-73` — 32 commands incl. `bash`, `fork`, `get_tree`, `export_html`, `set_model`, `clear_queue` | `cli/src/rpc_mode.rs:30-68` — 9 ops | **Partial; envelope incompatible** |
| **Provider coverage** | `ai/providers/all.ts:88-133` — 38 providers | `llm/src/catalog/catalog_data.rs` — 13 providers, 557 model rows | **Partial** |
| **Image generation** | `openrouterImagesProvider` | none | **Missing** |
| **Telemetry** | `harness/telemetry.ts:42,233,595` | none | **Missing** |
| **SQLite session backend** | `session-backends/sqlite-node` | JSONL only | **Missing (out of stated scope)** |
| **TUI terminal graphics** — Kitty/iTerm2 images, Kitty key protocol | `tui/src/terminal-image.ts`, `keys.ts:1401` | `state.rs:41-42` says "honest-parity deferred" | **Deferred, and honestly labelled** |

The RPC `Unknown` arm at `rpc_mode.rs:68` deserves a note: it is explicitly commented as
a deliberate refusal to half-wire a command. That is the right call, and it is the
opposite of the failure mode this audit is looking for.


### Two gaps in tool-call interception, read from pi's source rather than assumed

**1. pi mutates tool arguments in place; Epsilon does not.** `types.ts` documents
`ToolCallEvent.input` as mutable: an extension patches arguments before execution, later
handlers see earlier mutations, and — stated plainly in the reference — **no re-validation
happens after the mutation**. That last sentence is the reason this is not simply a missing
feature. Epsilon's `BeforeToolCallContext.args` crosses a process boundary, so honouring it
means re-validating against the tool's own schema after the edit, which pi does not do. A
faithful port of the *unsafe* half without the validation is a worse outcome than the honest
gap, so `args` stays read-only and the decision carries no `args` field. Recorded as a
deliberate divergence, not an oversight.

**2. pi returns the first `block` and stops; Epsilon consults every extension.**
`emitToolCall` returns immediately on a block, so an extension after a blocking one is never
reached. Epsilon asks all of them, on the stated grounds that the reason a user sees should
name every extension that objected rather than whichever loaded first. This is a **deliberate
divergence in load-order behaviour**: it is more informative, and it costs one round trip per
remaining extension on a call that was going to be refused anyway. The veto *semantics* match
pi exactly — a block is a block.

**3. Epsilon does not let extensions reshape the context; pi does.** `TurnHooks` has four
methods. `transform_context` and `convert_to_llm` return the messages the provider will
receive, and this bridge implements **both as pass-through**. Only `prepare_next_turn` (a
model swap) and `should_stop_after_turn` (a clean run exit) are reachable from a child.

The reasoning is about failure modes rather than trust. The extension host is the user's own
process and pi deliberately allows context rewriting — a harness that blocked it would be
useless for the extensions it exists to support. But a *malformed or buggy* answer on this
seam is not a refused tool call: it is a corrupted conversation, silently, with the model
behaving as though it had been told something false. There is no error result to show the user
and nothing to roll back, and the harness has no way to distinguish "this extension is
rewriting my prompt on purpose" from "this extension has a bug in its fourth handler".

`prepare_next_turn` is the reachable part because the loop already treats its result as
optional: a broken host yields `None` and the run continues. That is a real capability gap
and it is preferable to a port that lets a third-party binary corrupt the prompt by accident.

## What Epsilon has that pi does not

These are additions, not ports, and several are the reason to use Epsilon over pi:

- **A real permission and safety layer** — `epsilon-safety/`: `engine.rs`, `gate.rs`,
  `escalation.rs`, and `sandbox/` with bubblewrap plus Windows Job Objects
  (`sandbox/jobobject.rs:514`). pi has no policy engine; its `effect-gate.ts` is a seam,
  not a policy. This is Epsilon's clearest genuine advantage.
- **LSP diagnostics** — `epsilon-lsp/`, injected into the write and edit tools
  (`epsilon-tools/src/lib.rs:112-127`).
- **MCP** — `epsilon-mcp/`, with a stdio transport. pi 0.85.1 has no MCP package.
- **Subagent delegation as a tool** — `task` (`lib.rs:2598`, with a 16-child registry
  cap). pi uses extension-managed agents, not a built-in tool.
- **Three extra tools**: `glob`, `web_fetch`, `todo`.

## Two findings that matter more than the table

**The default tool set is much larger than pi's, on every request.** pi activates
**4** tools by default (`agent-session.ts:2810`). Epsilon's `default` agent type hands
the model the **full 11-tool registry** (`lib.rs:2612`) on every request. That is a
per-turn context tax on every single call, in a product whose entire premise is
managing a finite context window. It is the single most consequential behavioural
divergence from pi in this document, and it is not visible in any capability table.

**The cheapest remaining parity win is already built and unwired.**
`branch_with_summary` (`store.rs:1372`) mirrors pi's `branchWithSummary` root-abandonment
shape exactly, and `fork_branch_at` (`store.rs:1486`) writes only the selected ancestry
with fresh identity. Both are tested. I verified the gap myself:

```
$ grep -rn "branch_with_summary" --include=*.rs crates/
  → every hit is in tests/ or is the definition itself
```

Wiring `/tree` to navigate rather than merely display is a genuine parity feature whose
hard half is already done and proven.

## Where the documentation misleads

**`docs/qa/03-partial-vs-full-port.md` is stale and is the weakest claim in the
repository.** It states that tools, permissions, compaction, skills/MCP, and the
TUI/server/print modes are all *missing* (lines 26-33, describing M6-M9). Every one of
those is now implemented. It was accurate on 2026-08-23 and is wrong today. A reader who
takes it at face value gets a **false negative** — the document understates the project
by about five milestones. It should be marked as a historical snapshot or deleted.

The risk is concrete: that document's own headline is that "M5 delivers the complete
architectural spine", and a reader who stops there concludes the port is done. The spine
is complete. The product is roughly 60% of pi's coding-agent.

**`audit/history/03-port-plan.md` is disciplined but drifting.** It correctly self-labels
24 SUPPORTS / 25 PARTIAL / 2 CONTRADICTS / 4 UNFINDABLE, and notes that its citation
checker proves *checkable*, not *verified* — which is exactly the right caveat. Two
specific claims have fallen behind: its branch-navigation evidence predates
`fork_branch_at` existing (the conclusion still holds; the evidence is stale), and its
tool-count row is correct — pi's union at `index.ts:95` is 8 including `powershell`,
and Epsilon folds PowerShell into one `bash` tool.

**No document overstates completeness in the other direction.** Where Epsilon is
genuinely behind — extension SDK, prompt templates, package manager, 25 of 38 providers,
image generation, telemetry, RPC breadth — the docs say Partial or Missing. That
honesty is worth preserving through the restructure.

## Verdict

**The loop is a faithful port. The harness around it is roughly 60% of pi's, and the
repository's own documentation understates that gap rather than exaggerating it.**

That is a defensible place to be, and the remaining work is well-chosen rather than
scattered. Two of the missing items are not "features we might add someday" — they are pi
capabilities a user would immediately notice are absent: prompt templates, and a workable
extension surface.

**The extension gap is worse than the 60% figure suggests, and this paragraph was wrong
until a delegated recheck on 2026-09-28.** Counting the SDK by whether *some* counterpart
exists flatters the result: an extension can contribute a tool and observe events, which
looks like a working SDK. It cannot participate in a run, appear in the UI, or survive its
own process. pi declares 153 exports across 26 areas; `epsilon-ext` is 979 LOC and depends
only on `epsilon-contracts`, so a child process has no path to the agent at all. See
[`evidence/delegation-port-gaps-2026-09-28.md`](evidence/delegation-port-gaps-2026-09-28.md).

**And the same recheck corrected a claim above rather than confirming it.** Branch navigation
was listed here as **Missing**; it is wired (`keys.rs:495`). The audit searched for pi's
identifier, found no `navigate_to`, and concluded absence — while
`evidence/port-gap-recheck-2026-09-21.md` and `evidence/F22-compaction-usage-never-read.md`
had already said so, in this repository, a week earlier. The conclusion was right by luck
(there *was* a real defect, the missing re-projection, now fixed in `7489c00`) and the
mechanism was wrong. Treat the other rows in this table as less settled than they look: a
negative here has only been checked by reading, and one such check has already failed.

## Honest limits

- This is a static comparison. Nothing here was executed against the pi reference.
- Provider coverage is counted from the catalog's generated snapshot, not from a live
  `/models` call, so a provider pi supports dynamically would not appear.
- The `chord`, `client` and `evals` pi packages were judged out of scope. If they are
  in scope for Epsilon, the missing list grows.
- Line counts are gross and include tests and comments; they indicate distribution, not
  quality.
