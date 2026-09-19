# pi → Epsilon port fidelity audit (2026-09-19)

## Method (files read in pi and in the Rust crates)

Read in full or in the cited ranges. **pi** (`references/pi-0.85.1/`): `packages/agent/src/agent-loop.ts` (all 803), `types.ts:255-446`, `stream-fn.ts`, `harness/system-prompt.ts`, `harness/session/types.ts:14-73`, `harness/config.ts`; `packages/coding-agent/src/core/`: `system-prompt.ts` (all 168), `skills.ts:350-433`, `resource-loader.ts:55-164,1012-1049`, `prompt-templates.ts:1-80`, `compaction/compaction.ts` (all 1012), `tools/index.ts` (all), each of `tools/{read,bash,powershell,edit,write,grep,find,ls}.ts` (definitions + schemas), `agent-session.ts:1120-1160,1930-2000,2110-2310,2760-2820`, `settings-manager.ts:840-910`, `defaults.ts`; `packages/ai/src/utils/{retry.ts,overflow.ts,provider-retry.ts}`, `packages/ai/src/types.ts:546-562`.

Read in **Rust** (`crates/`): `epsilon-core/src/lib.rs` (loop `run_inner` 1188-1602, `compact_now` 1762-1965, `stream_one_turn` 2035-2286, helpers 255-284/730-1075), `exec.rs` (all 990), `compact.rs:1-512`, `prompt.rs` (all 709), `skills.rs` (all 206), `registry.rs` (all 151); `epsilon-contracts/src/{agent_event.rs,message.rs,stream.rs,session_entry.rs,tool.rs}`; `epsilon-tools/src/lib.rs` (all 154), `util.rs:1-40`, `descriptions/*.txt`, plus the tool modules cited below; `epsilon-llm/src/{retry.rs,retry_stream.rs,stream_fn.rs,lib.rs}`, `providers/mod.rs:1380-1419`; `epsilon-cli/src/main.rs:1535-1600`, `runtime.rs:460-540`.

Two breadth dives were delegated (tool parameter/description text; streaming shapes + retry constants); their anchors were spot-verified against the files by me before use. No cargo, no writes to source.

## Coverage table

| pi module | Rust counterpart | class | notes |
|---|---|---|---|
| `agent-group.ts` loop `runLoop` | `epsilon-core/src/lib.rs:1229-1595` | DIVERGENT (equiv.) | Same two-level steering/follow-up loop; turn order matches `agent-loop.ts:175-258` exactly, incl. `prepareNextTurn` before `shouldStopAfterTurn` (`lib.rs:1479-1534`) |
| `agent-loop.ts:96-119` | `lib.rs:1074-1124` | PARTIAL | `agent_start`/`agent_end` present; **no `message_start`/`message_end` for the prompt** (see D-17) |
| `agent-loop.ts:279-370` | `lib.rs:2035-2286` | DIVERGENT (equiv.) | Terminal-frame contract, partial preservation, `Done` authoritative — all match |
| `agent-loop.ts:409-561` tool batch | `exec.rs:260-621` | DIVERGENT (**better**) | pi: one `Sequential` tool serializes the whole batch. Rust: per-resource conflict groups + barrier only for unknown reach |
| `types.ts:431-446` `AgentEvent` | `epsilon-contracts/src/agent_event.rs:105-192` | EXTRA/PARTIAL | +permission/compaction/notice events; `tool_execution_end` lost its `result` (D-16) |
| `core/system-prompt.ts` | `epsilon-core/src/prompt.rs:92-149` | DIVERGENT | 3 tiers (STABLE/CONTEXT/VOLATILE); ordering matches; tool snippets + per-tool guidelines differ (D-19) |
| `core/resource-loader.ts:71-157` | `prompt.rs:159-193` | PARTIAL | candidate list and ancestor walk are narrower (D-8) |
| `resource-loader.ts:1023-1049`, `prompt-templates.ts` | none | MISSING | `SYSTEM.md`, `APPEND_SYSTEM.md`, `prompts/*.md` (D-7) |
| `core/skills.ts:355-383` | `skills.rs:118-127` | DIVERGENT (bug) | XML `<available_skills>` + `<location>` vs `- name: description`; no `skill` tool (D-1, D-4) |
| `core/compaction/compaction.ts` | `epsilon-core/src/compact.rs` + `lib.rs:1762-1965` | PARTIAL | trigger + tail budget differ (D-12), **summary template absent** (D-3), `length` not refused (D-5) |
| `harness/session/types.ts` | `epsilon-contracts/src/session_entry.rs` | DIVERGENT (equiv.) | Entry kinds match; `fromHook` and branch-summary `details` absent (non-matter) |
| `core/tools/*.ts` (8 tools) | `epsilon-tools/src/*` (11 tools) | DIVERGENT | No `powershell` (D-10); default active set differs (D-6); grep/find/ls semantics + description text differ (D-9, D-20-25) |
| `utils/retry.ts`, `provider-retry.ts` | `epsilon-llm/src/retry.rs`, `retry_stream.rs` | DIVERGENT | Constants, classification, structured metadata differ (D-14, D-15, D-27) |
| `utils/overflow.ts:37-63` | `lib.rs:276-284` | PARTIAL (bug) | 27 regexes + exclusions vs 6 needles (D-2) |
| `ai/src/types.ts:546-562` | `epsilon-contracts/src/stream.rs:18-60` | PARTIAL | Reduction to 6 variants; tool-call deltas never surface (D-11) |
| `defaults.ts:3` thinking default | `epsilon-contracts/src/message.rs:41-84`, `lib.rs:523` | DIVERGENT | pi defaults to `"medium"`, Epsilon to `Off` (D-26) |
| — | `epsilon-core/src/guards.rs`, `verify.rs`, `cache_stats.rs`, `untrusted.rs` | EXTRA | Doom-loop brake, stuck-run guard, verification nudge, cache attribution, untrusted shaping |

## Divergences that matter (ranked: HIGH = a pi behaviour a user would notice or rely on)

### D-1 — HIGH — the agent contract promises a `skill` tool that does not exist (DIVERGENT, bug)
pi: `references/pi-0.85.1/packages/coding-agent/src/core/skills.ts:355-383` publishes each skill as
`<skill><name>…</name><description>…</description><location>…</location></skill>` and instructs
`"Use the read tool to load a skill's file when the task matches its description."` (`:365`).
Epsilon: `crates/epsilon-core/assets/agent-contract.md:103-105` says `"Load a skill's body via the
`skill` tool"`, but `crates/epsilon-tools/src/lib.rs:109-138` registers no such tool (grep of
`crates/` for `"skill"` returns nothing), and `crates/epsilon-core/src/skills.rs:118-127` prints
`- {name}: {description}` with no path. **What differs:** the model is told a skill exists, told to
open it with a tool that is absent, and given no location it could `read`. **Smallest fix:** emit
`<location>` in the catalog and reword the contract to `read` (pi's own rule), or register the tool.

### D-2 — HIGH — an Anthropic context overflow is never recognised, so recovery never runs (PARTIAL, bug)
pi matches 27 patterns incl. `/request_too_large/i` for the Anthropic 413 body and `/prompt is too
long/i` (`references/pi-0.85.1/packages/ai/src/utils/overflow.ts:38-39`, with the exact bodies
documented at `:11-12`). Epsilon's classifier is 6 lowercase needles
(`crates/epsilon-core/src/lib.rs:276-284`) — `"request too large"` **with spaces**, and no
`prompt is too long` at all — while the failure text is the provider body verbatim
(`crates/epsilon-llm/src/providers/mod.rs:1401-1403`: `"HTTP {status} ({kind:?}): {clipped}"`).
**What differs:** `request_too_large` contains no `request too large`, so the overflow branch at
`lib.rs:1321-1352` never fires for Anthropic and the run ends with a raw HTTP error where pi
compacts once and retries. **Smallest fix:** add `request_too_large`, `prompt is too long`,
`exceeds the context window`, plus pi's 3-pattern non-overflow exclusion list (`overflow.ts:74-78`).

### D-3 — HIGH — the compaction summary template is never given to the model (PARTIAL)
pi sends an exact 6-section structure — `## Goal`, `## Constraints & Preferences`, `## Progress`
(Done/In Progress/Blocked), `## Key Decisions`, `## Next Steps`, `## Critical Context` —
(`references/pi-0.85.1/packages/coding-agent/src/core/compaction/compaction.ts:467-498`) with an
`UPDATE_SUMMARIZATION_PROMPT` variant for iterative updates (`:500-539`, selected at `:678`).
Epsilon's `crates/epsilon-core/src/compact.rs:147-218` sends `<conversation>` plus a ledger block and
the sentence "Do not paraphrase the user's instructions…"; the system prompt claims a template that
is nowhere in the request: `"Output ONLY the template."` (`compact.rs:222-223`). **What differs:**
the model is asked to follow a template it was never shown, so post-compaction context shape is
whatever it invents. Only Epsilon's own tags (`<file_ledger>`, `<user_instructions>`) are
machine-read, so nothing downstream breaks — but the summary quality pi engineered for is lost.
**Smallest fix:** inline the pi template into `summary_context` (one const, ~30 lines).

### D-4 — HIGH — `disable-model-invocation` skills are advertised anyway (DIVERGENT, bug)
pi filters them: `skills.filter((skill) => !skill.disableModelInvocation)`
(`coding-agent/src/core/skills.ts:356`). Epsilon parses the flag
(`crates/epsilon-core/src/skills.rs:100-102`) and stores it, but `catalog()` (`skills.rs:118-127`)
iterates every skill. **What differs:** a skill the author marked as slash-command-only still appears
in the model's prompt and will be attempted (see D-1 — with no way to open it).
**Smallest fix:** `skills.iter().filter(|s| !s.disable_model_invocation)` in `catalog`.

### D-5 — HIGH — a token-capped (truncated) summary is committed as context (DIVERGENT, bug)
pi refuses it explicitly: `if (response.stopReason === "length") return "…the summary is incomplete"`
(`compaction.ts:549-551`). Epsilon checks only `if answer.stop_reason.is_failure()`
(`crates/epsilon-core/src/lib.rs:1872`), and `is_failure()` is `matches!(self, Error | Aborted)`
(`crates/epsilon-contracts/src/message.rs:110-112`) — `MaxTokens` is **not** a failure. The next gate
is `summary_text`, which accepts any non-empty text (`compact.rs:232-235`). **What differs:** a
half-written summary can replace good context, which is the exact failure mode pi coded against.
**Smallest fix:** `if answer.stop_reason.is_failure() || answer.stop_reason == StopReason::MaxTokens`.

### D-6 — HIGH — eleven tools are active where pi ships four (DIVERGENT)
pi's default active set is `["read", "bash", "edit", "write"]`
(`references/pi-0.85.1/packages/coding-agent/src/core/agent-session.ts:2808-2810`), matching the
system prompt default (`core/system-prompt.ts:45`); grep/find/ls/powershell exist but are opt-in via
`activeToolNames`. Epsilon registers and activates all eleven on the product path
(`crates/epsilon-cli/src/main.rs:1547` → `crates/epsilon-tools/src/lib.rs:109-138`; no subsetting
anywhere in the CLI). **What differs:** a much larger tool block in the STABLE prompt tier and a
wider action surface by default. **Smallest fix:** default the registry to the four and let the rest
opt in — `ToolRegistry::subset` already exists (`crates/epsilon-core/src/registry.rs:59-67`).

### D-7 — HIGH — `SYSTEM.md`, `APPEND_SYSTEM.md` and prompt templates are detected but never loaded (MISSING)
pi loads `.pi/SYSTEM.md` (replaces the default prompt) and `.pi/APPEND_SYSTEM.md` (appends), project
then global (`resource-loader.ts:1023-1049`), and expands `prompts/*.md` slash templates with
`$1`/`$ARGUMENTS` substitution (`prompt-templates.ts:11-18,70-80`). Epsilon *enumerates* all three
for the trust gate — `crates/epsilon-cli/src/runtime.rs:468,472-475,506-521` — and nothing reads
their contents: grep of `crates/` finds no reader, and `prompt::build` has no slot for either
(`crates/epsilon-core/src/prompt.rs:92-149`). **What differs:** a user who writes
`.epsilon/SYSTEM.md` gets a trust prompt and no effect. **Smallest fix:** read both in
`epsilon-core::prompt` (an `append` field on `Stable`, a `custom` override path) and wire the
existing trust enum to it.

### D-8 — HIGH — `AGENTS.override.md` and ancestor context files never apply (PARTIAL)
pi's candidate list is `["AGENTS.override.md", "AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"]`
(`resource-loader.ts:72`) and it walks **every ancestor to the filesystem root** accumulating them
outermost-first (`:140-152`). Epsilon reads `["AGENTS.md", "CLAUDE.md"]`
(`crates/epsilon-core/src/prompt.rs:160`) at the workspace root, plus hints only from directories the
agent actually touched, bounded inside the workspace (`prompt.rs:221-241`, `:230`). **What differs:**
(a) an `AGENTS.override.md` — pi's escape hatch for a machine-local override — is ignored;
(b) opening Epsilon in a subdirectory of a repo loses the repo-root `AGENTS.md` entirely.
**Smallest fix:** add the two candidates to `AGENTS_CANDIDATES` (one line); the ancestor walk is a
separate, larger change.

### D-9 — HIGH — grep/find do not honour `.gitignore`, and say they do (DIVERGENT, doc bug)
pi shells to ripgrep with `--hidden` so `.gitignore` applies (`core/tools/grep.ts:162`), and `fd` for
find (`find.ts:182-198`). Epsilon's walker skips `SKIP_DIRS` **and every dot-directory**
(`crates/epsilon-tools/src/util.rs:25,160`) and never parses an ignore file — grep of `crates/` for
`gitignore` finds only the admitting comment `crates/epsilon-tools/src/find.rs:4`. Both
`descriptions/find.txt:1` ("Respects .gitignore") and `find.rs`'s own doc assert otherwise.
**What differs:** two errors in opposite directions — `.github/`, `.vscode/`, `.agents/` are
unsearchable, and gitignored build output is searched. **Smallest fix:** correct the description now;
add an ignore-file read (the walker already centralises entry filtering at `util.rs:135-169`).

### D-10 — MEDIUM — there is no `powershell` tool (MISSING as a tool; capability present)
pi ships one: `name: "powershell"` (`core/tools/powershell.ts:40-41`), prepending
`[Console]::OutputEncoding=UTF8` (`:16`), with its own prompt guideline (`:19-20`), and
`buildSystemPrompt` branches on `hasPowerShell` (`core/system-prompt.ts:98,104-112`). Epsilon has no
tool of that name; its `bash` dispatches internally (`crates/epsilon-tools/src/shell.rs:47`) and
declares `"powershell"` only as the spawned program (`shell.rs:149`). **What differs:** the capability
is reachable, the named tool and its guideline are not; Windows users get one tool named `bash`.
**Smallest fix:** none required for capability; a thin `powershell` alias would restore prompt parity.

### D-11 — MEDIUM — tool-call argument deltas never reach any frontend (PARTIAL)
pi emits `toolcall_delta` per provider (`ai/src/api/anthropic-messages.ts:697`,
`openai-completions.ts:652`) and rebroadcasts it (`agent-loop.ts:330-332`). No Epsilon adapter emits
`StreamEvent::ToolCallArgumentsDelta` (they feed the accumulator: `crates/epsilon-llm/src/providers/anthropic.rs:212-218`,
`openai_compat.rs:212-224`), `ToolCallStart` (`crates/epsilon-contracts/src/stream.rs:39-46`) is
produced nowhere, and core would drop the deltas from the broadcast anyway
(`crates/epsilon-core/src/lib.rs:2240-2241`, deliberate: "transport noise"). **What differs:** a UI
cannot say "calling `read`…" before the call completes. Justified for the wire, but a named loss.

### D-12 — MEDIUM — compaction is triggered before a turn, pi triggers it after a run (DIVERGENT)
pi: `_checkCompaction` runs post-run (`agent-session.ts:1141`, and pre-prompt for a prior abort at
`:1253`), then `agent.continue()` retries. Epsilon: at the top of every inner-loop iteration
(`crates/epsilon-core/src/lib.rs:1262-1276`). **What differs:** Epsilon can compact mid-tool-loop,
pi only between runs. Epsilon's is more eager — better against overflow, one extra summary call
possible in a long tool loop.

### D-13 — MEDIUM — a length-truncated (not overflow) response gets no recovery (DIVERGENT)
pi has a second recovery trigger, `isRecoverableLength` (`overflow.ts:171-173`, used at
`agent-session.ts:2162`), so a response cut off by the output cap can compact and retry. Epsilon's
only compaction triggers are the pressure check and the string-matched overflow
(`lib.rs:1262,1320-1326`); `StopReason::MaxTokens` drives only the fail-all-tool-calls rule
(`lib.rs:1379-1381`, `exec.rs:61-84`). **What differs:** `finish_reason: length` ends the run where pi
recovers once.

### D-14 — MEDIUM — retry constants differ and quota errors are retried (DIVERGENT)
pi CLI: `maxRetries ?? 3, baseDelayMs ?? 2000` → waits 2s/4s/8s, user-configurable
(`settings-manager.ts:882-888`), and quota/billing errors are **explicitly non-retryable**
(`ai/src/utils/retry.ts:7-24`: `insufficient_quota`, `out of budget`, `Monthly usage limit reached`).
Epsilon: `RETRY_MAX_ATTEMPTS = 5` (`crates/epsilon-llm/src/lib.rs:52`), base 500 ms with 50–100%
jitter (`epsilon-llm/src/retry.rs:96-102`), hard-coded, and `classify` routes anything containing
`429` or `quota` to `RateLimit` → retried (`retry.rs:29-35`). **What differs:** one extra attempt,
faster waits, and a retry loop against an exhausted quota that pi refuses.

### D-15 — MEDIUM — `Retry-After` / `x-should-retry` support is unreachable (PARTIAL)
pi honours `x-should-retry` and `retry-after-ms`/`retry-after`
(`ai/src/utils/provider-retry.ts:23-26,52-63`). Epsilon declares the parameter
(`retry.rs:88-91`) but the only caller passes `None` (`crates/epsilon-llm/src/retry_stream.rs:148`)
and `epsilon-llm/src/http.rs` parses no response headers. **What differs:** a server-requested wait is
ignored; the `Retry-After` branch is dead code.

### D-16 — MEDIUM — `tool_execution_end` carries no result (PARTIAL)
pi: `{ type: "tool_execution_end"; …; result: any; isError: boolean }`
(`packages/agent/src/types.ts:446`). Epsilon: `tool_call_id`, `tool_name`, `is_error` only
(`crates/epsilon-contracts/src/agent_event.rs:168-175`). **What differs:** a reducer over the event
stream cannot render a finished tool from the event; it must wait for `MessageEnd`.

### D-17 — MEDIUM — no `message_start`/`message_end` for user prompts or steering messages (PARTIAL)
pi emits both for the prompt (`agent-loop.ts:112-115`) and for every steering message (`:202-207`).
Epsilon's `push_message` (`lib.rs:915-920`) emits nothing, and both call sites are silent
(`lib.rs:1108` for the prompt, `lib.rs:1251-1255` for drained steering). Only task notices emit a
`MessageEnd` there (`lib.rs:1619-1621`). **What differs:** the documented reducer model
(`lib.rs:7-8`) never sees the user's own turn; the TUI draws it from its own state. *Inference:* the
TUI is therefore correct today, and an out-of-process reducer would not be.

### D-18 — MEDIUM — parallel batches announce starts late (DIVERGENT, minor)
pi emits all `tool_execution_start` events sequentially **before** any execution
(`agent-loop.ts:497-503`). Epsilon emits each start inside the worker that runs it
(`crates/epsilon-core/src/exec.rs:635-639`), so starts interleave with ends. Result ordering is
correct on both sides (pi `:547-555`, Epsilon slots at `exec.rs:542,615-618`).

### D-19 — MEDIUM — the tool list in the prompt is the full description; per-tool guidelines are absent (DIVERGENT)
pi keeps two texts per tool: a provider `description` and a short `promptSnippet`
(`core/tools/read.ts:21-23` = `"Read file contents"`) plus `promptGuidelines`
(`read.ts:23` "Use read to examine files instead of cat or sed.", `write.ts:19`, `edit.ts:46-52`,
`bash.ts:45`). Epsilon derives the snippet from the description
(`lib.rs:2071-2073`, `snippet: d.description.lines().next()`), and 9 of 11
`crates/epsilon-tools/descriptions/*.txt` are a single line — so `edit`'s 834-character description
enters the STABLE tier verbatim. `ToolDescriptor` has no snippet/guideline field
(`crates/epsilon-contracts/src/tool.rs:31-46`). **What differs:** duplicated prompt bytes and the loss
of pi's behavioural guidelines. Fix: add the two fields or split the description files.

### D-20 — MEDIUM — remaining tool-surface deltas a user could hit
Each anchored, classified, and individually small:
- **read page size** DIVERGENT: pi 2000 lines / **50KB** (`core/tools/truncate.ts:11-12`); Epsilon
  2000 / **256KB** (`crates/epsilon-tools/src/util.rs:12-14`). Bigger pages, fewer round trips.
- **read numbering** DIVERGENT: Epsilon prefixes `"  12| "` (`files.rs:163-173`); pi returns the raw
  selection and numbers only in its renderer (`read.ts:162`, `renderers/read.ts:28-36`). The model now
  copies line-numbered text into `old_string` candidates.
- **bash timeout** PARTIAL: pi has **no default** and max 2147483 s (`bash.ts:22-23,40`) and keeps the
  captured output in the timeout message (`:349-359`); Epsilon defaults 120 s / caps 600 s and
  **discards** captured output on both timeout branches (`shell.rs:332-341,390-396`).
- **bash exit code** DIVERGENT (documented, better): pi throws `Command exited with code N`
  (`bash.ts:364-366`); Epsilon returns `Ok` with `[exit code: N]` (`shell.rs:416-429`).
- **grep casing** DIVERGENT: pi's schema field is `ignoreCase` (`grep.ts:25`); Epsilon's is
  `ignore_case` with no serde alias (`crates/epsilon-tools/src/search.rs:47-48`), unlike `edit`
  which does alias (`files.rs:389,410`). A pi-shaped call silently searches case-sensitively.
- **grep byte cap** PARTIAL: pi truncates the whole result at 50KB (`grep.ts:282-296`); Epsilon caps
  only line length (`search.rs:319-327`), so a large `limit` is unbounded.
- **ls** PARTIAL: pi has `limit` (500) plus a `Use limit=1000 for more` notice and a 50KB cap
  (`ls.ts:13,141,147`); `LsArgs` has only `path` (`search.rs:488-493`) and no truncation.
- **find** PARTIAL: pi emits a "limit reached" notice (`find.ts:282-285`); Epsilon truncates silently
  (`find.rs:103-118`).
- **edit** PARTIAL: pi normalizes NFKC/smart quotes/dashes and strips+restores a BOM
  (`edit-diff.ts:34-55`, `edit.ts:191-198`) and returns `{diff, patch, firstChangedLine}`
  (`edit.ts:204-211`); Epsilon's batch path is exact-only (`files.rs:622-673`), keeps the BOM
  (`util.rs:176-192`), and returns `details.diff` only — while `descriptions/edit.txt:1` claims "All
  matching is exact after LF normalization" although the single-edit path runs six fuzzy stages
  (`edit_match.rs:113-168`). Doc-vs-behaviour mismatch on the same pattern as D-9.

### D-21 — MEDIUM — thinking defaults to `off`, pi defaults to `medium` (DIVERGENT)
pi `DEFAULT_THINKING_LEVEL = "medium"` (`core/defaults.ts:3`). Epsilon's `Agent` constructs with
`ThinkingLevel::Off` (`crates/epsilon-core/src/lib.rs:523`; the enum has no `Default` impl, so the
CLI never overrides it). **What differs:** out of the box Epsilon asks for no reasoning where pi asks
for medium — a cost/quality default a user would notice on a reasoning model.

### D-22 — MEDIUM — retries surface as prose only (PARTIAL)
pi carries structured metadata: `retry_scheduled`/`retry_end` with `attempt`, `maxAttempts`,
`delayMs`, `notBefore`, `errorMessage` (`harness/runtime/drive/response.ts:373-402`) plus CLI
`auto_retry_start/end` (`agent-session.ts:169-170`). Epsilon emits `AgentEvent::Notice { text }`
(`lib.rs:1022-1024`, `agent_event.rs:109-112`) built by `retry_notice_text` (`lib.rs:259-274`).
**What differs:** no attempt count or max is machine-readable, and there is no "retry finished" event.

## Divergences that do not matter (with the reason)

- **`length` renamed to `MaxTokens`** (`message.rs:88-100` vs `types.ts:406`) — internal id; the
  semantics and the fail-all rule are identical.
- **Session `CompactionEntry.fromHook` and `BranchSummaryEntry.details` absent**
  (`harness/session/types.ts:40,47` vs `session_entry.rs:122-142`) — Epsilon has no
  extension-authored compaction hook, so there is nothing to record.
- **`branch_summary` has no `details`** — same reason; read/modified lists ride the ledger block.
- **`agent_end` emitted once from `run()`** (`lib.rs:1116`) instead of at each pi return point
  (`agent-loop.ts:217,253,272`) — observable stream is the same, message list is the same.
- **Usage anchor excludes output tokens** (`lib.rs:2219-2223`) where pi's
  `calculateContextTokens` adds `usage.output` (`compaction.ts:146-148`) — output is not re-sent;
  Epsilon's number is the correct one for a context-pressure decision.
- **Tail budget 12 000 (clamped 2 000–15 000)** (`compact.rs:114-115`, `lib.rs:1770`) vs pi's
  `keepRecentTokens: 20000` (`compaction.ts:135`) — a tuning constant; reserve matches exactly
  (16 384 on both sides, `lib.rs:204` vs `compaction.ts:134`).
- **`text_start`/`text_end`/`thinking_start`/`thinking_end` dropped** (`stream.rs:3-6`) — the header
  documents the reduction; the folded partial carries the same information.
- **`contentIndex` hardcoded to 0 by wire adapters** (`anthropic.rs:199,207`,
  `openai_compat.rs:198,207`) — `StreamFolder` compensates with an order-dependent spill rule
  (`stream.rs:92-110`); only a cosmetic mismatch between the live partial and the final message.
- **A committed compaction pushes a `"[context compacted]"` user message** (`lib.rs:1956-1962`) —
  an Epsilon addition pi lacks; it keeps the runtime transcript a faithful record.
- **`todo`/`task`/`webfetch` tools and `glob`** — pi-0.85.1 ships none of these as built-ins (grep of
  `packages/coding-agent/src` for their names finds only examples, e.g.
  `examples/extensions/subagent/index.ts`). EXTRA, justified: the subagent tool is a core product
  capability here and the agent contract is written around it. Note `task.rs:1`'s claim of
  "pi/opencode `task` parity" is accurate for opencode, not for pi.

## Where the port is BETTER than pi (be honest; do not flatter)

1. **Per-resource batch planning** (`exec.rs:194-258`): pi serializes an entire batch the moment one
   tool declares `executionMode: "sequential"` (`agent-loop.ts:417-423`); Epsilon orders only
   conflicting calls and keeps the barrier for genuinely unknown reach (`exec.rs:208-214`). Three
   edits to three files now run at once; two edits to one file stay in call order.
2. **Durable-file/view coherence on compaction** (`lib.rs:1912-1955`): the store entry is appended
   *before* runtime history is replaced and rolled back if the swap fails (`:1936-1946`), so the
   provider view and the transcript cannot disagree. pi appends after summarizing and reloads the
   session.
3. **Explicit stale-context invalidation** — `history_epoch` (`lib.rs:1737-1745,1286-1288`): a
   `prepare_next_turn` replacement is dropped once a compaction invalidates its base. pi replaces
   `currentContext` wholesale with no such guard (`agent-loop.ts:176-190`).
4. **Hard bounds pi lacks**: `max_turns: 64` (`lib.rs:200`), doom-loop brake (`exec.rs:458-525`),
   stuck-run progress guard (`lib.rs:1390-1421`, `guards.rs`), empty-turn retry
   (`lib.rs:1543-1565`), verification nudge (`lib.rs:1567-1586`). Each is observable through
   `Notice`.
5. **Never-panic provider seam** (`stream_fn.rs:68-111`) plus a 120 s idle watchdog
   (`providers/mod.rs:734-752,833-843`): a hung socket becomes a retryable network failure.
6. **Cache attribution** (`cache_stats.rs`, `prompt::stable_fingerprint` at `prompt.rs:69-86`):
   a prompt-cache miss is explained rather than mysterious. pi has cache stats
   (`core/cache-stats.ts`) but no stable-tier fingerprint travelling with the request.
7. **Untrusted-content shaping** (`untrusted.rs`, applied at `exec.rs:753-766`) — pi wraps
   nothing; Epsilon marks tool content from external sources as data.
8. **Spill with locators** (`util.rs:690-800`) and **head+tail** bash truncation with an explicit
   omission marker, versus pi's tail-only + temp file (`bash.ts:317-335`).
9. **All three compaction outcomes are typed** (`CompactionOutcome`, `lib.rs:246`) and a failed
   summarizer is a reported outcome, not a thrown error that aborts the turn.

## What you could not determine

- **No execution.** As instructed, nothing was built or run: every behaviour claim is read from
  source. The two that most deserve a running check are D-2 (that an Anthropic 413 body actually
  reaches `looks_like_context_overflow` unmodified through `clipped`) and D-5 (that a `MaxTokens`
  summarization turn can reach `compact_now`'s commit) — both are inference from the two code paths,
  not observed.
- **TUI rendering of the user's own message** (D-17): I confirmed the loop emits no
  `message_start`/`message_end` for it and did not trace the TUI's local transcript far enough to
  state that no view depends on the missing event.
- **`epsilon-tui`'s event reduction and `epsilon-server`'s SSE surface** were not read: D-11, D-16 and
  D-17 may have frontend-side workarounds I did not look for.
- **pi's `harness/runtime/drive/*` generation machinery** (`drive.ts`, `structural.ts`,
  `tool-placement.ts`, `terminal.ts`) was only sampled. It is a second, richer loop than
  `agent-loop.ts` (deferred tools, structural repair, checkpointing); whether any of it is intended
  to be in scope for the Epsilon port is not something the source alone answers.
- **`epsilon-mcp`, `epsilon-ext`, `epsilon-lsp`, `epsilon-safety`** were read only where the loop
  touches them; their own fidelity to pi's extension/MCP model is outside this audit's scope.
