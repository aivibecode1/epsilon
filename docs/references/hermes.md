# Hermes Agent

Hermes Agent is one of the reference harnesses under `references/`. It is read-only input: this
document records what Epsilon took from it, what it changed, and what it left behind.

Paths into `references/` are written as inline code rather than as links. That directory is
gitignored (`.gitignore`), so a link into it would be dead in a fresh clone. Links in this
document point only at files this repository commits.

## What it is

Hermes describes itself as "**The self-improving AI agent built by Nous Research.** It's the only
agent with a built-in learning loop — it creates skills from experience, improves them during use,
nudges itself to persist knowledge, searches its own past conversations, and builds a deepening
model of who you are across sessions" (`references/hermes-agent-2026.9.24/README.md:19`). Its
`package.json` is narrower: "An AI agent with advanced tool-calling capabilities, featuring a
flexible toolsets system for organizing and managing tools" (`package.json:4`).

**Version vendored: 2026.9.24 — which is a release date, not a version string.**
`hermes_cli/__init__.py:6-7` reads `__version__ = "0.21.5"` and `__release_date__ =
"2026.9.24"`, and `pyproject.toml:5` agrees with the first: `version = "0.21.5"`. The npm tree
here is the JavaScript front ends and is not versioned for the agent at all: the root
`package.json` reads `"version": "1.0.0"` and `"private": true` (`package.json:3-5`). Quote the
release date when naming the vendored copy, and `0.21.5` when naming the Python package.

Site: <https://hermes-agent.nousresearch.com/>.

**Shape.** Mostly Python, with TypeScript front ends. `run_agent.py` is the agent facade,
`cli.py` the CLI, `model_tools.py` the tool dispatcher, `toolsets.py` the toolset table,
`hermes_state.py` plus its `hermes_state_*.py` siblings the session database,
`agent/` the turn loop split one file per phase, `tools/` the model tools, and `gateway/` the
messaging platforms. `ui-tui/` is an Ink/React terminal UI and `apps/desktop/` an Electron app.
The turn loop is synchronous (`agent/AGENTS.md`).

**Two invariants are declared at the top of its own development guide**, and they explain most of
the rest: "**Per-conversation prompt caching is sacred.** A long-lived conversation reuses a
cached prefix every turn" — so nothing mutates past context mid-conversation except compression —
and "**The core is a narrow waist; capability lives at the edges.** Every model tool is sent on
every API call, so the bar for a new *core* tool is high" (`references/hermes-agent-2026.9.24/AGENTS.md`).

## Who made it, and under what licence

- **Licence: MIT.** `LICENSE` reads "MIT License" and "Copyright (c) 2025 Nous Research".
  `package.json:32` agrees: `"license": "MIT"`. The README carries the same badge
  (`README.md:12`) and closes with "MIT — see [LICENSE](LICENSE). Built by Nous Research"
  (`README.md:260-264`).
- **Repository: <https://github.com/NousResearch/Hermes-Agent>**, from the `repository.url` field
  in `package.json:28-31` (`git+https://github.com/NousResearch/Hermes-Agent.git`). The `bugs`
  and `homepage` fields (`package.json:33-36`) point at the same place.
- **Organisation: Nous Research** (<https://nousresearch.com>), named in the copyright line, in
  the README badge ("Built by Nous Research", `README.md:13`) and in the closing line. The
  vendored copy names **no individual authors** — not in `LICENSE`, not in `package.json`. The
  README's community section (`README.md:250-257`) names Discord, the Skills Hub and the issues
  tracker, not people. Individual authorship is *not stated in the vendored copy*.
- **Project site:** <https://hermes-agent.nousresearch.com/>.

## Its strengths, in its own terms

Hermes is quoted and paraphrased from its own README, its own `AGENTS.md` files and its own
module docstrings.

**One agent core behind a CLI, a TUI, a desktop app and about twenty messaging platforms.**
"Hermes is a personal AI agent that runs the same agent core across a CLI, a messaging gateway
(Telegram, Discord, Slack, ~20 platforms), a TUI, and an Electron desktop app"
(`AGENTS.md`). The gateway front is a separate service, not a mode of the CLI: `hermes gateway
setup`, then `hermes gateway start`, then you talk to it from Telegram
(`README.md:143-159`).

**A learning loop that writes things down.** "Agent-curated memory with periodic nudges.
Autonomous skill creation after complex tasks. Skills self-improve during use. FTS5 session
search with LLM summarization for cross-session recall" (`README.md:26`).

**A goal is a thing that runs without you.** `hermes_cli/goals.py:1-7` describes it as "the Ralph
loop for Hermes. A goal is a free-form objective that stays active across turns; after each turn
an auxiliary-model judge decides whether it is satisfied", with two details that matter: the
continuation prompt "is a normal user message appended via `run_conversation` (no system-prompt
mutation or toolset swap — prompt caching stays intact)" and "Judge failures are fail-OPEN
(`continue`); the turn budget is the backstop".

**Completion can be checked by a command instead of by a model.** The `GoalGate` dataclass is
defined in one place and says what it is for: "A deterministic shell command that must pass
before a goal can be done. Gates run at turn boundary BEFORE the LLM judge; a failing gate
short-circuits judging and its bounded output becomes the continuation prompt"
(`hermes_cli/goals.py:343-349`; `run_gate` at `hermes_cli/goals.py:374-391`). The continuation
prompt is a named constant with a fixed shape — goal, command, exit code, a bounded output tail
in a fence, then two sentences: "Fix the underlying problem so this gate passes, then re-run it to
confirm. Do not declare the goal complete while any gate fails. If the gate itself is wrong or
cannot pass, say so clearly and stop" (`hermes_cli/goals.py:96-111`).

**Verification is a ledger, and the ledger refuses to overstate itself.**
`agent/verification_evidence.py:1-3` describes itself as "Coding verification evidence ledger:
records what the agent actually proved in a code workspace. **Deliberately passive — it never runs
a suite, never blocks completion, and never upgrades targeted checks into "repo green".**" The
turn-end nudge is separate and equally careful: "Policy-only: it never runs checks itself, it
turns the passive verification ledger into a bounded follow-up when the model tries to finish
right after editing code without fresh evidence" (`agent/verification_stop.py:1-3`). The nudge
also knows when to stay quiet — a turn touching only prose is exempt
(`agent/verification_stop.py:15-23,29-36`).

**The observer knows what it has actually seen.** `tools/file_state.py:1-9`: "Cross-agent file
state coordination. Prevents mangled edits when concurrent subagents (same process, same
filesystem) touch the same file: B writes a file A already read, so A's next write would clobber
B's changes." The read stamp carries a third field for exactly the case a flag cannot express:
"partial=True when read_file returned a windowed view (offset > 1 or limit < total_lines) — a
later write should still warn so the model re-reads in full" (`tools/file_state.py:20-23`). The
write-side guard is one function: `_stale_overwrite_blocker` (write_file only, under the per-path
lock) "refuses a whole-file overwrite of content this task never saw or that changed since"
(`tools/file_tools_write_guards.py:9-11`). The registry is bounded —
`_MAX_PATHS_PER_AGENT = 4096` — because a long session must not accumulate state without limit
(`tools/file_state.py:25-27`).

**Content from outside is not the same as the user's words.** The tool-result constructor carries
"untrusted-content wrapping" (`agent/tool_dispatch_helpers.py:1-8`), and `agent/threat_patterns.py`
is imported alongside it (`agent/tool_dispatch_helpers.py:23`).

**Recovery paths assume something went wrong.** The turn loop is split so each phase is one file:
`turn_overflow`, `turn_truncation`, `turn_context_compaction`, `turn_recovery`,
`turn_recovery_autorecover` ("post-exhaustion wait-and-retry ladder"), `turn_retry_state`,
`turn_stop_gates`, `turn_liveness` (`agent/AGENTS.md`). Three guards for a stuck run —
`agent/tool_guardrails.py`, `agent/empty_response_guard.py`, `agent/repetition_guard.py` — each in
its own file. And a stalled summary falls back deterministically rather than hanging
(`agent/AGENTS.md`, "Compression").

**Checkpoints that cover the workspace, not just the files the agent touched.**
`tools/checkpoint_manager.py:1-11` describes "transparent filesystem snapshots via one shared
shadow git store", taken "before file-mutating tool calls (once per directory per turn)", with an
exclude list that names secrets (`.env`, `.env.*`, `.env.local`), caches and build output
(`tools/checkpoint_manager.py:49-60`).

## What we adopted

Taken as documented behaviour and reimplemented in Rust. Each item names where it lives in
**this** repository, confirmed against the source.

**Read observations, scoped to a lifetime.** `crates/epsilon-tools/src/read_log.rs:1` names the
source (`Hermes tools/file_state.py parity`). What is seen is *coverage*, not a flag: the snapshot
records whether a read reached the end of the file, how many contiguous lines from line 1 it
covered, and whether the byte budget alone stopped it
(`crates/epsilon-tools/src/read_log.rs:40-70`). A whole-file overwrite needs the whole file — "a
page of a 5 MB file is not a license to replace it"
(`crates/epsilon-tools/src/read_log.rs:24-30`).

**Observations belong to a lifetime token, not to a session name.** `record_read`, `record_write`
and `record_edit` all take an `owner: &Arc<()>` (`crates/epsilon-tools/src/read_log.rs:226,284,331`),
and the store keeps it as a `Weak` so a retired map is reclaimed on the next access
(`crates/epsilon-tools/src/read_log.rs:32-34,124-143`). Children share mutation locks but not read
history. The calls are production, not test-only: `crates/epsilon-tools/src/files.rs:226,365,552`.

**The refusal names the path and the reason.** A mismatch produces a typed `Stale`
(`crates/epsilon-tools/src/read_log.rs:75`) whose `diagnosis` is written for the model to act on
(`crates/epsilon-tools/src/read_log.rs:99`), reached from the write and edit paths at
`crates/epsilon-tools/src/files.rs:353,528`.

**Verification evidence attached to the claim, not asserted beside it.**
`crates/epsilon-core/src/verify.rs:1-2` names the source. The module is explicit about what it
is not: "It is deliberately *evidence*, not enforcement: the harness does not run the checks
itself and does not refuse to stop. It records what happened, and at the stop point it says once —
twice at most — that the edits have nothing behind them. The model decides"
(`crates/epsilon-core/src/verify.rs:10-12`). What counts as a check is a table, not a regex
(`crates/epsilon-core/src/verify.rs:22-70`).

**A gate is part of the objective.** `GoalGate` lives on the goal snapshot beside the objective,
because "the goal is not done until the tests pass" is a fact about the goal
(`crates/epsilon-contracts/src/goal.rs:50-69`), and the module names Hermes as the source of the
idea (`crates/epsilon-contracts/src/goal.rs:59-63`).

**A failing gate's output is the next round's prompt.** `begin_goal_round` runs the gates before
the model is asked to judge completion
(`crates/epsilon-runtime/src/lib.rs:1432-1450`, and the prompt builder at
`crates/epsilon-runtime/src/lib.rs:1501-1517`). The round is spent either way, so a red suite
cannot be retried for free (`crates/epsilon-runtime/src/lib.rs:1442-1445`).

**The gate runs through the same sandbox the `bash` tool uses.** `crates/epsilon-tools/src/goal_gate.rs:3-17`
states the security reason — a gate outside confinement would be "a **confused deputy**", and one
command the agent cannot run but the goal can would turn the permission gate into decoration.

**An unchanged workspace does not re-pay for the same failure.** An unchanged tree with the same
failing gate is reported from the last result rather than recomputed
(`crates/epsilon-tools/src/goal_gate.rs:19-28`).

**Three pure predicates for a stuck run.** `crates/epsilon-core/src/guards.rs:1-2` names the
three Hermes modules, and the module states what the doom-loop brake cannot see: "A stuck run
usually looks different — four different commands that all fail the same way, or a model that
answers with nothing and retries forever" (`crates/epsilon-core/src/guards.rs:3-7`).

**Content this session did not author is shaped before it reaches the model.**
`crates/epsilon-core/src/untrusted.rs:1-2` names the source. Two passes at message construction,
applied once and never mutated later so prompt caching is unaffected: a wrapper the content cannot
forge, with every case-variant of the delimiter neutralised first, and an incompleteness notice
for silently truncated payloads (`crates/epsilon-core/src/untrusted.rs:10-22`).

**A fallback chain that starts at the primary every turn.** `crates/epsilon-llm/src/fallback.rs:1-13`
names `try_activate_fallback` and takes "per-turn primary restoration" from it: "each call starts
again at the head of the chain, so once the primary recovers it is used automatically. No sticky
state, nothing to reset."

**A health probe with a time-to-live, kept in the MCP client.** Hermes' availability check is a
`check_fn` whose result is cached process-wide
(`crates/epsilon-mcp/src/lib.rs:44,72,110` names the pattern and states the shortfall). Epsilon
keeps the TTL and moves it to the adapter that owns the tool, which is a placement change rather
than a semantic one.

**A durable `MEMORY.md` tier beside the context files.** `crates/epsilon-core/src/prompt.rs:204-219`
— the global tier loads first so workspace specifics override it, both hash-tagged into the same
CONTEXT tier so the prompt cache is not disturbed.

**Turn leases, so two processes cannot write one session.** `crates/epsilon-session/src/lease.rs:1`
names "Hermes' turn-lease pattern".

## What we adapted

Taken, then changed on purpose.

**The whole-workspace rewind became a per-file revert of this harness's own edits.** The contrast
is stated in the port's own header: Hermes has a "bare-git shadow store, per-project refs, a
50 000-file index" and does a "whole-workspace rewind"; Epsilon has "a content-addressed blob
store plus an append-only ledger of the files *this harness* changed" and does a "per-file
revert" (`crates/epsilon-tools/src/checkpoint.rs:8-18`). The reason is a disagreement about what
is being promised: a store that can rewind files the harness never touched is a bigger claim than
the one that needs making, and the trade buys a store that needs no `git`, works in a directory
that is not a repository, and cannot fail because an index went bad
(`crates/epsilon-tools/src/checkpoint.rs:16-18`). Layout is `blobs/` plus `ledger.jsonl`
(`crates/epsilon-tools/src/checkpoint.rs:20-29`), recording is wired into `write` and `edit`
(`crates/epsilon-tools/src/files.rs:356,533`).

**Scope, stated plainly: the recording half is reachable and the restoring half is not.**
`CheckpointStore::restore` (`crates/epsilon-tools/src/checkpoint.rs:322`) has no production caller
in this tree — no tool, no slash command, no front-end panel; its only callers are the tests in
its own file. So a session's checkpoints are written, bounded, eviction-reported, and not yet
offered to anybody. Hermes' manager is "controlled by the `checkpoints` config / `--checkpoints`
flag" (`tools/checkpoint_manager.py:4`); the flag Epsilon needs for this has not been built.

**A restore must verify the file before overwriting it.** Hermes's guard is about what the *task*
saw. Epsilon's checkpoint entry carries the digest of the bytes as they were when it was
captured, and the reason is about the *user*: "Restoring over a file somebody has since edited
destroys their work, and a snapshot that cannot tell whether that happened is not a preimage — it
is a loaded gun pointed at the user's last hour" (`crates/epsilon-tools/src/checkpoint.rs:60-72`).
An entry with no digest refuses rather than assuming, "because the assumption it would have to
make is the dangerous one" (`crates/epsilon-tools/src/checkpoint.rs:67-70`).

**The security claim is narrower, and says so.** The stale-read guard's header states its own
limit: `bash` runs the platform shell and can overwrite any file with a redirect, `shell.rs`
consults nothing here, and `checkpoint::restore` also writes bytes back — "So this guard closes
the route *through the file tools*, and a model with shell access can still destroy a file it
never read" (`crates/epsilon-tools/src/read_log.rs:9-15`). A guarantee stated at its real
boundary is worth more than a larger one that is not true.

**The goal has no LLM judge.** Hermes runs an auxiliary-model judge and gates in front of it
(`hermes_cli/goals.py:1-7`, `hermes_cli/goals.py:343-349`). Epsilon has none: the caller that
records completion is authoritative, which is why a gate is exactly that caller's evidence
(`crates/epsilon-contracts/src/goal.rs:59-63`). A judge is a second opinion from the same class
of system; a command that exits zero or does not is not.

**The continuation prompt kept its two sentences, not just its shape.** Epsilon reproduces the
Hermes template and explains each addition: *"then re-run it to confirm"* — a round that fixes
the cause without proving the gate passes has not finished — and *"If the gate itself is wrong or
cannot pass, say so clearly and stop"* — because without that the only exits are a human or the
round cap, and both are worse than a model saying so
(`crates/epsilon-runtime/src/lib.rs:1501-1511`).

**Sanitisation repairs, it does not replace.** `crates/epsilon-llm/src/sanitize.rs:1-18` names
Hermes' `message_sanitization.py` and draws the line explicitly: "The repairs are syntactic only.
Nothing here invents a missing field, reorders a call, or guesses at intent."

## What we skipped, and why

**Profiles, and the multiplexing design that goes with them.** One process serving many profiles,
with per-profile homes, per-profile secret scope and scope-bound reads, is the largest single idea
in the reference with no counterpart here (`references/hermes-agent-2026.9.24/AGENTS.md`, "One
process may serve many profiles"). Epsilon has one global directory and one workspace. The
nearest thing in this tree is a custom provider's `base_url_override`, which sits on its own seam
rather than being read from a profile (`crates/epsilon-llm/src/custom.rs:2`,
`crates/epsilon-llm/src/providers/mod.rs:90`).

**The auxiliary-model judge**, for the reason given under *What we adapted* above. It also costs
a second model call per round and adds a failure mode where the judge is unreachable — which is
why Hermes itself makes it fail open (`hermes_cli/goals.py:6`).

**Hermes' gateway.** About twenty messaging platforms under `gateway/platforms/`, each an adapter,
plus DM pairing and per-platform auth. Epsilon has four modes — terminal IDE, headless,
HTTP/SSE, stdio-RPC — and none of them is a chat channel somebody can message the agent
through.

**The plugin system, the skills hub, and skill self-improvement.** `skills/` plus
`plugins/memory/`, `plugins/context_engine/`, `plugin-catalog/`, `optional-skills/`, and the
curator in `agent/curator.py`. Epsilon has a skill *reader*
(`crates/epsilon-tools/src/skill.rs`, `crates/epsilon-core/src/skills.rs`) against the shared
Agents Skills format, and no curator, no hub, no in-process plugin loader. A skill that rewrites
itself mid-run is a caching and review problem Epsilon has not taken on; it ships
`crates/epsilon-core/assets/agent-contract.md` instead and relies on the skill being reviewable in
a diff.

**Seven terminal backends.** local, Docker, SSH, Singularity, Modal, Daytona and Vercel Sandbox
(`README.md:29`, `tools/environments/`). Epsilon runs commands on the host through
`crates/epsilon-safety/src/sandbox.rs`, with job-object limits on Windows and bwrap on Linux
(ADR-005). Remote execution environments are a hosting product decision, and Epsilon is a local
harness.

**The cron scheduler and the state database.** `cron/`, and `hermes_state*.py` — the facade plus
29 topical siblings (`agent/AGENTS.md`, "Facade + siblings layout"). Epsilon's durable state is one
append-only JSONL file per session (ADR-006) plus checkpoints under the workspace. That is a
smaller surface with a different recovery story: a session file can be read and repaired by a
person, a SQLite schema needs a tool.

**The TUI, the Electron desktop and the web dashboard.** `ui-tui/`, `apps/desktop/`, `web/`.
Epsilon's second front end is a native egui/eframe window (ADR-020), which is the same refusal
of a webview that applies to DeepSeek Harness' desktop.

**The compat layer.** `COMPAT_MANIFEST.md`, `compat_manifest.json` (289 KB) and the
`PLUGIN-COMPAT` blocks exist to keep old import paths alive for external plugins, and Hermes'
own guide calls them "OFF LIMITS in-tree". Epsilon did not need one: nothing imports from these
references at runtime, so there is no import path to keep.

**Text FTS search over past sessions.** Hermes runs FTS5 with LLM summarization for
cross-session recall (`README.md:26`). Epsilon's session search is a single sequential scan of
the JSONL store with the results capped, and it keeps no index that outlives the file it was
built from (`crates/epsilon-session/src/search.rs:1-13`).

## Licence compatibility

**Compatible. Hermes is MIT, Epsilon is MIT, and adaptation with attribution is permitted.**

- Hermes: MIT, "Copyright (c) 2025 Nous Research" (`LICENSE`; `package.json:32`).
- Epsilon: MIT, "Copyright (c) 2026 Mohamed" ([`LICENSE`](../../LICENSE)).
- The MIT permission notice requires one thing of anyone who copies: "The above copyright notice
  and this permission notice shall be included in all copies or substantial portions of the
  Software."

Concretely, that would mean two obligations in this repository: a `THIRD_PARTY_NOTICES` file
carrying Hermes' notice, and a header on every adapted file naming its source — which is what
[`AGENTS.md`](../../AGENTS.md) §10 requires of an adaptation. The largest candidate would have
been `tools/checkpoint_manager.py`, 63 KB of shadow-git store management.

**What actually happened is different, and the difference is deliberate.** No third-party source
code was copied into this repository; every item above is a reimplementation from Hermes' own
written behaviour, and the divergences above are divergences of design, not of authorship.
[`README credits`](../../README.md#credits) records this: "**no third-party source code was copied into this
repository**: what was taken is behaviour, structure and design decisions" — which is why there
is no `THIRD_PARTY_NOTICES` file. The cost is that a citation must be checked by hand, so every
item above carries the reference file and line where the behaviour is stated.

### A note on citation drift

The port sites in `crates/` cite Hermes paths that name version `2026.9.14`, while the copy in
`references/` is `2026.9.24`. The paths still resolve, and the two most load-bearing citations
still land on the same code — `hermes_cli/goals.py:97-111` is still
`CONTINUATION_PROMPT_GATE_FAILED_TEMPLATE`, and `hermes_cli/goals.py:337-368` is still the
`GoalGate` dataclass. Some line numbers do not survive the version move: the vendored
`agent/verification_evidence.py` is 492 lines, so the `:731` cited at
`crates/epsilon-core/src/verify.rs:1` is out of range, and
`agent/verification_stop.py` is 191 lines, so `:233` is too. Both files exist and both describe
what the citation says they describe; the line numbers are stale, not the references.

The name, logo and brand of Hermes Agent belong to its authors and are not used anywhere in this
repository.