# Hermes Agent → Epsilon: portable parts

Source mined: `references/hermes-agent-2026.8.19/` (Python, ~126k LOC in `agent/`, ~124k in `tools/`).
Target: Epsilon Rust workspace (`crates/epsilon-*`). Read-only audit; every anchor below was opened.
Deltas only — things Epsilon does not have, or does coarser. Already-ported Hermes ideas
(byte-stable tiers, replay fidelity, cross-process leases, TTL probes, error-classified fallbacks,
MEMORY.md tier) are excluded.

---

## Verdict

Hermes is a far broader *assistant* (gateway, desktop, cron, kanban, browser/vision/TTS) and most of
that surface is irrelevant to a focused coding harness. On harness fundamentals Epsilon is at parity
or ahead — bounded pools, fail-all-on-truncation, leases, catalog, OAuth are all things Hermes does
not have. But Hermes is meaningfully ahead in three coding-specific places. **First, consequence
checking:** Epsilon's own `agent-contract.md` *states* "after code changes, run the project's
declared checks" — Hermes is the only one of the four references that *enforces* it, by recording
verification evidence per session and injecting a synthetic nudge when the model tries to stop on
unverified edits. **Second, edit-safety bookkeeping:** Hermes tracks who read what, when, and at
which mtime, and blocks or warns on stale/write-without-read/partial-view writes — Epsilon has
nothing here, which bites hardest precisely because it runs parallel subagents that write files.
**Third, contact-time context:** Hermes loads `AGENTS.md` from directories the agent actually
touches, and plans tool batches by canonical-path conflict rather than by a coarse tool category.
Everything below is either that class of finding or a genuinely cheaper mechanism Epsilon lacks.

---

## Port candidates

| # | What Hermes does | Hermes anchor (path:line) | Why it matters | Effort | Risk |
|---|---|---|---|---|---|
| 1 | Records verification evidence per session; injects a synthetic "you edited code but have no fresh passing verification" nudge when the model stops, max 2 attempts | `agent/verification_stop.py:233`, `agent/verification_evidence.py:731` | The single highest-value gap. Epsilon *states* the verify rule in `agent-contract.md` but enforces nothing; a model can declare done on an unbuilt edit | M | Low — additive; needs a nudge queue that survives the finalization path |
| 2 | Stale-read guard: per-path read stamps (mtime + time + partial flag), last-writer map, and a write gate returning 5 distinct staleness diagnostics | `tools/file_state.py:142` | Prevents clobbering. Epsilon has parallel subagents writing files and zero read-before-write bookkeeping | M | Low — in-process registry (whole process), no cross-process claim |
| 3 | Batch planner splits a tool batch into ordered parallel/sequential segments by canonical-path conflict, with reader/writer roles | `agent/tool_dispatch_helpers.py:117` | Epsilon's rule is coarse ("any Sequential = barrier"). This makes two edits to different files concurrent, two edits to the same file ordered | M | Medium — must preserve model call order exactly (reservations reset on segment close) |
| 4 | Loop guardrail controller: exact-failure counts, idempotent no-progress detection by result hash, per-turn tool caps, warn/block/halt, guidance appended to the result | `agent/tool_guardrails.py:334`, `:186` | Epsilon's doom-loop brake is one streak counter → Ask. This separates *failed repeat* from *identical-result repeat* and only opts into hard stops | M | Low-Medium — thresholds must be config-gated (Hermes defaults hard-stop **off**) |
| 5 | Subdirectory context hints: `AGENTS.md`/`CLAUDE.md`/`.cursorrules` loaded from directories touched by tool calls, digest-deduped, ≤5-ancestor walk | `agent/subdirectory_hints.py:72`, `:30`, `:48` | Epsilon loads only root `AGENTS.md`. In a monorepo the per-package rules never reach the model | S | Low — must clamp to workspace and keep the excluded-dir list |
| 6 | Project verification recipe detection: bootstrap/build/test/start/port/readiness across node/python/go/rust/java/make/compose, cached as a manifest | `agent/verify/recipes.py:35`, `:140`, `:459`; `agent/verify/environment.py:49` | Feeds #1. Epsilon has no notion of "the project's declared checks" beyond prose | M | Low — pure detection, best-effort |
| 7 | `@`-reference expansion into context blocks: `@file:path:10-20`, `@folder`, `@diff`, `@staged`, `@git`, `@url`, confinement-checked, sensitive-path blocked | `agent/context_references.py:212`, `:24`, `:92`, `:484` | Epsilon has TUI `@`-completion only; refs are not expanded into context with line windows and diff modes | M | Medium — path containment must be enforced (`allowed_root`, default cwd) |
| 8 | Untrusted tool-result wrapping (`<untrusted_tool_result>`), delimiter-forgery regex, plus upstream-elision notice for provider-truncated payloads | `agent/tool_dispatch_helpers.py:589`, `:604`, `:643`, `:650` | Epsilon's `webfetch` returns provider text raw (verified absent: no `untrusted` in `epsilon-tools/src/webfetch.rs`). Promptware defense + false-completeness guard | S | Low — pure result-shaping at message construction |
| 9 | LSP diagnostics as post-edit feedback: baseline snapshot before write, delta-filtered post-edit diagnostics, line-shift remap so pre-existing errors don't read as "introduced" | `agent/lsp/manager.py:297`, `:321`; `agent/lsp/range_shift.py:33` | Epsilon has **no LSP at all**. This is the difference between an edit tool that guesses and one that knows | L | Medium-High — new subsystem (server registry, idle reaping, per-(server,root) broken marking) |
| 10 | Middleware contract: `llm_request`/`llm_execution`/`tool_request`/`tool_execution` — rewrite args before guardrails/approval, or wrap execution, in a nested chain, fail-open | `docs/middleware/README.md:43`, `:79`, `:105` | Epsilon extensions only subscribe to events (`epsilon-ext/src/manifest.rs:41` `wants_event`). No request/execution interception seam exists | M | Medium — ordering vs. permission gate is a security-relevant decision |
| 11 | Per-model guidance injection: tool-use enforcement, execution discipline, parallel-call, task-completion blocks, gated by model-name substring lists | `agent/prompt_builder.py:374`, `:411`, `:454`, `:484`; `agent/system_prompt.py:474` | Epsilon's STABLE tier is one static tool list + 3 guideline lines. These blocks are cheap and target real observed failure modes | S | Low — must be resolved once per session to stay byte-stable |
| 12 | Skill lint (`skill_linter`), AST audit of skill scripts (`skills_ast_audit`), install-time threat scanning (`skills_guard`), content-addressed mutation ledger with rollback | `tools/skill_linter.py:374`; `tools/skills_ast_audit.py:25`; `tools/skills_guard.py:101`; `tools/skill_ledger.py:196`, `:300` | Epsilon loads `SKILL.md` folders but has no validation, no scan, and no way to undo a bad skill edit | M | Low-Medium — ledger blobs are a disk-growth surface (needs a cap) |
| 13 | Workspace checkpoints: bare-git shadow store per project hash, per-project refs, per-file ledgers, pruning with observability | `tools/checkpoint_manager.py:72`, `:475`, `:1726` | Epsilon's fork/clone are *session*-level. There is no workspace rewind — a bad multi-file edit is unrecoverable except by hand | L | Medium — disk growth, exclude-list hygiene, Windows path handling |
| 14 | Background review fork: after a turn, a separate short agent (≤16 iterations, 24-message digest) reviews the transcript and writes memory/skill updates | `agent/background_review.py:201`, `:353`, `:401`, `:537` | Epsilon has skills but no self-improvement loop. This is how Hermes skills actually get written | M | Medium — must never mutate the live turn; needs its own cancel path |
| 15 | Compaction invariant: user turns are never summarized; only assistant exchanges are absorbed; head+tail protected; cursor recovered by scanning for the last summary marker | `docs/micro-compaction.md:63`, `:100`, `:115` | Epsilon's `compact.rs:53` serializes *user* turns into the summary body. Paraphrasing instructions is how an agent later does the thing it was told not to | S | Low — a filter in the summarization path plus a marker-scan on resume |
| 16 | Background process registry: rolling 200 KB output buffer, 64-process LRU cap, watch patterns with per-pattern strikes and global rate limits, 30-min finished TTL | `tools/process_registry.py:62`, `:73`, `:74`, `:78` | Epsilon has background *subagents*, not background *shell processes*. No `nohup`-style job with tail/watch exists | M | Medium — process reaping and Windows job-object interaction |
| 17 | Empty-response guard (retry budget keyed to cost threshold, deterministic-empty detection) and repetition guard (line-dominance ratio) | `agent/empty_response_guard.py:234`, `:254`; `agent/repetition_guard.py:43` | Cheap terminal-quality gates. Epsilon's never-fail contract gives one error shape but nothing catches a *successful* degenerate stream | S | Low — pure post-stream predicates |
| 18 | Message/stream sanitization: tool-call argument repair, interrupted-sequence closing, deterministic + uniquified call ids, reasoning-echo policy per provider family | `agent/message_sanitization.py:195`, `:296`, `:551`, `:714` | Epsilon has 8 provider wires; each of these repair paths is a class of provider-side 400 that Epsilon currently surfaces as a failure | M | Low-Medium — must not mask real provider errors |

---

## Detail

### 1. Verify-on-stop + verification evidence store

Two halves. `verification_evidence.py` keeps a SQLite evidence table keyed on
`(session_id, workspace_root)`. The interesting part is `classify_verification_command`
(`:516`): it splits a shell command into segments (`:157`), canonicalizes tokens (`:262`),
strips command prefixes, and matches against equivalent needles (`:291`) to decide whether a
`terminal` call counts as a real test/lint/build run versus an ad-hoc script — ad-hoc scripts
are recognized by a `hermes-verify-` filename prefix (`:29`) and recorded as *weaker*
evidence. `record_terminal_result` (`:563`) writes on exit; `verification_status` (`:731`)
reads back with a 30-day retention cutoff (`:26`) and a 100-events-per-root cap (`:27`).
`verification_stop.py:233` then builds the nudge: filters non-code extensions
(`:24`, `:43`) so a docs-only turn never fires, checks the snapshot, and emits
"Changed paths / Verification status / run `<cmd>`" as a synthetic follow-up, capped at
2 attempts (`:238`). With recipes present it names the commands; without them it tells the
model to write a temp script under the OS temp dir and label it ad-hoc. **Lands in:**
`epsilon-core` (new `verify.rs` gate + nudge source feeding the follow-up queue),
`epsilon-safety`/`epsilon-tools` for the evidence writer, a new evidence store beside
`epsilon-session`.

### 2. Stale-read / write-without-read guard

`FileStateRegistry` (`tools/file_state.py:59`) holds three maps under two locks: per-agent read
stamps (`path → (mtime, read_time, partial)`), a global last-writer map (`path → (task_id, ts)`),
and per-path locks for read-modify-write sections. `record_read` (`:93`) stamps mtime at read
time; `note_write` (`:114`) updates both the writer map and the agent's own stamp (a write is
an implicit read). `check_stale` (`:142`) returns one of five model-facing strings in severity
order: sibling subagent wrote after my read → "re-read the file"; sibling wrote but I never
read → "read first to avoid overwriting the sibling's changes"; mtime drifted (external edit);
last read was paginated (`partial` flag) → "re-read the whole file before overwriting"; never
read at all. It never raises — callers decide block vs. warn. `writes_since` (`:218`) lets
`delegate_task` append a "subagents modified files you previously read" reminder to the
delegation result. Caps: 4096 paths/agent (`:53`), 4096 global writers (`:56`), insertion-order
trim. **Lands in:** a new `file_state` module in `epsilon-tools` (or `epsilon-session` if the
stamp should survive restarts), consulted by `write`/`edit`/`mutation` before commit and by
`task` on child return.

### 3. Path-conflict-aware parallel batch planner

`_plan_tool_batch_segments` (`agent/tool_dispatch_helpers.py:117`) returns ordered
`(kind, calls)` segments where `kind ∈ {parallel, sequential}`. It walks the model's calls in
order, closing the current parallel run whenever a barrier appears, so a later call never
crosses an earlier barrier and result ordering is identical to fully-sequential execution.
Path-scoped tools (`read_file`, `search_files`, `write_file`, `patch` — `:69`, `:70`) join a
run only if their canonical paths don't conflict with an existing reservation (`:204`):
reader↔reader overlap is harmless and stays parallel; *any* overlap involving a writer closes
the run. Reservations carry a writer bit (`:152`). Unparseable or non-dict args become barriers
(`:176`). `search_files` reserves its search root as a reader, so a search batched after a
write into that subtree is ordered behind it. Runs shorter than two calls demote to sequential
and adjacent sequential segments merge (`:227`). `_should_parallelize_tool_batch` (`:238`) is
the old all-or-nothing view kept as a thin wrapper. **Lands in:** `epsilon-core/src/exec.rs`
— replaces the current barrier rule with reservations, keeping the bounded pool.

### 4. Tool-loop guardrail controller

`ToolCallGuardrailController` (`agent/tool_guardrails.py:334`) resets per turn (`:341`) and
keeps four counters: exact-failure per `(tool, sha256(canonical args))`, same-tool failure per
tool name, no-progress per signature keyed on result hash, and an identical-call streak.
`canonical_tool_args` (`:286`) is sorted compact JSON so arg key order can't dodge detection.
`before_call` (`:376`) enforces per-turn `LoopCapConfig` (`:186`, defaults 50 web searches /
50 subagents — a value of 0 disables) and, only if `hard_stop_enabled`, blocks on
`exact_failure_block_after` or `no_progress_block_after`. `after_call` (`:431`) increments and
returns `warn` at `exact_failure_warn_after` (2) / `same_tool_failure_warn_after` (3), `halt`
at 8. Crucially the defaults ship **warnings on, hard stops off** (`:118`) so an interactive
session gets a nudge, not a circuit breaker. `observe_call` (`:541`) additionally replaces a
byte-identical duplicate result with a reference stub above 512 chars (`:94`) and appends a
loop-breaker notice; `append_toolguard_guidance` (`:739`) folds it into the result text.
**Lands in:** `epsilon-core/src/exec.rs` beside the existing doom-loop brake, with the
thresholds in `epsilon-core/src/config.rs`.

### 5. Subdirectory context hints

`SubdirectoryHintTracker` (`agent/subdirectory_hints.py:72`) hooks *after* each tool call.
`check_tool_call` (`:117`) extracts candidate directories from `path`/`file_path`/`workdir`
args (`:41`) and, for shell tools, from path-like tokens in the command (`:191`). Each
candidate walks up toward the root, stopping at the first already-loaded directory or after
`_MAX_ANCESTOR_WALK = 5` (`:48`), so reading `project/src/main.py` discovers
`project/AGENTS.md`. `_load_hints_for_directory` (`:256`) tries
`AGENTS.override.md`, `AGENTS.md`, `agents.md`, `CLAUDE.md`, `claude.md`, `.cursorrules`
(`:30`, first match wins), rejects anything outside the working tree, skips excluded dir
names (`:53`), runs the same context-content scan as startup loading, clamps to 8 KB (`:38`),
and — the good part — dedupes on a SHA-256 of the content (`:295`) so the same `AGENTS.md`
reached through a symlink farm or a copied backup is never injected twice. The working dir's
own file digest is pre-seeded (`:95`) because startup already loaded it. Output is appended to
the tool result as `[Subdirectory context discovered: <rel path>]`. **Lands in:**
`epsilon-core/src/prompt.rs` (loader) + `epsilon-core/src/exec.rs` (append to result).

### 6. Project verification recipe detection

`Recipe` (`agent/verify/recipes.py:35`) is the data shape: `name`, `kind`, `bootstrap[]`,
`build[]`, `test[]`, `start`, `port`, `readiness_path`, `evidence[]`. `detect_package_manager`
(`:140`) is lockfile-ordered (`pnpm-lock.yaml` → pnpm, `bun.lock` → bun, then yarn/npm/uv/
poetry/pipenv). Per-ecosystem detectors (`:193` node, `:259` python, `:347` go, `:360` rust,
`:373` java, `:406` make, `:442` compose) pick commands from manifests and Makefile targets.
`_infer_port_from_command` (`:158`) reads the port out of the start command. `detect_recipe`
(`:459`) is the entry point. `agent/verify/environment.py` caches the result as a manifest at
a known path (`:21`, `:49`) and exposes `load_or_detect` (`:66`) so detection runs once.
`agent/verify/runner.py:242` executes the phases in `PHASE_ORDER = (bootstrap, build, test)`
with a 600 s default phase timeout, then polls readiness (`:146`) and terminates the process
group (`:162`). **Lands in:** a `recipe.rs` in `epsilon-tools` (detection is pure filesystem
work) feeding the #1 evidence store and `epsilon-cli` for an explicit verify entry point.

### 7. `@`-reference expansion

`BUILTIN_PREFIXES` (`agent/context_references.py:24`) is `diff`, `staged`, `file`, `folder`,
`git`, `url`; plugins may register more (`:61`). `parse_context_references` (`:148`) runs a
regex with a quoted alternative for space-containing paths (`format_reference_value` at `:133`
is the inverse), strips trailing punctuation, and parses `file:path:10-20` into line window
fields. `preprocess_context_references` (`:212`) resolves `allowed_root` (defaults to cwd, so
refs cannot escape the workspace), expands all refs concurrently with `asyncio.gather`
(`:267`), and returns a `ContextReferenceResult` carrying `injected_tokens` and a `blocked`
flag. `_ensure_reference_path_allowed` (`:484`) refuses sensitive targets — `_SENSITIVE_HOME_DIRS`
at `:92` lists `.ssh`, `.aws`, `.gnupg`, `.kube`, `.docker`, `.azure`, `.config/gh` — plus
sensitive home files and the skills hub dir. Expanders: file with line windows (`:368`), folder
listing capped at 200 entries (`:402`/`:590`), git (`:419`), url. Binary files get a metadata
header instead of bytes (`:681`). **Lands in:** `epsilon-core` message preprocessing, before
the loop builds the user turn.

### 8. Untrusted tool-result wrapping + elision notice

Two independent result-shaping passes at message-construction time (never mutated later, so
prompt caching is safe). Untrusted sources: `web_extract`/`web_search` by name (`:589`) plus
`browser_`/`mcp_` prefixes (`:594`); results under 32 chars skip the wrapper (`:599`). The
wrapper is `<untrusted_tool_result>` and — the non-obvious part — `_DELIMITER_TOKEN_RE`
(`:604`) matches the token **case-insensitively** before wrapping, so attacker content cannot
forge or prematurely close the boundary with a differently-cased variant. Separately,
`_UPSTREAM_ELISION_PATTERNS` (`:628`) matches provider-side truncation markers
(`"...13 more items"`, `"has_more": true`, `saved to sandbox`, `data_preview`); when one is
found in a result over 1000 chars, `_maybe_append_elision_notice` (`:665`) appends one compact
"[this data is INCOMPLETE]" note. The stated reason is empirical: the result looks
structurally complete, so models treat the visible slice as the whole dataset and claim
completeness. `_tool_output_risk_metadata` (`:679`) scans untrusted text for threat patterns
and attaches internal-only risk metadata without retaining the text. **Lands in:**
`epsilon-tools/src/util.rs` result shaping + the tool-result constructor in `epsilon-core`.

### 9. LSP diagnostics as post-edit feedback

`LSPService` (`agent/lsp/manager.py:134`) owns a background loop (`:65`) so sync tool code can
drive async LSP calls (`:101`). The mechanism that matters: `snapshot_baseline` (`:297`) is
called *before* a write and stores the file's current diagnostics; `get_diagnostics_sync`
(`:321`) filters the post-edit set against that baseline so the model only sees errors its
own edit introduced. Because an edit shifts line numbers, baseline diagnostics are remapped
through `line_shift` first — `agent/lsp/range_shift.py:33` `build_line_shift(pre_text,
post_text)` — otherwise every pre-existing error below the edit point looks newly introduced
(`shift_baseline` at `:136`). Servers idle out after 600 s (`:61`, floor 30 s), and a
timeout on `(server_id, workspace_root)` marks the pair broken (`:416`) so later edits skip it
instead of re-paying the timeout. Returns `[]` and never raises when LSP is disabled, no
workspace matches, or the server won't spawn. **Lands in:** a new `epsilon-lsp` crate (new
crate → ADR required per AGENTS.md) or `epsilon-ext` as a supervised child; consumed by the
`edit`/`write` tool result path in `epsilon-tools`.

### 10. Middleware contract

Four kinds, each registered from a plugin and each with a distinct interception point
(`docs/middleware/README.md:43`): `llm_request` replaces provider kwargs before the call;
`llm_execution` wraps the provider call with a `next_call` callback; `tool_request` replaces
tool args **before** guardrails and approvals see them; `tool_execution` wraps the actual tool
call. Execution ordering is spelled out (`:79`): parse args → `tool_request` → availability
checks, observer blocks, guardrails, approval → `tool_execution` → post-tool hooks →
`transform_tool_result`. Request middleware may return a `source`/`reason` pair that is
recorded into later hook payloads as `middleware_trace`. Failure semantics are the careful
part (`:75`, `:240`): middleware failures are **fail-open**; if execution middleware raises
*before* `next_call` the base path still runs; if it raises *after* `next_call` succeeded the
downstream result is preserved and the provider/tool is **not** re-run; a downstream failure is
never converted into a successful `None`. Multiple registrations nest in registration order.
The doc also states the risk plainly: tool-request middleware mutates exactly the values
downstream policy evaluates. **Lands in:** `epsilon-ext` — extend `ExtManifest` beyond
`wants_event` with per-kind middleware registration, and define the chain in
`epsilon-core/src/exec.rs` (tool path) and `epsilon-llm` (request path).

### 11. Per-model guidance injection

Prompt text as a first-class port. `TASK_COMPLETION_GUIDANCE` (`agent/prompt_builder.py:411`)
is the no-fabrication rule — "the deliverable is a working artifact backed by real tool
output, not a description of one… NEVER substitute plausible-looking fabricated output for
results you couldn't actually produce." `PARALLEL_TOOL_CALL_GUIDANCE` (`:454`) tells the model
to batch independent calls, which is exactly what Epsilon's bounded pool is built to exploit
but nothing currently steers. `OPENAI_MODEL_EXECUTION_GUIDANCE` (`:484`) is structured as XML
blocks — `<tool_persistence>`, `<mandatory_tool_use>` (do arithmetic/hashes/time/system-state
via tools, never mentally), `<act_dont_ask>`, `<prerequisite_checks>` — and its own comment
records that it started fenced to gpt/codex/grok and was un-fenced after eval traces showed
DeepSeek/Kimi doing financial math in prose and "repairing" malformed identifiers.
`STEER_CHANNEL_NOTE` (`:778`) documents the mid-turn steer marker to the model and — the
subtle half (`:795`) — adds the chronology rule: a replayed marker in history is not a new
instruction. Injection gates are model-substring lists (`:374`, `:390`) resolved once per
session in `agent/system_prompt.py:474`–`:524` to keep the prompt byte-stable. **Lands in:**
`epsilon-core/src/prompt.rs` STABLE tier constants + a model-name gate in `epsilon-llm`'s
catalog lookup.

### 12. Skill lint, AST audit, install scan, provenance ledger

Four tools. `skill_linter.py:374` `lint_content` checks frontmatter (name matches dir `:119`,
name format `:139`, description `:155`, metadata block `:187`), flags marketing words (`:60`),
POSIX primitives (`:73`), forbidden files (`:86`), expected sections like "When to Use"
(`:97`), reference links resolving on disk (`:262`), and platform gating (`:291`) — findings
carry `error`/`warning` severity. `skills_ast_audit.py:25` parses Python in skill scripts and
reports dangerous constructs; `:84` `ast_scan_path` walks a skill dir. `skills_guard.py` scans
at install time: `THREAT_PATTERNS` (`:101`), `INSTALL_POLICY` by trust level (`:55`),
structure limits (50 files / 1 MB total / 256 KB per file, `:532`), invisible-character
detection (`:550`), `.skillignore` support (`:1054`), and a content digest cache (`:699`).
`skill_ledger.py` is the rollback machinery: content-addressed blobs (`:114`), per-mutation
entries recording before/after path snapshots (`:138`, `:196`), an actor attribution
(`curator`/`agent`/`user`, `:43`) derived from a contextvar, and `rollback_entry` (`:300`)
which validates every path is inside the root before restoring. **Lands in:** `epsilon-core`
skills module (lint + audit) and a `skill_provenance` store next to the session store.

### 13. Workspace checkpoints

`tools/checkpoint_manager.py` keeps a bare-git shadow store under
`<home>/checkpoints` (`:72`) with `store/`, `refs/hermes`, `indexes/`, `projects/`, `ledgers/`
(`:76`–`:79`). Each working directory gets a `_project_hash` (`:205`) and a registered project
record (`:577`), its own ref (`:281`), and a per-file ledger mapping path → hash, capped at
2000 entries (`:83`) with a `_hash_file` helper (`:235`). Writes go through `_run_git` with a
controlled env (`:293`, `:355`) and a `_MAX_FILES = 50_000` ceiling (`:152`), with a repairs
pass for damaged bare repos (`:333`) and commit-hash/path validation before any restore
(`:162`, `:178`). `prune_checkpoints` (`:1726`) and `maybe_auto_prune_checkpoints` (`:2003`)
manage growth, and `_workdir_is_observably_gone` (`:1626`) stops a deleted directory's
checkpoints from accumulating forever. `DEFAULT_EXCLUDES` (`:85`) keeps node_modules/.git
etc. out. The value is the combination: a *workspace* rewind counterweight to session-level
fork, with a ledger so you can see which files a checkpoint owns. **Lands in:** new
`epsilon-checkpoint` module or inside `epsilon-session`; CLI/TUI needs a restore surface.

### 14. Background review fork

`agent/background_review.py` runs a second, short agent after a turn with its own cancel
path. `_REVIEW_MAX_ITERATIONS = 16` (`:201`) bounds it; `_digest_history` (`:353`) takes only
the last 24 messages as input; three prompt variants select the job —
`_MEMORY_REVIEW_PROMPT` (`:401`), `_SKILL_REVIEW_PROMPT` (`:412`), `_COMBINED_REVIEW_PROMPT`
(`:537`). The run is bracketed by `prepare_background_review_run` (`:75`) /
`finish_background_review_run` (`:97`), with `cancel_background_review_for_live_turn`
(`:148`) so a new user turn preempts a stale review instead of racing it. Token usage is
attributed back to the parent (`:906`) and the outcome is classified (`:969`) so the parent
knows whether the fork wrote anything. `_run_review_in_thread` (`:1013`) installs an
auto-deny approval callback (`:1044`) — the review fork may never prompt the user. Its own
comment notes the fork inherits the cached system prompt verbatim, which is what keeps the
review cheap. **Lands in:** `epsilon-core` as a post-turn hook spawning a child `Agent` with
an auto-deny permission ruleset and a hard iteration cap.

### 15. Compaction invariant: user turns are never summarized

The design argument in `docs/micro-compaction.md:63`–`:85` is the portable part, independent of
whether micro-compaction itself is adopted: an *exchange* deliberately starts at the assistant
message, so what the user typed is never paraphrased — "they cannot be reconstructed from the
work that followed." Everything else in the doc is mechanism worth borrowing selectively:
the cursor is recovered by scanning the transcript for the last summary marker rather than
persisted (`:100`), so a resumed session never re-summarizes; exactly one rolling summary
exists and older markers are dropped as superseded (`:115`); a defrag pass re-summarizes the
summary itself past a token threshold (`:126`); and a summarizer that fails three times on the
same exchange advances the cursor anyway (`:158`) so one bad exchange can't wedge every
subsequent turn. Epsilon's `epsilon-core/src/compact.rs:53` currently serializes *user* turns
into the summary body alongside tool results, keeping only a protected tail window
(`MIN_TAIL_TOKENS`/`MAX_TAIL_TOKENS` at `:46`). Adding a user-turn filter plus a
marker-scan cursor recovery is a small, high-value change; adopting per-turn amortization is a
separate, cache-costing decision.

### 16. Background process registry

`ProcessRegistry` (`tools/process_registry.py:424`) with `ProcessSession` (`:367`) tracks
detached shell processes. Bounds are explicit: 200 KB rolling output buffer (`:62`), finished
processes kept 30 min (`:63`), 64 concurrent tracked processes with LRU pruning (`:64`), 24 h
max active age (`:65`). `_emit_output` (`:506`) appends and fans out; `_check_watch_patterns`
(`:517`) matches per-process watch regexes against new text with a 15 s minimum spacing
between consecutive matches (`:73`) and disables the watch after 3 consecutive strikes (`:74`),
promoting to notify-on-complete. A global admission gate (`:648`) rate-limits watches to 15
per 10 s window with a 30 s cooldown (`:78`–`:80`) — the fix for a chatty process turning
notification into a flood. On Linux it can wrap processes in systemd user scopes
(`:273`, `:170`) so they survive and can be stopped as a unit. State checkpoints to
`processes.json` (`:59`). **Lands in:** `epsilon-tools/src/shell.rs` (spawn/track) plus a
registry owned by `epsilon-core`, surfaced through the existing event bus.

### 17. Empty-response guard and repetition guard

Two terminal-quality gates that run *after* a stream succeeds, which is exactly the hole
Epsilon's never-fail contract leaves. `empty_response_guard.py`: `_zero_output` (`:173`) and
`deterministic_empty` (`:234`) detect a finish with no content; `record_empty_attempt`
(`:204`) tracks a signature per attempt; `empty_retry_budget` (`:254`) returns a full budget
(3, `:56`) normally but a reduced one (1, `:57`) once the streak's estimated cost passes a
threshold (default $0.25, `:58`), so a provider that returns empty on expensive requests stops
being retried blindly. `repetition_guard.py:43` `is_repetition_dominated` requires a minimum
fragment length (400 chars), a 60-line window, at least 5 repeats and a 0.5 dominance ratio
(`:28`–`:40`), with a line-mode fallback (`:84`) — a conservative predicate that won't false-
positive on legitimate repetitive output. **Lands in:** `epsilon-core/src/compact.rs`'s
neighbour or a new `quality.rs`, wired into the loop's terminal-frame handling.

### 18. Provider-facing message and stream sanitization

The repairs a harness with many provider wires eventually needs, all keyed to real API
behaviors. `_repair_tool_call_arguments` (`agent/message_sanitization.py:195`) recovers
malformed tool-call JSON (with a bounded log of full args at `:192`).
`close_interrupted_tool_sequence` (`:296`) closes an assistant tool-call block that never got
results — the case where a retry or cancel leaves the transcript structurally invalid for the
next request. `deterministic_call_id` (`:527`) derives a stable id from
`(name, args, index)` rather than a random one, and `uniquify_tool_call_ids` (`:551`)
disambiguates collisions — both matter for prefix caching and for providers that reject
duplicate ids. `apply_reasoning_content_policy` (`:714`) plus `reasoning_echo_family` (`:695`)
decide per provider family whether prior reasoning content must be echoed back or stripped,
and `reapply_reasoning_echo` (`:805`) re-applies it after a repair pass. Surrogate and
non-ASCII sanitization (`:32`, `:330`) prevents encode-time failures. **Lands in:**
`epsilon-llm`'s request-building path, alongside the existing error-classified fallbacks.

---

## Deliberately do not port

- **Cron scheduler** (`cron/scheduler.py`, 60 s gateway tick, file-lock via `fcntl`/`msvcrt`):
  Epsilon's Architecture §14 excludes it; a coding harness needs no scheduler, and the
  Windows/POSIX lock split is pure liability.
- **Kanban tools** (`tools/kanban_tools.py`, 2.5k lines, `KANBAN_GUIDANCE` prompt block): a
  multi-agent work-queue product with its own PDF spec. Not a harness primitive.
- **Browser/CDP stack** (`tools/browser_*.py`, ~13k lines, camofox, supervisor, dialogs):
  enormous surface, needs a real browser, and `webfetch` covers the coding case.
- **Computer-use / desktop UI tools** (`tools/computer_use_tool.py`, `desktop_ui.py`,
  `apply_layout_tool.py`, `gui_tour`): driving a GUI is not what a coding harness is for.
- **Media generation** (`image_generation_tool.py`, `video_generation_tool.py`, `tts_tool.py`,
  `transcription_tools.py`, `vision_tools.py`, `flux3_video_tool.py`): ~14k lines of provider
  adapters for non-code modalities.
- **Messaging gateway and platform adapters** (`gateway/`, `plugins/platforms/`,
  `send_message_tool.py`, `react_to_message_tool.py`, `discord_tool.py`): Epsilon's server/NDJSON
  frontends already cover editor and programmatic clients.
- **Billing/credits/subscription accounting** (`agent/billing_usage.py`, `credits_tracker.py`,
  `account_usage.py`, `subscription_view.py`, `usage_pricing.py` beyond per-token cost):
  Epsilon has cost tracking; invoicing, spending-limit surfaces and subscription views are a
  hosted-product concern.
- **MoA loop** (`agent/moa_loop.py`, 2.4k lines) and **relay runtime** (`relay_*.py`):
  mix-of-agents routing and a hosted relay are architecture choices that would fight Epsilon's
  single-loop + fallback-chain design.
- **Native compaction for one vendor** (`agent/native_compaction.py`, `_ELIGIBLE_MODEL_MARKER =
  "gpt-5.6"` at `:62`): vendor-specific context management Epsilon can't generalize.
- **`hermes_state.py` at 592 KB** / **`cli.py` at 999 KB**: not a design to imitate — the
  single-file god-module is the thing Epsilon's crate boundaries exist to prevent.
- **Emergency-stop file sentinel** (`agent/estop.py`): genuinely useful, but for a single-user
  local CLI, Ctrl-C and the run-scoped cancellation token already cover it.
- **Task-completion / verification *scaffolding* removal on the resume path**
  (`agent/turn_finalizer.py:107` `_drop_verification_continuation_scaffolding`): a hack Hermes
  needs because nudges are appended into history. Port the nudge (#1), not the cleanup.
- **82 skills under `skills/`** plus `optional-skills/`: content, not mechanism. The
  `popular-web-designs` / `interior-design` / `smart-home` categories are evidence that a
  general assistant's skill library is the wrong shape for a coding harness.

---

## Suspicious content encountered

Nothing in the mined files attempted to instruct the auditor. Two things to note as *data*
rather than instruction: (a) `tools/skills_guard.py:101` `THREAT_PATTERNS` is a list of
prompt-injection and exfiltration patterns that reads, in isolation, like an attack cookbook —
it is a defensive scanner definition; (b) `agent/prompt_builder.py:778` `STEER_CHANNEL_NOTE`
is model-facing text that explicitly tells the model which in-band markers to trust, and it is
the one place Hermes' own source could be spoofed if a tool result forged the marker —
worth reading before porting #8 or #11, since Epsilon's steering channel uses the same idea.

No secret values are reproduced in this report; credential-handling was cited by file location
only (`agent/redact.py`, `agent/secret_scope.py`, `agent/credential_persistence.py`,
`agent/secret_sources/`) and none of those files were read for content.
