# Architecture review and forward plan

**Date:** 2026-10-22. **Scope:** the whole repository and all six vendored references.
**Status:** the research phase is complete; this document is the plan it produced.

This is the record of a review, not a history. The findings below were re-derived at the
cited anchors; where a claim could not be opened and checked, it says so.

> **2026-10-23 file-structure note.** The legacy `audit/`, `agents/memory/`,
> `docs/milestones/` and `docs/plans/` trees were removed as part of the structure
> cleanup. References to those paths below are historical citations from the review,
> not current documents. The current tree is the numbered docs, `design/`, `user/`,
> `parity/` and `references/`.

---

## 0. What has been done so far

**The review is complete. Seven defects have been fixed and verified**; the rest of
the plan is open. This section is the reconciliation — the waves below mark what is
done and what is not.

| Fixed | Finding | Evidence |
|---|---|---|
| The quit path had two implementations, and one was dead | F1 | `cargo clippy --workspace --all-targets -- -D warnings` clean |
| A refused exit was invisible under an open file viewer | F1 | frame test `a_refused_quit_says_so_on_the_frame_under_a_file_viewer`; the frame was rendered and read at 80×24 |
| The settings writer's staging name was unique per *process*, not per *write* | F25 | test `concurrent_writes_to_one_settings_file_use_distinct_staging_paths`, verified RED against the old naming and GREEN against the new |
| **Eleven registered tools asked for permission in a default session** | **F17, F24, F5** | test `every_registered_tool_is_free_by_default_or_explicitly_ask_first`, RED→GREEN; the free set is now derived from one list, and a tool declares the capability it acts under |
| **The rule-matching loop existed in three copies** | **F18** | the gate now composes plan + base + preset into one engine and evaluates once; `glob_match` no longer appears in `gate.rs`; test `the_gate_and_the_composed_engine_agree` |
| Two parity rows claimed Epsilon lacks a session delete it has | F33 | `docs/parity/pi-tui.md` P-070 and P-236 rewritten; the area summary that contradicted them rewritten too |
| `02-ARCHITECTURE.md` described a manifest edge that is gone | F34 | the `epsilon-cli` → `epsilon-ext` paragraph now says the edge was removed |
| `TUI-DECISION.md` understated the switching cost by 1.75× and cited a deleted crate | F35 | re-measured on this tree: 66,482 lines / 57 files / 689 tests |

**The battery was green on the tree as it stood at the review snapshot
(2026-10-22).** All 14 legs; **2,618 passed / 0 failed / 19 ignored** over 77 test
binaries, terminal suite **902 passed / 0 failed / 18 ignored**. Evidence:
`target/gate-f17b-20261022.log`. This is a historical measurement; current numbers
are in [`04-STATUS.md`](04-STATUS.md).

Two things the review found are **not** fixed and are the top of the backlog:
**F16** (the repository is one unwaved change — 600 changed files that must be split
before any of it can be reviewed) and **F22** (`plan_ruleset()` is a stale four-name
deny-list still live beside the newer read-only allowlist).

**F17 and F18 are fixed** — the two highest-ranked permission defects. The free set
is derived from one list, a tool declares the capability it acts under, and the
rule-matching loop now exists once instead of three times.

**A second reference pass landed after the review was written** (§2b) and added four
findings: **F26** (a subagent's result body has no size cap), **F27** (a failed tool
call is repaired only when the session is reopened), **F28** (the permission policy is
never told to the model) and **F30** (pi's prompt-in-transcript shape). It also
reported **F29**, which was **false** and has been corrected in place: the tools
`write.rs` and `edit.rs` do not exist — both live in `files.rs` and both call the
stale-read guard. Most of what the pass reported, Epsilon already has.

---

## 1. What was reviewed

### 1.1 The references

| Reference | Depth reached | Method |
|---|---|---|
| `pi-1.3.0` | agent loop, context assembly, tool model, sessions, extensions, skills, permissions, TUI event/layout model, configuration, reliability | 65 files opened, incl. `packages/agent/src/agent-loop.ts` in full |
| `deepseek-harness-dsh-v0.2.1-rc.2` | agent core, cordis plugin kernel, tool system, workspace/session model, approval flow, front-end sync, subagents, reliability | core packages opened; `vendor/cordis` semantics read |
| `opencode-1.18.34` | session/prompt core, tool layer, permission engine, LSP, editing/provenance, plugin system, config, reliability | `packages/core/src/*` opened; permission matching code read |
| `hermes-agent-2026.9.24` | turn model, tool registry, approvals, skills/plugins, delegation, memory/context, persistence, front-end contract | `agent/`, `tools/`, `gateway/` opened; cross-checked against Epsilon |
| `opentui-0.5.14` | Zig-core/TS layering, renderer, layout, keymap separation, agent conventions | `packages/core`, `packages/keymap` opened |
| `codewhale-ratatui-main` | component kit, responsive degradation, approval/diff views, terminal profiles | `src/`, `DESIGN.md`, `VIEWS.md` opened |

### 1.2 Epsilon

All 13 crates (198,015 lines of Rust across 241 files), the 9 repository checkers, the
14-leg battery, the 42-row command registry, the 42-id terminal keybinding table, the
339-row Pi parity matrix, the documentation set, and the inherited dirty tree
(485 deletions, 112 modifications, 11 untracked).

### 1.3 What was measured, not quoted

- Battery `gate-commands2-20261018`: **2,606 passed / 0 failed / 19 ignored** over 77 test
  binaries, parsed from the log; the terminal suite is **895 passed** (keyed by binary
  name, because every crate's lib target prints the same `Running unittests src\lib.rs`).
- `epsilon-tui`: 66,407 lines / 57 files. `app/` is 55% of it (36,683), `ui/` 17% (11,015).
- `SLASH_ITEMS` = **42** rows, `COMMAND_HEADS` = **41** heads, `ALIAS_HEADS` = **9**,
  `TERMINAL` = **42** ids, `WINDOW` = **55** ids with **zero** shared ids and **zero**
  production readers. The 42 `SLASH_ITEMS` rows cover the 41 heads exactly, plus one
  extra row for the `models` alias — which is F4.
- Pi's registry: 24 builtin slash commands, 47 keybinding ids. Epsilon binds 42 of its own
  ids, of which **24** have a direct pi counterpart; 31 pi ids (the session-tree filter
  family, the model-list management family, the session-management family) have no
  Epsilon counterpart.

---

## 2. The verdict

**Epsilon's architecture is sound; its problems are in the edges.** The core
(`epsilon-core`), the safety engine (`epsilon-safety`), the session store
(`epsilon-session`) and the provider layer (`epsilon-llm`) are better than their
references in the dimensions that matter: one permission gate over every front end, a
monotonic deny, atomic config writes, a durable session mirror that refuses to serve a
projection it cannot vouch for.

Four things are wrong, and they are the whole plan:

1. **The tree does not compile clean.** `clippy -D warnings` fails on two dead items
   introduced by the inherited slice. The battery's clippy leg exits 101 today.
2. **The TUI has four parallel ladders over the same set of surfaces** — key order, paint
   order, wheel order, footer order — and they have already drifted apart.
3. **The capability that the permission engine matches is the tool's own name**, so a
   third-party tool cannot join an existing capability without a new user rule.
4. **The documentation consolidation deleted the evidence that the documentation cites**,
   leaving dozens of dangling references, a third of them in the two files a reviewer
   trusts most.

And one thing is wrong in the *core*, which outranks all four:

5. **Eleven registered tools fall through to `Ask` in a default session.** The default
   ruleset allows nine capability names; the registry registers twenty-one. Everything
   not named is `Ask` — so `find`, `todo`, `task`, `skill`, `lsp`, `question`, the three
   goal tools, `apply_patch` and one more all ask the user for permission the first
   time they run, in a session that granted nothing. F17 below.

---

## 2a. The core finding, in full

### F17 — Eleven registered tools ask for permission in a default session — **FIXED 2026-10-22**

**The fix, in three parts:**

1. **One list, not two.** `READ_ONLY_CAPABILITIES` is now the single source of the
   read-only classification, and `default_ruleset()` *derives* its free set from it
   (`engine.rs`). The hand-written nine-name list is gone, so the two lists can no
   longer disagree.
2. **A capability on the tool** (F5). `Tool::capability()` defaults to the tool's
   own name — so every existing rule keeps matching what it matched — and
   `apply_patch` overrides it to `edit`, so a rule the user wrote for `edit` covers
   the patch tool. The gate now matches the *declared capability* rather than the
   name, in both the direct path (`exec.rs`) and the batch path.
3. **A test that reads the live registry.** `epsilon-safety`'s
   `every_registered_tool_is_free_by_default_or_explicitly_ask_first` builds the
   real tool set with `epsilon_tools::builtin_registry` and asserts that the only
   tools that ask are the ones that ask **by decision** — `bash` and `webfetch`,
   the two that reach outside the workspace. A new tool that lands and asks has to
   say why, in a list a reader will see.

**Verified RED → GREEN.** The RED run failed with the defect named:

```
assertion `left == right` failed: create_goal only reads, so the defaults must not ask for it
  left: Ask
 right: Allow
```

The GREEN run reports `56 passed; 0 failed` for the `epsilon-safety` library.

**What it means for a user.** A session that granted nothing now meets a permission
dialog for the shell and the network, and for nothing else. `find` — a *read* — and
`todo` — a tool that cannot touch the workspace — run without asking. A user who
meets a dialog for everything learns to approve everything; a user who meets two
honest dialogs keeps the gate meaningful.

`crates/epsilon-safety/src/engine.rs:112-133` — the default ruleset names **nine**
capabilities:

```
read · grep · glob · ls · write · edit · external_dir · bash · webfetch
```

`crates/epsilon-tools/src/lib.rs:166-226` registers **twenty-one** tools. The eleven
whose names are not in that list fall through `PermissionEngine::evaluate`'s
`decision.unwrap_or(Action::Ask)` (`engine.rs:101-103`) to **`Ask`**: `find`, `todo`,
`task`, `skill`, `lsp`, `question`, `get_goal`, `create_goal`, `update_goal`,
`apply_patch`, and one more.

**Measured, not inferred** — the registered names were read from each tool's
descriptor and compared against the ruleset's rule set:

```
default free-set:  bash edit external_dir glob grep ls read webfetch write
registered, NOT in the free-set:  alpha apply_patch create_goal find get_goal
                                  lsp question skill task todo update_goal
```

**Why it matters.** A user who has granted nothing still meets a permission dialog for
`find` — a *read* — and for `todo` — a *session-state* tool that cannot touch the
workspace. Every one of those dialogs is a false alarm: the user learns that the
harness asks about everything, and starts approving reflexively, which is the exact
failure the permission gate exists to prevent. It also means the **read-only
allowlist** (`engine.rs:172-191`, fourteen names) and the **default free-set** (nine
names) are two hand-maintained lists of the same thing, and they disagree — which is
the class of defect the audit called out as F1 and F2 on the safety side.

**Fix shape.** Derive the default free-set from `is_read_only_capability` rather than
hand-listing it, and add a test that **every registered tool name is either free by
default or explicitly ask-first** — a table the test reads, so a new tool cannot be
forgotten. That single test would have caught this the day the eleventh tool landed.

**This is a security-adjacent defect, so it is first in Wave 3.**

### F18 — The permission engine's rule evaluation is duplicated — **FIXED 2026-10-22**

`crates/epsilon-safety/src/gate.rs` held its own copy of the rule-matching loop —
**twice**, once for the plan layer and once for the preset layer — beside the
engine's copy in `PermissionEngine::evaluate`. Three copies of "does this rule match,
and what does it decide" is three places for the same question to be answered
differently.

**The claim in the first draft of this review was wrong, and the correction matters.**
It said the two copies *"disagree about what a preset rule means when the base also
matched"*. They do not: on ordinary matching the gate's three-phase evaluation and
the engine's flat last-match-wins reach the same answer, which is why nothing had
broken. **The real divergence is narrower and latent:** the engine lets a layer's
*allow* win, while the gate's plan loop honoured only *denies* — so a plan-layer
allow was **inert** in the gate and would have **won** in the engine. `plan_ruleset()`
has only denies today, so the divergence could not fire; it was a trap for whoever
added an allow to the plan layer next.

**The fix.** The gate now composes **plan + base + preset** into one engine and
evaluates it once:

- `PermissionEngine::with_layers(layers)` builds an engine over exactly the given
  layers, and `PermissionEngine::layers()` reads the stack back so the gate can
  rebuild one.
- `RulesetGate::evaluate` composes the plan layer first, the base stack next, the
  preset last — the same order it used — and calls the engine's `evaluate` once.
- The read-only pre-check stays where it was: it is a **posture**, not a rule, and
  it has to run before any of them.

**Measured, after the change:** `glob_match` and the hand-rolled capability
matching no longer appear in `gate.rs` at all. The rule matching now lives in
`Rule::matches` (`engine.rs:42`) and is called from exactly one place
(`engine.rs:123`). The gate's hand-rolled loops were 43 lines; they are gone.

**The invariant is now tested.** `the_gate_and_the_composed_engine_agree` walks every
posture the gate can be in (`auto`, `workspace-write`, `read-only`,
`danger-full-access`, and plan mode) against eight capability/subject pairs, and
asserts the gate's decision matches what the composed engine says. If a second copy
of the matching logic ever comes back, that is where the two would be seen to differ.

**Why this was still worth fixing.** The duplication was not wrong today; it was
**twice wrong waiting to happen** — one in the plan layer's inert allows, and one
in whatever the next person writes in the second copy.

### F19 — The production retry path reports on stderr; the copy that reaches the UI is unreachable

`crates/epsilon-runtime/src/lib.rs:9091-9098` wires `Retrying` with an
`eprintln!("[retry] …")` hook. `crates/epsilon-core/src/run_.rs:45-59`
(`Agent::retrying_real_stream`) wires the *same* policy with an
`AgentEvent::Notice` hook — and it is behind `#[cfg(feature = "real")]`, so the
runtime's stderr version is the one that runs.

**Consequence:** a user watching the terminal IDE sees a retry only if the process
writes to stderr, which the TUI does not surface. The TUI's own notice path — the one
that would say "retrying" on the frame — is dead in production.

**Fix shape.** One `Retrying` constructor taking a notice sink; the runtime's hook
routes to the agent's event channel.

### F20 — Context-overflow recovery matches 12 English needles, and throws away the provider's code

`crates/epsilon-core/src/util.rs:273-290` detects an overflow by substring match.
`crates/epsilon-llm/src/providers/openai_compat.rs:229-234` and
`anthropic.rs:264-273` **parse the structured error code**
(`context_length_exceeded`) and then discard it.

**Fix shape:** keep the code and the type; check them first, and keep the substring
match as the fallback for a provider that sends neither.

### F21 — The declared JSON Schema is never enforced at the tool boundary

`crates/epsilon-contracts/src/tool.rs:66-85` — `basic_validate` checks only that the
arguments are an object. The schema the tool declares is not consulted, so a call
missing a required field reaches the tool's `execute` and fails there, with a
tool-specific error instead of a validation error.

**Fix shape:** validate `required` and the primitive types of `parameters_schema` in
`basic_validate`. The `schemars` crate is not needed for this — the schema is already
a `serde_json::Value`.

### F22 — `plan_ruleset()` is a stale four-name deny-list kept beside the newer allowlist

`crates/epsilon-safety/src/engine.rs:143-155`; installed at `gate.rs:324-331`;
evaluated at `gate.rs:148-158`. `docs/user/permissions.md:21-32` describes exactly this
rule set as the *pre-fix* state — and the code still evaluates it. There are two
answers to "what does plan mode forbid?": the layer says four names, the allowlist
says "anything not on the list". They agree today only because the allowlist runs
first.

**Fix shape:** delete the `plan` ruleset layer and its field; make `plan_on` a boolean
that selects the allowlist path. Then there is one answer.

### F23 — There is no tool-result budget for a turn

`crates/epsilon-core/src/run_.rs:482-598` performs no accounting; the prune pass in
`compact.rs:361-404` runs only *inside* a compaction. The harness caps each tool
result but not their **sum** — the only 64 KiB cap in the tree is on file references
(`references.rs:61`), not on tool output.

A model that runs twenty reads in one batch produces a request past the window, and
the only recovery is the overflow path (`run_.rs:432-465`), which pays for a
summarization call to fix something the harness could have noticed for free.

**Fix shape:** track the turn's accumulated result bytes beside `usage_anchor`; when it
exceeds `limits.compaction_reserve_tokens`, run the existing prune pass **before** the
results are pushed, and report it in the `Notice`.

### F24 — `is_read_only_capability` is `pub(crate)`, so the core cannot ask

`crates/epsilon-safety/src/engine.rs:172`. The core derives new capabilities —
`external_dir` (`exec.rs:190`), `no_progress` (`exec.rs:622`), `doom_loop`
(`exec.rs:688`) — and cannot consult the list that says which of them are read-only.

**This is the root cause of F17.** A capability added to the tool set requires editing
a private list in another crate, which is exactly why the list went stale.

**Fix shape:** make it `pub`, and let the core and the default free-set read one
definition.

### F25 — The settings writer's staging name is unique per *process*, not per *write*

`crates/epsilon-core/src/config.rs:201-209` (now `staging_path` at `:197-215`).

```rust
let staging = path.with_extension(format!("toml.tmp.{}", std::process::id()));
```

A pid is unique per process. Two threads in **one** process writing the same file
share one staging path: the first renames it into place, the second's rename fails
with `os error 2` — *"the system cannot find the file specified"* — and the caller
reports a write failure for a file that was perfectly fine.

**This is not hypothetical; it failed the battery.** The first post-fix gate run to
reach the test leg (`gate.sh fix2-20261022`) failed
`alt_enter_while_streaming_queues_a_follow_up` (`tests.rs:11350`) with:

```
the reader must be told it waits: "history not saved: replace
C:\Users\Hp\.config\epsilon\history: The system cannot find the file specified.
(os error 2)"
```

The prompt history is written from the TUI's submit path (`editing.rs:346`), so
**two submits racing in one session lose one of them** — a real user-visible
defect, not only a test artifact. The test caught it by accident: it read the
*first* notice off the channel and got the failed write instead of the notice it
was waiting for.

**Fix shape:** a monotonic per-process counter in the staging name, so two writes
never share a staging path. The pid stays, because two *processes* can still write
the same config file.

**On the test.** A test that only asserted "both writes returned Ok" would have
**passed against the broken code** — the write-and-rename window is too small for
two threads to reliably interleave into the failing order. The test that actually
proves the causation asserts the invariant that *causes* the failure: two
concurrent writes use two different staging paths.

**Verified RED → GREEN.** Against the old naming the test fails with the collision
named in the assertion — the same staging path for two writes of one file:

```
thread 'config::tests::concurrent_writes_to_one_settings_file_use_distinct_staging_paths'
panicked at crates\epsilon-core\src\config.rs:2106:9:
assertion `left != right` failed: two writes of one file must not share a staging
path: "C:\Users\Hp\AppData\Local\Temp\.tmp4C5Vsz\history.toml.tmp.26984" ==
       "C:\Users\Hp\AppData\Local\Temp\.tmp4C5Vsz\history.toml.tmp.26984"
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 262 filtered out
```

Against the fixed naming, both tests pass:

```
test config::tests::concurrent_writes_to_one_settings_file_all_succeed ... ok
test config::tests::concurrent_writes_to_one_settings_file_use_distinct_staging_paths ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 262 filtered out
```

---

## 2c. The Epsilon-side audit, reconciled

A second fan-out audited Epsilon's own tree — the core, the TUI, the repository
hygiene, and the two UI-toolkit references — with the brief "what works, what is
partial, what is duplicated, what is misplaced, what is missing."

**Its first two reports confirmed what the first pass already found.** Findings 1–7
of the core audit are F17, F18, F19, F20, F21, F22, F23; the TUI audit's finding 12 is
F2. They are not new findings — they are **the same defects, found independently**,
which is worth knowing: two independent audits agreeing on the file and the line is
stronger evidence than either alone.

**The genuinely new material is in the TUI audit's specifics and in the hygiene
audit.** All of it below was re-derived before it was kept.

### F31 — The mouse wheel's surface order is not the keyboard's, and its doc comment says it is

**`crates/epsilon-tui/src/app/keys.rs:3062-3069`.** The comment claims:

> Order mirrors `handle_key` — viewer, tree, palette, picker, timeline — so the wheel
> and the arrows always agree about where the reader is.

**The code does tree first** (`keys.rs:3066`), then viewer (`keys.rs:3078`). The
comment is wrong about the order it claims to mirror.

**And the wheel covers fewer surfaces than the keyboard does.** `handle_key` branches
on `pending_ask` (`:618`), `viewer_open` (`:634`), `auth_prompt_open` (`:1104`),
`auth_pick_open` (`:1109`), `think_pick_open` (`:1199`), `scope_open` (`:1251`) and
the transcript search. `scroll_focused_overlay` handles tree, viewer, palette, picker
and timeline — and **nothing else**.

**The consequence is user-visible:** the wheel over an open search panel scrolls the
transcript *behind* the panel, because the search surface is not in the wheel's list
at all. The panel owns the keyboard and the wheel does not know it exists.

**Fix shape:** one ordered `match` over the same surface list, transcript last — the
same fix as F2, which is why this is listed under Wave 1 rather than as its own wave.

### F32 — The palette's chord column resolves 2 of the heads that have one

**`crates/epsilon-tui/src/picker.rs:1049-1057`.** `palette_key_id` is a two-arm
match — `"model" → app.model.select`, `"expand" → app.tools.expand` — and everything
else returns `None`. The palette has eight mirrored heads, and the other six print
no chord even though the binding table has one.

**Fix shape:** look the chord up from the binding table by head id rather than
hand-listing two of them, and add a test that every palette row with a bound chord
prints it.

### F33 — Two parity rows are stale in the direction of understating Epsilon

**`docs/parity/pi-tui.md:390` (P-070)** says the session browser has *"no
cross-directory scope, no rename panel and no delete"*. Epsilon **has** the scope
toggle and the delete: `app.session.delete` is bound (`keys.rs:2140`) and the
delete-refusal path is implemented with a frame-pinned test (`keys.rs:2009-2051`).

**`docs/parity/pi-tui.md:663` (P-236)** says *"there is no delete to protect"*. The
active-session refusal exists.

**The same document says so elsewhere** — P-237 at `:814` describes the delete
refusal, and the area summary at `:805` contradicts P-070. **A parity matrix whose
rows disagree with each other is worse than one that is merely out of date**, because
a reader cannot tell which row to believe.

**Fix shape:** rewrite both `ours` cells, and add a checker rule that a row may not
contradict the area summary it appears in.

### F34 — `docs/02-ARCHITECTURE.md` claims a manifest edge that does not exist

**`docs/02-ARCHITECTURE.md:33-35`:**

> The `epsilon-cli` → `epsilon-ext` manifest edge still exists while nothing
> references it; it is on the dead-edge removal list, not a claimed dependency.

**`crates/epsilon-cli/Cargo.toml` has no `epsilon-ext` dependency** — the list is
`contracts, llm, core, runtime, tools, lsp, safety, session, tui, server, mcp`. The
edge was removed; the document still describes it as present.

**Fix shape:** delete the paragraph. A document that names a specific dead edge to
keep an eye on is worth maintaining — until it describes an edge that is already gone.

### F35 — `docs/references/TUI-DECISION.md` understates the switching cost by 1.75×

**Measured on this tree, not quoted:**

| | Claimed (`TUI-DECISION.md:55`) | Actual |
|---|---|---|
| Lines | 38,067 | **66,482** |
| Files | 49 | **57** |
| Tests | 509 | **689** |

The decision document's reason 5 is *"the switching cost is the largest of the three,
by an order of magnitude"*, and it argues from a number that is 1.75× too small. The
conclusion survives — it is still the largest reason — but the argument is made from
a stale measurement, and the same document cites
`crates/epsilon-desktop/src/commands.rs:67` as evidence, a file deleted in ADR-039.

**Fix shape:** re-measure, and either refresh the numbers or replace them with a
pointer to how to measure them. A number in a decision document is a claim that
decays.

### F36 — There is no cost ceiling

**`crates/epsilon-core/src/util.rs:92-110` (`LoopLimits`)** has `max_turns`,
`max_parallel_tool_calls`, `doom_loop_threshold` and the compaction thresholds — and
nothing that stops a run by **spend**. `cache_stats.rs` prices usage
(`usage_cost_micro`, `rebilled_cost_micro`, `format_cost_micro`) and the TUI shows
it, so the cost is *reported* everywhere and *capped* nowhere. `max_turns` bounds
the number of turns; a turn that calls a large model is not bounded by its count.

**Fix shape:** `max_session_cost_micro` in `LoopLimits`, checked at the inner loop's
top, next to where `max_turns` is already checked.

### F37 — Two capabilities are implemented, tested, and unreachable

**`epsilon_session::archive_session_file` (`archive.rs:58`)** — a move into
`archived/` — has **no production caller**. The `WINDOW` chord that names it
(`keybind.rs:562-568`) is inert by design (ADR-039 removed the desktop frontend), so
the capability has no surface at all.

**`SessionStore::branch_with_summary` (`store.rs:1831`)** — the only caller is
`crates/epsilon-core/tests/compaction_scenarios.rs:679`. The store's own comment
(`store.rs:1659-1663`) records *why*: the reason it had no caller was a missing
*view* — and `tree()` now exists (`store.rs:1677`). **The view landed; the writer
did not.** The user-visible consequence: an abandoned branch is visible in `/tree`
but cannot be summarised and merged back.

**Fix shape:** one command each. `epsilon-contracts/src/command.rs` already has the
registry to hang them on. This is F13 plus one more, and it is a real product gap,
not dead code to delete — the capability works.

---

## 2b. The second reference pass — findings it added

A second fan-out read the four reference harnesses again, this time with the brief
"find the ten things Epsilon should copy." It landed after the first review was
written, so its findings are reconciled here rather than in the review body.

**Most of what it reported, Epsilon already has.** That is the finding, not a
disappointment: a reference audit is only worth reading if the *deltas* are real.
Every claim below was re-derived against Epsilon's source before it was kept.

| Reference idea | Epsilon's position | Verified at |
|---|---|---|
| pi: the truncated-tool-call guard | **has it** | `exec.rs:61` `fail_all_truncated`, wired at `run_.rs:494` |
| pi: tool errors are data, the loop never rejects | **has it** | `exec.rs:1143-1156` `aborted_result` synthesises a result for every unstarted call |
| DSH: the durable inbox, steering survives a crash | **partial** | `queue.rs` is an in-memory `VecDeque`; `restore` puts drained messages back on an abnormal exit, but nothing is written to disk |
| pi/opencode: write-side compare-and-swap | **has it** | `read_log::verify` (mtime + length + partial-read), called by the **write** path (`files.rs:715`) and the **edit** path (`files.rs:902`), both inside the mutation queue. A CAS would be a refinement, not a gap. |
| Hermes: a permanent allowlist scope | **missing** | `Action` is `Allow`/`Ask`/`Deny` (`engine.rs:10-17`); there is no third scope to remember |
| DSH: monotonic guards that structurally cannot allow | **partial** | deny is monotonic in the engine, but a guard is still a *rule* that can be outranked |
| DSH: the permission policy told to the model | **missing** | nothing in `prompt.rs` mentions a policy, a ruleset or a permission |
| Hermes: deny floors evaluated before every bypass | **missing** | no user-defined deny list exists anywhere in `crates/` |

### F26 — The subagent's result body has no size cap

**`crates/epsilon-core/src/tasks.rs:527-538` (`task_notice`) and
`crates/epsilon-tools/src/task.rs:205-215`.** The child's text is interpolated
straight into the envelope:

```rust
"<task id=\"{id}\" state=\"{state}\" ...>\n<task_result>\n{body}\n</task_result>\n</task>\n..."
```

There is no truncation and no cap on `body`. A child that reads a large file and
reports it back puts that text into the **parent's** transcript, where it counts
against the parent's window — and the parent is the one that then has to pay for a
compaction.

Hermes records the same failure with a measurement
(`tools/delegate_tool_results.py:239-264`): its budget must come from the parent's
*current* remaining headroom, not from a cumulative total; using the cumulative
figure produced 1,393 summaries in one run, each spilling to disk.

**This is the same defect class as F23** (no per-turn tool-result budget) — an
unbounded payload crossing a boundary into a context that has to pay for it.

**Fix shape:** cap the child body at the parent's remaining headroom, and say so in
the envelope when it is truncated. The parent's headroom is already known at the
drain point.

### F27 — A failed tool call is repaired only when the session is *reopened*

**`crates/epsilon-session/src/repair.rs:92` (`repair_on_open`) is called from
`session.rs:138` and `tasks.rs:303` — both are *open* paths.** Inside a live run,
nothing repairs a tool call whose result never landed.

Hermes repairs at the point of failure
(`agent/turn_loop_errors.py:120-142`), because the *next provider request* is
malformed without it: a `tool_use` with no matching `tool_result` is a provider
error, not a transcript that will fix itself later.

**The blast radius is small but real.** A tool that panics, is cancelled, or has its
process killed mid-call leaves the transcript provider-invalid for the rest of the
run, and the run continues until the provider rejects the request. The session looks
fine on disk and is repaired at the next open — but the *current* run is broken.

**Fix shape:** repair in the loop, at the point the call fails, and keep
`repair_on_open` for the crash case it was written for. The synthesised result
already exists (`exec.rs:1147` `aborted_result`) — it is the *missing* case that
needs one.

### F28 — The permission policy is not told to the model

**Nothing in `crates/epsilon-core/src/prompt.rs` mentions a policy, a ruleset or a
permission.** The model is asked to act, and is told nothing about what the harness
will permit. When a call is refused, the refusal arrives as a tool error — the
model learns the rule one violation at a time.

DSH puts the permission posture into the model's context
(`packages/interaction/user-approval/src/index.ts:73-75`), placed **after** the
retained history so that changing the policy does not invalidate the prompt-cache
prefix.

**Fix shape:** a runtime-context section, appended after the retained history, that
names the active posture. It is one block, and it is the difference between a model
that adapts and a model that retries the same refused call.

### F29 — ~~`write` and `edit` do not consult the stale-read guard~~ — **FALSE, corrected**

**This finding was raised by the second reference pass and it is wrong.** It said
`grep read_log crates/epsilon-tools/src/write.rs` and `…/edit.rs` return nothing —
which is true, and irrelevant: **`write.rs` and `edit.rs` do not exist.** Both tools
are defined in `files.rs` (`files.rs:483` for `write`, `files.rs:800` for `edit`),
and both call the guard:

- `files.rs:715` — `read_log::verify(&ctx.observation_scope, &path, true)` in the
  **write** path, with the comment *"Refuse to overwrite something that changed since
  the model read it, or that the model never fully read"*.
- `files.rs:902` — `read_log::verify(&ctx.observation_scope, &path, false)` in the
  **edit** path, with the comment *"Same guard as `write`, with `full_overwrite:
  false` — an edit matches exact text the model saw, so a partially-read file is
  fine, but a file that moved under it is not."*

Both run **inside** `crate::mutation::with_file_mutation_queue`, so no other harness
writer can slip between the check and the write.

**The lesson is worth keeping.** A grep for a symbol in a *file that does not exist*
reads as a clean negative, and it is a false negative: the guard was there, on both
mutators, with a doc comment explaining it. **A negative finding is a claim about a
path, so the path has to be resolved before the claim is made.** This is the same
discipline the first pass applied to the three corrected findings, and it is the
reason the second pass's table above lists "verified at" for every row.

### F30 — pi has no permission engine at all, and that is the wrong lesson

The Pi audit's most valuable finding is a **negative** one:
`docs/how-pi-works.md:49` says *"Enabled tools use the operating-system permissions
of the Pi process"*, and the gate is a 40-line *example* extension
(`examples/extensions/permission-gate.ts:22`). Pi has no read-only posture, no plan
mode in core, and no allow/deny memory.

Epsilon's `epsilon-safety` is strictly stronger, and the review already says so.
**What Pi contributes is the mechanism, not the posture:** the system prompt and
tool declarations live *in the transcript*
(`packages/agent/src/types.ts:24-26`), so branch, fork, compaction and replay are
all operations on one array. Epsilon's prompt assembly is a separate function from
its transcript, and pi's shape is the cleaner one.

**Fix shape:** a named-section prompt assembly whose sections are individually
replaceable, so a compaction boundary can re-pin one section without rebuilding the
whole prompt. `prompt.rs:123` already says *"Sections have fixed names and order;
extensions (M8) will register into named slots"* — the naming exists; the
*diffing* does not.

---

## 3. Findings, ranked by user value

### F1 — The inherited slice is half-landed; the tree fails its own gate

`crates/epsilon-tui/src/app/commands.rs:761` defines `App::request_quit`, and
`crates/epsilon-tui/src/app.rs:607` declares the `quit_refused` field it writes. **Neither
has a reader.** `Command::Quit` (`commands.rs:2287-2307`) still carries its own copy of the
same two guards inline.

Measured, not inferred:

```
error: field `quit_refused` is never read   --> crates/epsilon-tui/src/app.rs:607
error: method `request_quit` is never used    --> crates/epsilon-tui/src/app/commands.rs:761
error: could not compile `epsilon-tui` (lib test) due to 2 previous errors
```

The battery's clippy leg: `[exit=101 23s]`. The whole rest of the battery still runs, so
the failure is easy to miss in a summary — but the tree does not pass the gate it defines.

**Why it matters:** the refactor's own doc comment says *"Every route that returns from the
TUI uses this check"*. It does not. The two guards live in one arm, so the `Ctrl+Q` path
and the `/quit` path are the same arm today, but the extraction was made for a reason and
the reason was not finished. The `quit_refused` field's stated purpose — *"keeps the result
visible when a file viewer covers the transcript"* — is a real gap: `draw_viewer` centres a
panel over the transcript band (`app.rs:2706-2719`), so a refusal notice pushed to the
transcript is **not on screen** at that moment.

**Fix shape:** delete the unused `request_quit` and `quit_refused` (the guard already works
in the `Command::Quit` arm and `quit_refuses_unsaved_viewer_edits_in_inactive_tabs` already
proves it), or finish the extraction by routing `Command::Quit` through it and rendering
the refusal in the footer. **Do the second** — the invisible-notice problem is real, and
the footer is where a refusal belongs. Add the frame test that shows the refusal while a
viewer is open.

### F2 — Four ladders over the same surfaces, already drifting

The terminal IDE decides *"who owns the keyboard right now"* in four separate places:

| Ladder | Location | Shape |
|---|---|---|
| key order | `app/keys.rs:618-2497` | 19-deep `if` ladder |
| paint order | `app.rs:2623-2758` | 14-deep `if` ladder |
| wheel order | `app/keys.rs:3065-3127` | a third ladder — **and it disagrees** |
| footer order | `app/view.rs:443+` | a fourth ladder |

`scroll_focused_overlay`'s doc comment claims *"Order mirrors `handle_key` — viewer, tree,
palette, picker, timeline"*. It does not: the order is **tree, viewer**, palette, picker,
timeline — the reverse of the first two — and ten surfaces are absent entirely.

**Measured consequence:** the mouse wheel reaches tree, viewer, palette, picker and
timeline. It **misses** the search panel, the session browser, the settings panel, the
`/changes` panel, the scope checklist, all three auth surfaces, the ask overlay and the
approval dialog. The wheel over an open search panel scrolls the transcript behind it.

**Fix shape:** one `enum Surface` and one `fn owner(&self) -> Surface`; the four ladders
become four arms of one match over it. `fn keys_for(surface) -> Vec<(&str,&str)>` then
feeds both the footer and each panel's own hint row. This single change closes F2, the
tree-hint hole below, the palette chord column, and the hint-text drift, because they all
become one fact instead of four.

### F3 — The tree's `Left`/`Right` is implemented and advertised nowhere

`app/keys.rs:1016-1050` implements the fold/unfold gesture that every IDE tree has. The
tree's own title (`tree.rs:224-231`) and the footer's tree hint row
(`app/view.rs:565-585`) mention neither. `Enter` on a directory *also* toggles it, and
that is not advertised either.

Worse: `Right` on an already-expanded directory, and `Right` on a file, **do nothing
silently** — `keys.rs:1039-1050` requires `tree_collapsed.remove(&row.rel)` to succeed.
`AGENTS.md` §2.7: *"Nothing a UI advertises may be a no-op. A silent no-op is worse than a
missing feature."*

**Fix shape:** add the fold/unfold pair to the tree hint row, and make `Right` on an
expanded directory or a file say so (one notice) or move to the first child.

### F4 — `/model` and `/models` are two menu rows for one command

`crates/epsilon-contracts/src/command.rs` `SLASH_ITEMS` carries **both** `/models`
("Open the model picker") and `/model ` ("Switch or pick a model"). The parser folds them
into one grammar (`command.rs:536`, `:546`), and `ALIAS_HEADS` already lists `models` as an
alias.

`docs/07-COMMANDS.md` contradicts itself two sections apart: `:118-119` says *"Aliases are
parser inputs, not extra menu rows"*, and `:123` says *"`/models` … **and it has its own
offered row**"*.

**Fix shape:** delete the `/models` row from `SLASH_ITEMS` (`command.rs:52-55`). One entry.
The head stays, the alias stays, the parser stays. The palette loses one of 42 rows and
stops offering two spellings of one command.

### F5 — The permission capability is the tool's own name

`epsilon-safety/src/engine.rs:19-20` says the capability *is* a tool name. `exec.rs`'s
`gate_subject` (`exec.rs:112-238`) returns `(name.to_string(), subject)` for every ordinary
tool — with two hand-written exceptions, `apply_patch` to `edit` and an escaping path to
`external_dir`.

**Consequence:** an MCP server's `my_edit_tool`, or an extension's write tool, cannot join
the `edit` capability. A user who writes one rule for `edit` gets `write`, `edit` and
`apply_patch`; a third-party tool with the same *intent* needs its own rule, and the user
cannot know its name in advance.

opencode solves this with an explicit action namespace: `withPermission(tool, "edit")`
(`packages/core/src/tool/tool.ts:139-148`), and its own `AGENTS.md:45` states the rule —
*"Most tools default to their registered name; `edit`, `write`, and `apply_patch` declare
the shared `edit` action."*

**Fix shape:** add a `fn capability(&self) -> &str` to `ToolDescriptor`, defaulting to the
tool name, and have `gate_subject` use it. One field, one default, and every existing rule
keeps working unchanged. This is the change that makes the permission model *composable*
rather than a list of tool names.

### F6 — Dangling citations, a third of them in the files a reviewer trusts most

The mass deletion of `docs/milestones/` (261 files), `audit/evidence/` (96),
`agents/memory/` (31), `docs/plans/` (26), `audit/01`–`07` and `CREDITS.md` left the
documentation pointing at evidence that no longer exists.

`scripts/check-links.py` reports **"221 relative links, 0 broken"** — it parses *markdown
link targets* only, so a **backticked path in a Rust doc comment** is invisible to it.
Measured with a path-shaped scan over the surviving tree: **27 dangling backticked paths in
17 Rust files**, plus **21 in `CHANGELOG.md` and `docs/design/epsilon-decisions.md`**, 2 in
`docs/06-GLOSSARY.md`, and 1 in `docs/parity/pi-tui.md:172` — the *"never take a name, logo
or brand"* rule now pointing at a deleted `CREDITS.md`.

The Rust ones include `crates/epsilon-contracts/src/goal.rs:4` (`docs/subsystems/goal.md`)
and `crates/epsilon-tools/src/goal.rs:4` (`docs/tool-catalog.md`) — **two module headers
pointing at documents that have never existed in this tree.**

The ADR log — the file `AGENTS.md` §2 names as the authority for architecture — cites three
deleted milestone logs. `crates/epsilon-runtime/tests/goal_round_gates.rs:4` says *"This is
the gap `audit/03-defects.md` A26 describes"*, and `audit/03-defects.md` is gone.

**Fix shape:** restore the cited evidence into `docs/evidence/` (the history is in git —
`git show HEAD:<path>`) or rewrite the citations to what survives. Then **extend
`check-links.py` to check backticked repo paths**, so this class cannot recur silently.
Two of the 27 (`crates/epsilon-contracts/src/goal.rs:4`,
`crates/epsilon-tools/src/goal.rs:4`) name documents that never existed in this tree, so
they need a new sentence, not a restoration.

**Partly done 2026-10-22: the markdown link layer is clean.** The `agents/memory/`
notes were restored from HEAD, and they cite 17 files under `audit/` and
`docs/milestones/` that the consolidation had deleted — so restoring the notes without
restoring what they cite re-created the break. Every target the link checker reported
was checked out from HEAD, in dependency order, until the checker reported **387
relative links, 0 broken**. The *backticked-path* layer — the 27 Rust doc comments
above — is still open, and it needs the `check-links.py` extension to become visible.

**The lesson for Wave 4:** a documentation consolidation must restore what it cites,
not delete it. The content is in git; deleting it and its citations in one change
turns a clean tree into a broken one and leaves no way to tell which was intentional.

### F7 — The parity matrix contains four verifiable falsehoods

| # | Claim | Where | Measured reality |
|---|---|---|---|
| 1 | "48 offered items over 41 heads" | `pi-tui.md:252`, `:260` | `SLASH_ITEMS` has **42** rows |
| 2 | "Epsilon declares 23 ids in one table" | `pi-tui.md:244` (L-000) | `TERMINAL` has **42** ids |
| 3 | P-070: "no cross-directory scope, no rename panel and no delete" | `pi-tui.md:390` | scope toggle **and** delete both exist — and P-230 in the same document says so |
| 4 | P-236: "there is no delete to protect" | `pi-tui.md:663` | delete exists and the active session **is** protected — P-237 says so |

Two rows also cite **test names that do not exist**:
`compact_from_the_typed_line_compacts_and_reports` (`pi-tui.md:292`) and
`slash_import_switches_live` (`pi-tui.md:399`) — 0 hits in `crates/`. The second is a
*window* test, deleted with the desktop. The matrix's own rule says `done` requires a test
that exists.

**Why this is F7 and not lower:** `docs/parity/README.md` says *"`done` means exercised, not
written."* A row citing a test that does not exist is the matrix asserting its own central
claim falsely, in the two places that most need to be true.

**Fix shape:** correct the four numbers/facts and the two test names; then extend
`scripts/check-parity-citations.py` to resolve cited **test names**, not just cited files
and lines. That is the gate hole that let them in.

### F8 — The palette's chord column is right 2 times out of 8

`picker.rs:1049-1057` gives a chord to `model` and `expand` only. Five more rows have a
bound id the palette could name: `/new` to `Ctrl+N` (`app.session.new`), `/copy` to `F4`
(`app.message.copy`), `/think` to `Ctrl+T` (`app.thinking.cycle`), `/editor` to `Alt+E`
(`app.editor.external`), `/exit` to `Ctrl+Q` (`app.exit`).

A column the reader cannot trust is worse than no column. `kb.key_text` already resolves
them, so this is a ~6-line change plus the missing test.

### F9 — Two doors to one action report differently

`Command::Expand` (`commands.rs:2259-2273`) calls `toggle_tool_expansion()` **and** pushes
a notice. The `Ctrl+X` key path deliberately does not, because a golden test asserts the
frame is byte-identical. So one gesture has two outcomes, and the divergence is pinned by
a test rather than by a design.

Same shape in `/copy`: the `SLASH_ITEMS` description says "last **reply**", the id
(`app.message.copy`) says "last **assistant message**", and `docs/07-COMMANDS.md:66` says
"most recent assistant **transcript line**". Three spellings of one fact.

**Fix shape:** one arm owns the notice, and the golden test asserts the *invariant* (the
same action reports the same way) rather than the current divergence.

### F10 — Six scripts have no caller

`report-citation-subjects.py`, `report-reference-fidelity.py`, `remap-citations.py`,
`remap-by-vintage.py`, `clear-cache.sh` and `fix-links.py` are called by nothing — not by
the battery, not by CI, not by a document except `scripts/README.md`. `fix-links.py`'s every
target has already been deleted. `rpc_probe.py` has no gate, no CI and no source
reference.

**Fix shape:** delete the six. `scripts/README.md` is a table of tools; a row for a tool
that nothing calls is a claim the reader will try to act on.

### F11 — 13 of 15 in-repo skills are named by nothing

Only `ponytail` and `improve` are referenced by path from any product file. The other
13 are a declared agent-tooling library that no product file names, and `AGENTS.md` does
not say it is one.

**Fix shape:** decide. Either `AGENTS.md` names `.agents/skills/` as the repository's
agent-tooling library — one sentence, and the tree becomes coherent — or prune it. The
middle state is the problem.

### F12 — The `WINDOW` keybinding table is a 55-row tombstone

`epsilon-contracts/src/keybind.rs:452` holds 55 `window.*` ids. **Zero** of them has a
production reader: `grep -rn '"window\.' --include=*.rs crates/` returns **0 hits** outside
the defining file. The desktop they were written for was removed in ADR-039.

`AGENTS.md` §2.3 says the table *"is retained deliberately for the TUI IDE, not as a second
frontend"* — but the TUI IDE binds `app.*` ids, not `window.*` ids, and the two tables
share **zero** ids.

**Fix shape:** this is a decision, not a cleanup. Either delete the table and amend
`AGENTS.md` §2.3, or keep it and say *why* — "the chord grammar is shared and the window
table is the reference for the eventual window front end" — in the same sentence, so the
next reader does not have to rediscover that it is unwired. 55 rows of unwired data is
the largest single piece of dead weight the review found.

### F13 — `Agent::search_history` has no production caller

`crates/epsilon-core/src/session.rs:367` implements the pi-parity session search
(`epsilon_session::search_session`, `SearchOptions`, newest-first, capped). The TUI's
`/search` searches **the rendered transcript** instead (`commands.rs:2281-2285` says so
explicitly, and it is the right call — pi searches the screen too).

So the durable search is correct, tested, and unreachable from every surface.

**Fix shape:** do not wire it to `/search` — the rendered-transcript search is the better
behaviour and pi's. Either expose it as a different verb (`/search --all`) or delete it.
A correct primitive with no production caller is not a feature; the repo's own rule says
so.

### F14 — Seventeen Rust files cite deleted documents

Distinct from F6 because these are *claims about evidence*, not links:
`crates/epsilon-core/src/skills.rs:401` cites `audit/03-port-plan.md` (a file that never
existed under that name); `crates/epsilon-core/tests/usage_anchor.rs:4` cites
`audit/04-roadmap.md`; `crates/epsilon-llm/src/oauth_login.rs:194` cites
`docs/qa/08-oauth-login-client-ids.md`. Each is a sentence explaining *why* the code is
the way it is, pointing at a document that cannot be opened.

**Fix shape:** fold the surviving explanation into the comment and drop the citation, or
restore the document. The information is in git; it is one `git show` away.

### F15 — The documentation consolidation left the ADR log stale and the status un-re-stamped

`docs/02-ARCHITECTURE.md:33-35` says the `epsilon-cli` to `epsilon-ext` manifest edge
*"still exists"*. It does not — `crates/epsilon-cli/Cargo.toml` has no `epsilon-ext`.
`:130` points at "the defect ledger", now deleted. The file is a 2026-10-14 snapshot
published as an entry point with no staleness banner, and it disagrees with
`docs/04-STATUS.md`.

`docs/04-STATUS.md` claims a green battery (`gate-commands2-20261018`, 2,606 passed).
That log is real and the count is correct — **but it describes a tree that no longer
exists.** The current tree does not compile clean. The record is honest about the past
and silent about the present.

**Fix shape:** re-run the battery on the settled tree, re-stamp `04-STATUS.md` with the
label and the measured count, and put the live limitation (F1) in the *known limitations*
list, not only in the commit.

### F16 — The repository is one big unwaved change

The inherited tree is 112 modified files: 68 `.rs` code files (+4,988/-1,398), 11 test
files (+995/-220), 25 docs, 7 config, 1 binary. The TUI half alone is 24 files and
+2,376/-907.

**It is not a "settle" commit.** It changes behaviour in the command registry
(`command.rs` -111/+60), the keybinding table, `tool.rs`, `main.rs` (+232), four gate
scripts, the CI workflow and two manifests. Committing it as one blob would make it
unreviewable, and it would bury the feature boundaries that the ADR log already names.

**Fix shape:** split by the boundaries the code already draws, and run the battery per
wave.

---

## 4. The plan

### Wave 0 — Settle the tree (prerequisite for everything else)

**Why first:** nothing else can be verified on a tree that does not compile clean, and the
current tree carries 600 changed files that must be split before any of it can be
reviewed.

| Step | What | Verify |
|---|---|---|
| 0.1 | ~~Finish or revert the `request_quit` extraction (F1)~~ **DONE 2026-10-22** | `cargo clippy --workspace --all-targets -- -D warnings` clean ✔ |
| 0.2 | Split the inherited tree into waves by feature boundary | each wave compiles; `git status` shows only that wave's files |
| 0.3 | Commit the doc-only wave (the 25 doc files, the 3 staged renames) | the 9 repository checkers green |
| 0.4 | Commit the `apply_patch` feature with its description file and `auth_cli.rs` test | ADR-059's claim becomes true |
| 0.5 | Commit the remaining code waves | battery green at each wave |

**0.1 is done, and it was not a cosmetic fix.** `Command::Quit` and the keybinding
called two different exit routines; the one `Command::Quit` called was dead code that
`clippy -D warnings` rejected, so the *wired* path was the one without the refusal
guard. Both now go through `App::request_quit`, which refuses on an in-flight run or a
dirty viewer, pushes a notice saying why, and clears the refusal on a successful exit.
The refusal is also **visible**: the file viewer is a panel over the transcript band,
so a notice-only refusal is a refusal the reader cannot see. The footer's mode word
now reads `quit refused` — *additively*, so the viewer's own `alt+s save` hint still
prints, because that is the key the reader needs next.

**F25 was found by the battery, not by the review.** The first post-fix gate run to
reach the test leg (`gate.sh fix2-20261022`) failed
`alt_enter_while_streaming_queues_a_follow_up` with a history-write error. The cause
was a genuine concurrency defect in the settings writer, described at F25, and it is
**user-visible**: two submits racing in one session lost one of them.

**The staged-rename hazard.** The index holds three renames into `docs/archive/` with
**1,226 lines of real content** in the blobs — but `docs/archive/` does not exist in the
worktree, and the three source paths are gone from disk. The content is safe **in the
index**; it is lost the moment anything runs `git add -A` (which would stage the three
deletions) or `git commit -a`. **Do not run either before the archive is restored to the
worktree.**

**Discipline:** `git add <explicit paths>` only. Never `-A`, never `-a`.

### Wave 1 — The surface table (F2, F3, F8, F9, F31, F32, and the hint drift)

**Why this wave is first among the code waves:** it is the single change that makes the
TUI's four ladders into one fact, and it is the foundation for the Phase 3 polish — a
layout that knows which surface owns the keyboard is a layout that can draw that
surface's hints.

1. `enum Surface` + `fn owner(&self) -> Surface` in `epsilon-tui`.
2. Route the key ladder, the paint ladder, the wheel ladder and the footer through it.
3. `fn keys_for(surface) -> Vec<(&str,&str)>` feeds both the footer and each panel.
4. **The wheel reaches every surface the keyboard reaches, in the same order** (F31).
   Today the wheel handles five surfaces and the keyboard handles thirteen, and the
   wheel's own comment claims an order it does not implement. The user-visible
   consequence: the wheel over an open search panel scrolls the transcript behind it.
5. The tree hint row names the fold/unfold pair; `Right` on a non-collapsible row says so.
6. **The palette's chord column resolves every row that has a bound id** (F32) —
   looked up from the binding table by head id, not hand-listed.

**Acceptance:** a frame test per surface, at 80x24 and 100x30, showing the correct hint
row; a test that the wheel order equals the key order; a test that every palette row
with a bound chord prints it.

### Wave 2 — Command system integrity (F4, F7, F9, F33, F34, F35)

1. Delete the `/models` `SLASH_ITEMS` row; the alias stays.
2. One notice for one action: `/expand` and `Ctrl+X` agree; `/copy`'s three spellings
   become one.
3. Correct the four parity-matrix facts and the two dead test names.
4. **Rewrite the two parity rows that contradict their own area summary** (F33) —
   P-070 and P-236 both say Epsilon lacks a session delete that it has.
5. **Delete the `epsilon-cli` → `epsilon-ext` paragraph from `02-ARCHITECTURE.md`**
   (F34): the edge is gone and the document still names it as present.
6. **Re-measure `TUI-DECISION.md`** (F35) and either refresh the numbers or replace
   them with how to measure them; drop the citation to the deleted
   `epsilon-desktop/src/commands.rs`.
7. Extend `check-parity-citations.py` to resolve cited **test names**, and to reject a
   row that contradicts the area summary it appears in.
8. Extend `check-links.py` to check **backticked repo paths**, not only markdown links.

**Acceptance:** the two checkers fail on a seeded violation (a doc citing a test that does
not exist; a Rust comment citing a deleted path) and pass on the real tree.

### Wave 3 — The permission engine and the capability namespace (F5, F17, F18, F19, F20, F21, F24, F28)

**This wave moved ahead of the TUI waves because F17 is a safety-adjacent defect: a
harness that asks about `find` and `todo` teaches its user to approve reflexively, and
that is how a real permission gate stops working.**

1. ~~**Make `is_read_only_capability` public** (F24) and **derive the default
   free-set from it** (F17)~~ **DONE 2026-10-22** — `READ_ONLY_CAPABILITIES` is the
   single list, `default_ruleset()` derives from it, and a test reads the live
   registry. ✔
2. ~~**Fold plan + base + preset into one `PermissionEngine`** (F18) and evaluate
   once.~~ **DONE 2026-10-22** — the gate composes the three layers and evaluates
   once; `glob_match` no longer appears in `gate.rs`; the invariant is tested. ✔
3. **Delete `plan_ruleset()` and its layer** (F22); make `plan_on` a boolean that
   selects the allowlist path. One answer to "what does plan mode forbid?"
4. ~~`ToolDescriptor::capability(&self) -> &str`, defaulting to the tool name (F5)~~
   **DONE 2026-10-22** — `Tool::capability()` on the trait, `apply_patch` declares
   `edit`, and the gate matches the declared capability in both paths. ✔
5. **One `Retrying` constructor with a notice sink** (F19); the runtime's hook routes
   to the agent's event channel, so a retry is on the frame.
6. **Keep the provider's structured overflow code** (F20); check it first, keep the
   substring match as the fallback.
7. **Validate `required` and primitive types in `basic_validate`** (F21). The schema is
   already a `serde_json::Value`; no dependency.
8. **Tell the model the active posture** (F28): one runtime-context section, appended
   after the retained history so a policy change does not invalidate the cache
   prefix. The model should not learn the rule by being refused.
9. **Add a third scope — "always"** (the Hermes ladder: once / session / always),
   with a permanent allowlist that a later save cannot resurrect a revoked entry from.
   `Action` (`engine.rs:10-17`) has no third arm today.

**Acceptance:** a test that a rule on `edit` covers `write`, `edit`, `apply_patch` and an
extension tool that declares `edit` — and that every rule written before the change still
matches; a test that every registered tool is free-by-default or explicitly ask-first;
a frame test that a retry notice reaches the transcript; a test that the model is told
the posture and that a refusal names it.

### Wave 3a — Context and durability bounds (F23, F26, F27, F30)

**One wave for one shape of defect: an unbounded payload, or a broken transcript,
crossing a boundary that something downstream has to pay for.** F23 and F26 are the
same defect at two boundaries — the tool-result sum and the subagent's return — and
F27 is the same shape as F26's consequence: a payload that never arrives.

1. **A per-turn tool-result budget** (F23): track the turn's accumulated result bytes
   beside `usage_anchor`; when it exceeds `limits.compaction_reserve_tokens`, run the
   existing prune pass **before** the results are pushed, and report it in the
   `Notice`. Today the prune pass runs only *inside* a compaction, so the harness
   pays for a summarization call to fix something it could have noticed for free.
2. **Cap the subagent's result body at the parent's remaining headroom** (F26), and
   say so in the envelope when it is truncated. The budget must be the parent's
   *current* headroom, not a cumulative total — Hermes records the measurement that
   makes the difference (`tools/delegate_tool_results.py:239-264`).
3. **Repair a failed tool call in the loop** (F27), at the point the call fails, not
   only when the session is reopened. A `tool_use` with no `tool_result` makes the
   *next provider request* malformed, so the current run is broken even though the
   file on disk will be fine after the next open.
4. **Named, individually-replaceable prompt sections** (F30): so a compaction
   boundary can re-pin one section instead of rebuilding the whole prompt, and a
   prompt-cache prefix survives. `prompt.rs:123` already names the slots; the
   diffing does not exist.
5. **A cost ceiling** (F36): `max_session_cost_micro` in `LoopLimits`, checked at
   the inner loop's top next to where `max_turns` already is. The cost is priced
   and shown everywhere; nothing stops a run by it.

**Acceptance:** a test that a turn whose tool results exceed the budget is pruned
before the request rather than after the overflow; a test that a subagent returning a
huge body lands truncated and says so; a test that a tool which fails mid-call still
leaves a transcript the next provider request accepts; a test that a run whose spend
passes the ceiling stops and says so.

### Wave 4 — Documentation and evidence integrity (F6, F10, F11, F13, F14, F15, F37)

1. Restore the cited evidence into `docs/evidence/`, or rewrite the citations.
2. Repair the 27 Rust doc-comments that cite deleted or never-existing documents.
3. Delete the six uncalled scripts; decide `rpc_probe.py`.
4. Decide the `.agents/skills/` tree: name it in `AGENTS.md` or prune it.
5. Decide `WINDOW`: delete the 55-row table and amend `AGENTS.md` §2.3, or keep it and say
   why in the same sentence.
6. **Give the two unreachable capabilities a surface** (F37): a `/summarize-branch`
   command for `branch_with_summary`, and `app.session.archive` in the `TERMINAL`
   table for `archive_session_file`. Both work today and neither is reachable —
   the branch view landed (`store.rs:1677`) and the writer did not follow.
7. Decide `Agent::search_history`: expose it under a distinct verb, or delete it.
8. Re-stamp `04-STATUS.md` from a real battery log on the settled tree.

**Acceptance:** `check-links.py` reports 0 broken *and* the new backticked-path check
reports 0 broken; no surviving file names a deleted path; `/summarize-branch` produces
a summarised branch a frame test can assert on.

### Wave 5 — The harness's next capability tier (from the reference studies)

Ordered by user value, each with its reference cited:

| # | Capability | Reference | Epsilon today |
|---|---|---|---|
| 5.1 | **Compare-and-swap file writes.** The write fails with a stale-content error when the bytes changed since the read. | opencode `packages/core/src/file-mutation.ts:144-157` | checkpoints record preimages; the *write* does not check |
| 5.2 | **A serialised event writer.** One permit, so a stream's events and a parallel tool's events cannot interleave. | opencode `packages/core/src/session/runner/llm.ts:237-239` | broadcast channel; commit order is model order, but append is not one-writer |
| 5.3 | **A per-session run coordinator.** One drain per session id, coalesced wakeups, join-on-resume, cross-session concurrency. | opencode `packages/core/src/session/run-coordinator.ts:24-104` | `Agent::running` is one flag for the whole agent |
| 5.4 | **A latched-verdict mechanism.** A phase cannot clear a safety flag an earlier phase armed. | Hermes `agent/conversation_loop.py:1392` | not present |
| 5.5 | **An in-loop tool-call repair.** After an escaped exception, every unanswered `tool_call` gets a synthetic error result. | Hermes `agent/turn_loop_errors.py:120-142` | repair happens on session *open*, not at the failure point |
| 5.6 | **A skill/plugin tamper scan.** Skills that write to `AGENTS.md`, `.cursorrules`, or the agent's own config are detected. | Hermes `tools/skills_guard.py:64-71` | `skills.rs` is a reader with no scanner |
| 5.7 | **A pluggable context-engine contract.** An interface with `should_compress`/`compress`/session-boundary hooks, so compaction is one implementation of a contract rather than the only one. | Hermes `agent/context_engine.py:53-80` | `compact.rs` is stronger in detail but not pluggable |
| 5.8 | **A typed provider-error kind.** Structured signals (HTTP status, provider error code) with regex as fallback. | pi's weakness (`packages/ai/src/utils/retry.ts`) | Epsilon has the same regex-over-string shape |
| 5.9 | **Global-user rules override repository rules**, explicitly. | opencode `packages/core/src/config.ts:204-211` | layers evaluate later-wins; a project rule can currently widen its own permissions |

**5.9 is a security item, not a feature.** It is listed with the features because it is
worth the same weight, and it should be first among them.

### Wave 6 — Phase 3: the TUI as a product

The layout model is already coherent (`ui/layout.rs`: one `Regions` split, one
`Explorer::split`, `explorer_width` derived from the frame, a documented narrow-terminal
band). The 42-row command model is already clean. What is missing is the polish, and the
polish is blocked on Wave 1, not independent of it.

1. **Visual hierarchy.** The header, the transcript, the composer and the footer are
   separated by a rule and a blank row (`CHROME_ROWS = 4`); the explorer is a rail plus a
   title. Give the transcript its own type scale and the chrome its own.
2. **Density and alignment.** The command rows already use a measured command column, a
   description column and a right-aligned hint (`ui::overlay::list_row_columns`) — extend
   that to every overlay.
3. **The composer's width.** The slash menu and the palette are already the composer's
   width; the tree is not, and a 60-column frame shows the commands at 60 while the tree
   is gone.
4. **Every async view has its states.** Loading, empty, error, cancelled, long-running.
   The permission dialog and the tree cap already name themselves; the session browser
   and the model picker do not.
5. **Contrast.** The measured floor is asserted for `high-contrast` only; the other two
   palettes are not measured.
6. **The accessibility pass that is still owed.** `docs/04-STATUS.md` names it as a known
   limitation and it is human work.

---

## 5. What the references taught, and what Epsilon should not copy

### Adopt

| Idea | From | Why it fits here |
|---|---|---|
| The prompt lives in the transcript, with named sections and a section diff | pi (`core/system-prompt.ts`) | Epsilon already has the three tiers; the *diff* is what is missing |
| `terminate` is per batch, not per call — one tool cannot hijack the stop decision | pi (`agent-loop.ts`) | Epsilon already does this (`exec.rs:39-45`) |
| Tool schemas validated in **both** directions; the schema is generated once at registration | opencode (`tool/tool.ts:79-110`) | Epsilon validates input; it does not validate output |
| The permission **action** is a namespace separate from the tool name | opencode (`tool/tool.ts:139-148`) | **Done 2026-10-22** — `Tool::capability()`; `apply_patch` declares `edit`; the gate matches the declared capability |
| A compare-and-swap write | opencode (`file-mutation.ts:144-157`) | 5.1 |
| A per-key run coordinator with coalesced wakeups | opencode (`run-coordinator.ts:24-104`) | 5.3 |
| A deny-only monotonic guard | DSH (`packages/guard/…`) | Epsilon already has it |
| The durable two-target inbox | DSH (`packages/core/agent-loop/src/inbox.ts`) | Epsilon's session mirror is the same idea, better executed |
| A queryable command catalogue | OpenTUI (`packages/keymap/src/services/command-catalog.ts`) | Wave 1's `Surface` + `keys_for` is the Rust form |
| The paint reports what it did not show, and the caller refuses the decision | codewhale (`components/approval.rs:837-889`) | Epsilon's ask overlay does not do this |
| Fixed layout order that *yields* information as the window narrows | codewhale | already the shape of `Regions` |

### Reject — deliberately, with the reason

| Idea | From | Why not |
|---|---|---|
| **No permission system at all** | pi | Epsilon's gate is the product's core value. Pi reads `C:\Users\Hp\.ssh\id_rsa` without asking (`core/tools/read.ts:15`); the README says pi "has no permission system" and the code agrees |
| **Extensions in-process with no capability model** | pi | An extension that can widen its own scope is not a permission system |
| **Two keybinding grammars** — semantic ids for built-ins, raw key combos for extensions | pi | Epsilon's one grammar is the thing to keep |
| **Non-atomic settings writes** | pi (`settings-manager.ts:245`) | Epsilon's `write_config_atomic` is strictly better |
| **Regex over provider error strings** for retry and overflow classification | pi, opencode | A maintenance liability; a typed error kind is the fix |
| **A whole plugin framework** (cordis) | DSH | A Rust trait plus `Arc<dyn>` is a different, smaller thing. Take the *semantics* — reversible effects, scoped registration — not the framework |
| **A second install story** (eight native platform packages via a JS package manager) | OpenTUI | Epsilon's promise is one binary with no runtime |
| **The Typert remote / API-gateway stack** | DSH | Epsilon's SSE mode is the equivalent and is smaller |
| **A repository rule overriding a user's own rule** | — | opencode gets this right and pi does not have it at all; Epsilon must not lose it. See 5.9 |

---

## 6. Testing and acceptance

**The battery is the verdict.** 14 legs, one build owner, `CARGO_BUILD_JOBS=1`, an external
`CARGO_TARGET_DIR`, a fresh label per attempt, foreground, and never an edit while it
runs. The log's own tail is the answer — `===== GATE PASS =====` — not a printed status.

Per wave:

| Wave | New test | What it proves |
|---|---|---|
| 0.1 | a frame test showing the quit refusal **while a file viewer is open** | the refusal is on screen, not in a covered transcript |
| 1 | a test that the wheel's surface order equals the key's | the ladders cannot drift again |
| 1 | a frame test per surface showing that surface's hint row | the footer and the panel agree |
| 2 | a checker test with a seeded dead test name | the new checker fails for the reason it names |
| 3 | a rule on `edit` covering `write`/`edit`/`apply_patch`/an extension tool | the capability namespace composes |
| 4 | a checker test with a seeded backticked deleted path | the new link check fails for the reason it names |
| 5.1 | a write against changed bytes fails with a stale-content error | the CAS is real, not advisory |
| 5.4 | a phase cannot clear an earlier phase's flag | the latch holds |

**Regression risks, named:** the key-routing change can move a key from one surface to
another, so every existing key test must still pass *and* the surface order must be
proved. The capability-namespace change can re-route a permission rule, so the whole
permission suite must be green and the old-rule compatibility test must pass. The
checker changes can fail the tree, so each is run against the real tree before it is
committed.

**The evidence discipline is the project's own** (`AGENTS.md` §5): a test that passes is
not a claim that is true; a control that has not been shown to fail proves nothing; a
frame that has not been looked at is not a frame.

---

## 7. Repository shape after this plan

```
docs/
  01-OVERVIEW.md … 07-COMMANDS.md     the numbered entry points (unchanged)
  08-ARCHITECTURE-REVIEW.md           this document
  design/epsilon-decisions.md         the ADR log (grows by 6-9 entries)
  user/                               the task guides
  parity/pi-tui.md                    the live matrix
  references/                         one page per reference
  evidence/                           RESTORED: the cited audit and milestone logs
audit/evidence/                       the gate logs and frame dumps
crates/…                              unchanged in shape
```

**The one rule:** a document is current or it is gone. This plan deletes nothing that
carries information the repository still needs — it *moves* the deleted evidence into
`docs/evidence/` and *repairs* the citations that point at it. The `audit/` directory
keeps what is a live gate artifact; the narrative that was deleted from it lives in
`docs/evidence/` and stays reachable.

---

## 8. What was not done, and what remains open

- **The `WINDOW` table (F12) is a decision, not a fix.** This plan does not make it.
- **The screen-reader pass is human work** and is not in any wave.
- **A permanent "always allow" allowlist** — Epsilon has `AllowOnce` and `AlwaysSession`
  (`epsilon-safety/src/gate.rs:25`, `:46`) but nothing that survives the session. Hermes
  has a project-scoped permanent list; opencode's V2 persists grants to the database.
  Whether a coding agent should have one is an owner decision, not an engineering one.
- **The Windows read-only backend** — a restricted token or an ACL. Job Objects bound
  memory and the process tree, not the filesystem. Named in `docs/04-STATUS.md` as a
  known limitation and unchanged by this plan.
- **The OS-keyring credential store** — optional hardening; plaintext `auth.json`
  is the owner-accepted, Pi-1.3.0-parity model (ADR-079 amendment).
- **Every claim in this document is anchored to a file:line that was opened during the
  review.** The full studies, with their own anchors and their own corrections log, are in
  the review's working set; this document is the plan, not the evidence dump.
