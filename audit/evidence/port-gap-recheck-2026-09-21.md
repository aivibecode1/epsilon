# Port-gap re-check — `audit/03` capability matrix vs the tree at `d56ee8e`

**Read-only.** No file outside this one was touched; no cargo command was run. The working tree
already carried two modified source files when this re-check started
(`crates/epsilon-contracts/src/message.rs`, `crates/epsilon-tui/src/ui/cost.rs`); they are not mine,
they were left untouched, and nothing below depends on their contents.

`audit/03-port-plan.md` was written on 2026-09-16 over `e200815` and names its reference
snapshot as `references/pi-0.84.2/`. The tree is **129 commits** past that baseline
(`git rev-list --count e200815..HEAD`) and the only pi checkout present is
`references/pi-0.85.1/` — so every pi citation below was re-derived from 0.85.1, and
`audit/03`'s own citations can no longer be followed by a reader (see §3, W1).

Rows checked: **all 17** in the matrix (`audit/03-port-plan.md:28-44`). Row 1 was already
`Implemented`; the other 16 were `Partial`/`Missing`/intentional. Verdicts on those 16:
**2 closed outright** (rows 2 and 3), **1 unchanged accepted divergence** (row 4), and
**13 still carry an open gap** — though five of those are narrower than the plan says
because part of the row landed (rows 5, 6, 8, 11, 12). **12 factual claims are now wrong**
across `audit/03` and `audit/11` (W1–W12).

## 1. Summary table

| # | Capability (`audit/03` row) | Plan's claim | What the code does now | Verdict | Smallest closing slice |
|---|---|---|---|---|---|
| 1 | Two-level loop and drain points (`:28`) | Implemented | Drain point before each request (`crates/epsilon-core/src/lib.rs:1347-1348`), follow-ups only at the stop boundary (`:1686-1690`), stop ends before any drain (`:1566-1567`) | plan accurate | — |
| 2 | Queue delivery defaults (`:29`) | Partial: drains all follow-ups | Both queues default `OneAtATime` (`lib.rs:196-200`), matching pi (`references/pi-0.85.1/packages/agent/src/agent.ts:231-232`) | **closed** — M85 `3afafd3` | — |
| 3 | Embeddable context / next-turn / stop callbacks (`:30`) | Partial; ADR-002 promise unmet | `TurnHooks` implements all four seams (`crates/epsilon-contracts/src/turn.rs:129-173`), called by the loop (`lib.rs:1584`, `:1615`); ADR-002 carries an M89 amendment (`docs/design/epsilon-decisions.md:57-73`) | **closed** — M89 `7e23e6e` | — |
| 4 | Scheduling + token-limit rejection (`:31`) | Intentional divergence; F03 outstanding | Per-resource conflict groups + barrier for unknown reach (`crates/epsilon-core/src/exec.rs:188-213`); truncated-message calls failed as a batch (pi `agent-loop.ts:227-233`) | **accepted divergence, unchanged** | F03 (SSE EOF) is a separate ledger item (`audit/02-defect-ledger.md:16`) |
| 5 | Basic coding tools (`:32`) | "seven pi tool names… F01 blocks confidence" | 12 tools + conditional `skill` registered (`crates/epsilon-tools/src/lib.rs:128-157`, `crates/epsilon-cli/src/main.rs:1583-1587`); F01 fixed 2026-09-17 (`audit/02:14`) | **partly wrong, still open** | default active set = pi's four (`agent-session.ts:2808-2810`); `ls` limit; `ignoreCase` alias |
| 6 | Durable session trees and format (`:33`) | Partial; retained-tail model unproven vs pi entries | Full human log + derived view (`crates/epsilon-session/src/store.rs:1161-1213`); `EntryKind` lacks `label`/`session_info` (`crates/epsilon-contracts/src/session_entry.rs:93-155`) so such a file is **refused** (`store.rs:610-612`) | **still open** | add `Label`/`SessionInfo` variants (+ ignore-unknown read path) |
| 7 | Cross-branch nav, branch summaries, fork (`:34`) | Partial; no BranchSummary producer | Producer exists (`store.rs:933-962`), model view projects it (`:1175-1188`), `fork_branch_at` exists (`:1042`); the TUI calls none of them (`crates/epsilon-tui/src/app.rs:1458-1489`, `:1606-1621`) | **still open, one sub-claim wrong** | make `/tree` list all entries and `/fork` call `fork_branch_at` |
| 8 | Compaction and pressure accounting (`:35`) | Intentional; F04/F06/F07 open | 6-section template is sent (`crates/epsilon-core/src/compact.rs:229-274`), `MaxTokens` summary refused (`:294-297`), projection replaced not appended (`store.rs:1198`); F07 live (`lib.rs:987`, `:1003`) | **F04/F06 closed, F07 open** | surface `persist`/`pop_last` failures instead of `let _ =` |
| 9 | Provider protocols and authentication (`:36`) | Partial; "generic Responses does not supply Codex" | Wires: Anthropic, OpenAI-compat, **OpenAI Responses**, Mistral, Copilot, Google/Gemini, Vertex, Bedrock (`crates/epsilon-llm/src/providers/mod.rs:94-175`); logins: anthropic, openrouter, xai, copilot (`crates/epsilon-llm/src/oauth_login.rs:141-197`) | **partly wrong, still open** | the `openai-codex` login flow (`oauth_login.rs:216`) |
| 10 | Extension lifecycle and SDK (`:37`) | Partial: no commands/shortcuts/provider registration/UI | Manifest is name/version/bin/args/tools/events (`crates/epsilon-ext/src/manifest.rs:20-37`); host exposes spawn/send_event/logs/invoke (`crates/epsilon-ext/src/host.rs:85-184`) | **still open** | no small slice — needs a command + UI-event surface |
| 11 | RPC embedding (`:38`) | Partial: "six operations; no abort… or request ID" | Nine ops incl. `abort`, `get_state`, `set_thinking_level` with `id` echo and pi's envelope (`crates/epsilon-cli/src/rpc_mode.rs:52-80`, `:295-350`) | **mostly wrong, still open (narrower)** | model-switch ops + session/tree ops (`rpc_mode.rs:59-66` names them) |
| 12 | Resource loading / package lifecycle (`:39`) | Partial; no npm/git install | `SYSTEM.md`/`APPEND_SYSTEM.md` load, project shadows global (`crates/epsilon-core/src/prompt.rs:327-350`); no install/update/remove anywhere in `crates/` (grep) | **still open (smaller)** | nothing loads `.epsilon/themes/` either (`crates/epsilon-cli/src/runtime.rs:471-511`) |
| 13 | Markdown prompt templates (`:40`) | **Missing** | Only the trust gate names them (`runtime.rs:468`, `:486`); no substitution or loader in `crates/` (grep for `$ARGUMENTS`, prompt templates: none) | **still open, claim accurate** | port `substituteArgs` + a `prompts/*.md` loader |
| 14 | Skill discovery / frontmatter / collisions (`:41`) | Partial; collisions differ | One-level scan (`crates/epsilon-core/src/skills.rs:66-114`), flat `key: value` frontmatter (`:38-57`), later root **replaces** (`:104-109`), invalid name **skipped** (`:89-95`) | **still open, claim accurate** | recursive scan + collision diagnostic |
| 15 | Configuration breadth (`:42`) | Partial; queue/retry/resource/theme knobs undisposed | `config.rs` holds model id, recents, favourites, favourite-thinking, prompt history, trust hash only (`crates/epsilon-core/src/config.rs:13-463`) — no compaction/queue/retry key | **still open** | a `[compaction]`/`[queue]` table read via `load_config_string_in` (`config.rs:20`) into `LoopLimits` |
| 16 | TUI / editor / themes (`:43`) | Partial; parity not established | `crates/epsilon-tui/src/ui/` owns theme/layout/overlay (`lib.rs:12-29`); the cited `theme.rs` no longer exists; editor primitives persist (`editor_ops.rs`) | **still open, citation stale** | scope decision, not a patch — the hedge is still the honest answer |
| 17 | Image input / processing / rendering (`:44`) | Partial; no resize pipeline or terminal graphics | read attaches a base64 image block (`crates/epsilon-tools/src/files.rs:129-156`), MIME from magic bytes (`image.rs:12-37`), BMP omitted (`files.rs:130-136`); **no resize**; no graphics escape anywhere (grep for kitty/sixel/iterm: none) | **still open, claim accurate** | resize before attach (pi `read.ts:113`, `utils/image-process.ts:86-87`) |

## 2. The still-open rows, ranked by what a user notices

> **Status re-checked 2026-09-26. Items 1 and 2 below are STALE — both have since been fixed,
> and item 1 was this document's loudest claim.** See
> [`../../agents/memory/next-hardening.md`](../../agents/memory/next-hardening.md) for the rule
> that produced the second correction. **Item 3 re-verified and still accurate** (see below).
> Items 4+ were not re-read.
>
> **Item 3, re-verified 2026-09-26 — the claim holds, the count was imprecise.** It is **11**
> built-in registrations in `register_builtins_with` (`crates/epsilon-tools/src/lib.rs`: Read,
> Write, Edit, Bash, Grep, Glob, Find, Ls, WebFetch, Todo, Task), **plus** `SkillTool`
> registered only when skills are non-empty — so 11, or 12 with skills loaded, not "twelve …
> plus conditional skill", which would be 13. pi's default active set is confirmed still
> `["read", "bash", "edit", "write"]` (`pi-0.85.1/.../core/agent-session.ts:2808–2810`).
> Epsilon's mechanism is `crates/epsilon-core/src/lib.rs:2593`: agent-type routing treats
> `default` as "full registry". So the model really does see every tool by default, against
> pi's four. That is a deliberate divergence with a real cost — more tool schemas in every
> request — and it is **not** recorded as a divergence anywhere in `03-port-plan.md`, which
> describes tool parity as a count question instead. Worth a disposition.
>
> Related and checked while there: `EntryKind::ActiveToolsChange` exists in the session format
> (pi parity) but **nothing in Epsilon ever constructs it** — only the variant, the
> `KNOWN_ENTRY_KINDS` guard and a test fixture. Like `BranchSummary`, it is a read path with no
> writer, which is defensible for reading pi sessions, but pi's own default set is exactly what
> this variant would record and Epsilon never records it.

**1. ~~A pi session file can refuse to open, and cannot carry labels (row 6).~~ CLOSED since
2026-09-21.** `EntryKind` now has a catch-all that preserves what it does not understand:
`EntryKind::Unknown { type_name: String, body: serde_json::Value }`
(`crates/epsilon-contracts/src/session_entry.rs:263`), with `is_unknown()` at `:347` and the
round-trip test `an_unknown_kind_keeps_every_field_and_value` at `:474`. An unrecognised
`kind.type` therefore no longer becomes `StoreError::corrupt`; it is kept verbatim so a later
write reproduces the same object. The line the entry occupies is *not* preserved as raw text
(`seq` is the file index and every rewrite re-serializes), and key order is not preserved — the
comment at `:255` says so and is right about the limit. **The "loudest failure this harness
has" is no longer true.** The `label`/`session_info` gaps are a separate matter and were not
re-checked.

**2. ~~Branch navigation is a timeline, and fork does not move you (row 7).~~ HALF STALE.**
"nothing on the user path calls them" is no longer true: the timeline picker calls
`SessionStore::navigate_to` on Enter — `crates/epsilon-tui/src/app/keys.rs:457` runs
`st.navigate_to(&id).is_ok()`, resolved from `store.branch_entries()` by `seq`. Navigation is
wired. The remaining half is accurate: `branch_with_summary` still has **no production caller**,
so a jump records nothing about the abandoned path, and `fork_branch_at` is still not reached
from the UI. That is the same gap F22 records, and it is a *missing summarization on navigation*,
not a missing navigator.

*(The original text of items 1 and 2 is superseded by the corrections above; it is quoted in this file's history rather than repeated here, because leaving a refuted claim in place is how it gets re-read as current.)*

**3. The model is handed twelve tools where pi hands four (row 5).**
pi's default active set is `["read", "bash", "edit", "write"]`
(`packages/coding-agent/src/core/agent-session.ts:2808-2810`); its eight built-ins are bash, edit,
find, grep, ls, powershell, read and write (`core/tools/index.ts:84-91`), and the last four are
opt-in. Epsilon's product path registers read, write, edit, bash, grep, glob, find, ls, webfetch, todo,
task plus `skill` when skills exist (`crates/epsilon-tools/src/lib.rs:128-157`,
`crates/epsilon-cli/src/main.rs:1574-1587`) with no subsetting. Two smaller mismatches ride along:
pi's `ls` takes a `limit` and says "N entries limit reached" (`tools/ls.ts:13`, `:146-148`) while
Epsilon's takes only `path` (`crates/epsilon-tools/src/search.rs:489-493`); `find` truncates silently
(`crates/epsilon-tools/src/find.rs:109-118`) where pi emits a notice (`tools/find.ts:281-285`).
**What the user gets:** a larger tool block in every prompt, more ways for the model to reach past the
four verbs the contract is written around, and a `find` that looks complete when it is not.

**4. `.epsilon/prompts/*.md` is gated but never loaded (row 13).**
pi expands `prompts/*.md` slash templates with `$1`/`$@`/`$ARGUMENTS` and `${N:-default}`
substitution (`packages/coding-agent/src/core/prompt-templates.ts:57-70`) from a global and a project
directory (`:188-194`). Epsilon enumerates the directory only to decide whether the project needs a
trust prompt (`crates/epsilon-cli/src/runtime.rs:468`, `:486`); no reader or substitution exists
anywhere in `crates/`. **What the user gets:** the worst combination — the harness asks them to trust
a directory, then silently ignores everything in it.

**5. A failed durable write still leaves memory and the model disagreeing (row 8).**
`persist` discards the append result (`crates/epsilon-core/src/lib.rs:987`) and `pop_last_message`
discards the pop result (`:1003`), while the provider request is built from the store projection. If
the disk fills or the append fails, the transcript on screen advances and the model's view does not
(or vice versa). **What the user gets:** the model answering from an older conversation than the one
they can see, with nothing said about it — a silent divergence, not an error. This is the only
half of the compaction row still live; F04 (repeat compaction duplicated old summaries) is closed by
the replace rule (`store.rs:1198`, `audit/02-defect-ledger.md:92-106`) and F06 by the empty and
`MaxTokens` refusals (`crates/epsilon-core/src/compact.rs:294-299`).

**6. Loop tuning is not configurable (row 15).**
`LoopLimits` carries queue modes, turn cap, compaction thresholds and guards
(`crates/epsilon-core/src/lib.rs:151-189`), but `config.rs` reads only the model id, picker recents
and favourites, favourite thinking levels, prompt history and the trust hash (`config.rs:13-463`). pi
exposes compaction (`enabled`, `reserveTokens`, `keepRecentTokens` — `settings-manager.ts:13-17`),
branch-summary and retry settings the same way. **What the user gets:** no way to raise the compaction
threshold or change queue delivery for a large repo without editing source.

**7. Skills are found one level deep and resolved unlike pi (row 14).**
pi recurses into subdirectories until it finds a `SKILL.md` (`packages/coding-agent/src/core/skills.ts:255-259`);
Epsilon reads direct children of each root only (`crates/epsilon-core/src/skills.rs:66-114`). On a
name collision pi keeps the first and reports a `collision` diagnostic (`skills.ts:432-439`); Epsilon
replaces the earlier entry and reports nothing (`skills.rs:104-109`) — and the doc comment there
claims that rule mirrors pi, which it does not. pi also loads a skill with an invalid name and warns
(`skills.ts:324-332`); Epsilon drops it silently (`skills.rs:89-95`). **What the user gets:** a skill
nested under a category directory simply never appears, and an override behaves backwards from pi's —
with no diagnostic either way.

**8. Resources can be read but never installed (row 12).**
pi has a resource loader covering extensions, skills, prompts and themes (`resource-loader.ts:40-50`)
plus a package manager driven by `packages` in settings (`package-manager.ts:978-999`). Epsilon loads
skills, extensions, `AGENTS*.md`, `SYSTEM.md` and `APPEND_SYSTEM.md`
(`crates/epsilon-core/src/prompt.rs:247-251`, `:327-350`) but has no install/update/remove path for
anything — a grep of `crates/` for npm/git resource installation finds only unrelated matches — and
`.epsilon/themes/` is trust-gated with no loader (`runtime.rs:471-511`). **What the user gets:** the
only way to add a capability is to place files by hand, and a project theme is asked about but never
applied.

**9. No ChatGPT-subscription (Codex) transport; no Azure or Cloudflare provider (row 9).**
The wire layer is broader than the plan says — an OpenAI Responses wire exists
(`crates/epsilon-llm/src/providers/openai_responses.rs`, dispatched at `providers/mod.rs:99-102`) and
its module doc names `openai`, `openai-codex` and `azure-openai-responses` (`:10`). But the OAuth
login surface has only anthropic, openrouter, xai and github-copilot
(`crates/epsilon-llm/src/oauth_login.rs:141-197`), and the code deliberately withholds `openai-codex`
until model data lands (`:133-137`; asserted at `crates/epsilon-llm/tests/oauth_login.rs:470-476`).
pi ships `azureOpenAIResponsesProvider` and the Cloudflare providers (`packages/ai/src/providers/all.ts:94-98`).
**What the user gets:** a Codex subscription cannot be used at all, and a catalog row alone will not
make one work.

**10. RPC is an editor protocol without model or session operations (row 11).**
Nine ops plus an explicitly-refused catch-all (`crates/epsilon-cli/src/rpc_mode.rs:30-68`), pi's
envelope and an echoed `id` (`:71-80`, `:295-350`). Missing, and named as missing in the source:
`set_model`/`cycle_model`, queue modes, compaction, and session/fork/tree/export/messages
(`rpc_mode.rs:59-66`). **What the user gets:** an editor extension can prompt, steer and abort, but
cannot switch models or open another session — and a request for those answers `unknown op` rather
than doing something plausible.

**11. Extensions can run tools, not contribute UI or commands (row 10).**
Manifest fields are name, version, bin, args, tools and forwarded events
(`crates/epsilon-ext/src/manifest.rs:20-37`); the host spawns, forwards events, collects logs and
invokes tools (`crates/epsilon-ext/src/host.rs:85-184`). pi's extension surface includes message and
entry renderers and markdown transformers (`packages/coding-agent/src/core/extensions/types.ts:1201-1223`).
M103 added tests, not capability (`git log -- crates/epsilon-ext`: `5bfd560`, `74ed230`).
**What the user gets:** an extension can be a tool provider, never a first-class UI participant.

**12. Image bytes are attached unresized (row 17).**
read detects JPEG/PNG/GIF/WebP (and refuses BMP) by magic bytes (`crates/epsilon-tools/src/files.rs:119-136`,
`crates/epsilon-tools/src/image.rs:12-37`) and attaches the original bytes as base64 (`files.rs:138-156`).
pi resizes by default (`tools/read.ts:69`, `:113`; `utils/image-process.ts:86-87`). No terminal
graphics protocol appears anywhere in `crates/`. **What the user gets:** a phone-sized screenshot can
blow the request budget or be rejected by the provider, and no image is ever shown in the terminal.

**13. TUI parity is still a hedge, not a measurement (row 16).**
The presentation layer was rebuilt (`crates/epsilon-tui/src/ui/`, module list at `lib.rs:12-29`) and
`crates/epsilon-tui/src/theme.rs` — the plan's citation — is gone. pi's TUI exports components
including `Image`, `Editor`, `Markdown` and `ScrollView`
(`references/pi-0.85.1/packages/tui/src/index.ts:13-30`). **What the user gets:** nothing specific
today; this row is a scope decision that should be restated or closed, not carried as a stale hedge.

## 3. Claims in `audit/03` / `audit/11` that are now factually wrong

- **W1 — the reference snapshot does not exist.** `audit/03-port-plan.md:4` and `:22` name
  `references/pi-0.84.2/`; that directory is absent (the only pi checkout is
  `references/pi-0.85.1/`). **21** citations in that file point into it, so no pi claim in the port
  plan can be checked as written.
- **W2 — the baseline is 129 commits stale.** `audit/03:4` pins "current Rust working tree over
  `e200815`, 2026-09-16"; `git rev-list --count e200815..HEAD` = 129 (HEAD `d56ee8e`).
- **W3 — "Documented commitment not fulfilled" is fulfilled.** `audit/03:52` (repeated at `:30`)
  says ADR-002's next-turn/stop callbacks are unmet. They exist: `crates/epsilon-contracts/src/turn.rs:129-173`,
  called at `crates/epsilon-core/src/lib.rs:1584` and `:1615`, with the ADR amended at
  `docs/design/epsilon-decisions.md:57-73`.
- **W4 — the ADR citation drifted.** `audit/03:30` cites `docs/design/epsilon-decisions.md:36-40`
  for ADR-002's promise; those lines are ADR-001's closing paragraph, and ADR-002 begins at `:44`.
- **W5 — the queue-default mismatch is closed.** `audit/03:29` ("Rust drains all follow-ups by
  default") no longer holds: `lib.rs:196-200` defaults both queues to `OneAtATime` (M85 `3afafd3`).
- **W6 — F01 no longer blocks tool confidence.** `audit/03:32` says the large-file defect blocks
  confidence; the ledger records it fixed 2026-09-17 (`audit/02-defect-ledger.md:14`), and M95 added
  paged reads with an explicit partial-view notice (`crates/epsilon-tools/src/util.rs:292-352`).
- **W7 — "no producer was found" is wrong.** `audit/03:34` says `BranchSummary` has no producer;
  `SessionStore::branch_with_summary` writes one (`crates/epsilon-session/src/store.rs:933-962`) and
  `model_view` projects it (`:1175-1188`) — M90 `e663bd3`.
- **W8 — RPC has abort and request IDs.** `audit/03:38` ("Six operations; no abort… or request ID")
  is contradicted by `crates/epsilon-cli/src/rpc_mode.rs:52-58` (`Abort`), `:71-80` (correlation
  header) and `:333-338` (abort reply) — M92 `a238ce5`.
- **W9 — an out-of-range pi citation.** `audit/03:44` cites `tools/read.ts:209-268` for pi's image
  handling; that file is **186 lines**. The behaviour is real but lives at `read.ts:69`, `:113` and
  `utils/image-process.ts:86-87`.
- **W10 — a deleted Rust file is still cited.** `audit/03:43` cites `crates/epsilon-tui/src/theme.rs:1-25`;
  the file does not exist (`crates/epsilon-tui/src/lib.rs:12-29` lists `ui`, and the theme lives at
  `crates/epsilon-tui/src/ui/theme.rs`).
- **W11 — the plan's suspicion about the skills comment is confirmed, and worse than stated.**
  `audit/03:41` doubts the "exact collision parity" comments. `crates/epsilon-core/src/skills.rs:65`
  says later roots shadow earlier ones "mirroring pi"; pi keeps the **first** name and emits a
  collision diagnostic (`references/pi-0.85.1/packages/coding-agent/src/core/skills.ts:432-439`). The
  code comment is the false claim, and it is still in the tree.
- **W12 — `audit/11`'s `@`-completion bullet is stale.** `audit/11-current-assessment.md:164-169`
  states the `@` popup keeps its own skip list and "suggests files `.gitignore` ignores", calling the
  fix "queued, not done". M109 (`0afa121`) did it: `App::at_file_rows` loads
  `epsilon_core::ignore::IgnoreSet` (`crates/epsilon-tui/src/app.rs:1009-1018`) and the walk takes
  that matcher (`crates/epsilon-tui/src/at_files.rs:43`, `:85-103`).

## What I did not check

- **Nothing was executed.** No cargo command, no test, no binary. Every claim is read from source,
  so "the code now does X" means the source says X, not that a run produced X.
- **F03's SSE/EOF code** (`audit/02:16`) was not re-read; row 4's pointer to it is reported as
  "still listed open in the ledger", nothing more.
- **F04 and F06** were verified only at their commit points — the replace rule
  (`store.rs:1198`) and the two summary refusals (`compact.rs:294-299`). I did not run the
  multi-compaction scenarios, and I did not re-read the ledger's remaining test-coverage items.
- **Provider wire fidelity** was not compared field by field: I enumerated wires and login flows
  (`providers/mod.rs:94-175`, `oauth_login.rs:141-197`) and confirmed the absence of Codex/Azure/
  Cloudflare, but I did not read the request/response shapes of the wires that do exist.
- **`register_builtins_with_skills`** (`crates/epsilon-tools/src/lib.rs:98-108`) has no caller —
  `crates/epsilon-cli/src/main.rs:1583-1587` registers `SkillTool` itself. I did not decide whether
  that is intentional; it looks like a seam that was bypassed.
- **pi's `harness/runtime/drive/*` generation machinery** (a second, richer loop) was not opened.
- **`epsilon-mcp`, `epsilon-lsp`, `epsilon-safety`, `epsilon-server`** were read only where a cited
  row touched them; `epsilon-server`'s HTTP surface was not re-checked against `audit/11`'s F13
  claims beyond the ledger's own text.
- **Rows 4, 16 and parts of 5** are scope judgements as much as code facts: "complete component,
  theme or key-by-key parity not established" cannot be falsified by reading, only by an owner
  decision on what parity means.
