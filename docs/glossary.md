# Glossary

Every project has words that mean something specific inside it. This file defines
Epsilon's words. Each entry says what the term means here, in plain language, and
where the idea lives in the code. Read this alongside
[`docs/qa/06-agent-harness-primer.md`](qa/06-agent-harness-primer.md), which explains
the bigger picture for someone new to coding agents.

Entries are grouped by theme. Within a group they are alphabetical.

---

## The agent loop

**Agent loop** — The cycle that makes an LLM useful: send the conversation to the
model, stream back its reply, run any tool calls it asked for, append the results,
and repeat until the model stops asking for tools. Epsilon's loop lives in
`epsilon-core`. See [`design/epsilon-architecture.md`](../design/epsilon-architecture.md).

**Two-level loop** — pi's loop shape, ported verbatim. The *inner* loop keeps running
while tool calls remain or steering messages are queued. The *outer* loop drains
follow-up messages when the agent would otherwise stop, so queued work still happens.
Named seams like this are improved deliberately, never by accident (ADR-002).

**Steering queue** — Messages the user types *while* the agent is busy. They join the
current run instead of waiting for it to end, so you can correct course mid-task.

**Follow-up queue** — Messages typed when the agent is idle-ish (between turns). They
start new turns after the current one finishes cleanly.

**Turn** — One trip through the loop: prompt in, assistant reply out, tool calls
executed, results appended. Sessions are made of turns.

**Stop reason** — Why a turn ended: completed, aborted by the user, hit the token
limit, errored, or a tool asked to stop. "Sticky" stop reasons (a DSH idea) survive
across the rest of the batch so transcripts replay exactly.

## Talking to models

**StreamFn** — The trait every LLM backend implements. Its contract: it *never*
throws. Even panics are converted (by a `PanicGuard`) into a normal terminal event.
Failures arrive in-band as a `Done { stop_reason }`, so the loop has exactly one
error shape to handle. Ported from pi.

**Provider wire** — The HTTP protocol a model backend speaks. Epsilon implements
three directly (no SDK crate in between): Anthropic Messages, OpenAI-compatible
`/chat/completions`, and Google `streamGenerateContent`. Routing reads each catalog
row's declared `wire` field, never guesses from the model id string.

**Catalog** — The bundled table of known models (`epsilon-llm/src/catalog/`),
generated from [models.dev](https://models.dev) by `scripts/regen_catalog.py`. Each
row carries context window, prices, tool support, and the wire type. Browsing models
needs no network and no API key.

**Custom provider** — An endpoint you declare yourself in
`~/.config/epsilon/config.toml` under `[provider_custom.<name>]` (a `base_url`
plus optional `env_key`). Model ids like `lmstudio/qwen2.5-7b-instruct` then
stream to your server on the normal OpenAI-compatible wire. How Epsilon talks
to LM Studio, vLLM, or a provider the bundled catalog does not know yet.
Bundled catalog ids always win over same-named custom rows.

**Faux provider** — A scripted fake LLM used by tests, demos, and offline runs. It
replays deterministic steps, which makes full-loop integration tests instant, free,
and reproducible in CI. pi tests its loop the same way.

**Thinking level** — How hard the model is told to reason before answering
(off through max). One shared ladder in `epsilon-contracts`; each wire maps it to its
native option (Anthropic adaptive thinking, OpenAI `reasoning_effort`, Google
`thinkingConfig`). Carried per session, cycled with Ctrl+T or `/think`.

**Fallback chain** — An ordered list of models (`--fallback <model>`, repeatable).
When the primary fails with a retryable error (rate limit, transient 5xx, network),
the next candidate gets the turn. Auth failures do not walk the chain. Every new turn
starts at the head again, so a recovered primary is reused automatically. Pattern
from Hermes Agent.

**Retry policy** — Automatic re-sends of failed requests with backoff, tracked in a
durable retry ledger (ADR-010). Distinct from fallback: retry tries the *same* model
again; fallback moves to a *different* model.

**Prompt cache** — Providers charge less and answer faster when consecutive requests
share an identical prefix. Epsilon treats preserving that prefix as an invariant:
prompt tiers are byte-stable, auxiliary calls (like compaction summaries) never touch
the live prefix, and hash tests pin the bytes. This is the core pi thesis: tiny
stable prompts make caches hit, and hits are what make agents cheap and fast.

**Byte-stable prompt tiers** — The prompt is assembled in three fixed layers (system
instructions, then stable history, then volatile tail). Identical inputs produce
byte-identical prompts, verified by tests. Any byte difference breaks the provider's
cache.

## Tools and safety

**Tool call** — A structured request from the model to run a capability: read a file,
edit it, run a shell command, search with grep. Results go back as tool result
messages the model reads next turn.

**Fail-all-on-truncation** — If output hits the token limit mid-message, every tool
call in that message fails with a re-issue hint. Truncated JSON arguments look
plausible and execute dangerously wrong, so none of them run (pi's rule).

**Doom-loop brake** — Three identical tool calls in a row raise a permission ask
instead of burning tokens. Counted across batches within one run.

**Bounded parallel pool** — Independent tool calls run concurrently, capped by a
semaphore (8 at a time). Sequential tools act as barriers. Results commit in the
order the model emitted them, so transcripts replay identically.

**Permission ruleset** — Declarative allow/ask/deny rules matched against each tool
call, last match wins, wildcards supported. Ported from opencode. Deny beats stored
memory; a deny can never be talked out of.

**Sandbox (confinement)** — The OS-level wall around shell commands. On Windows:
Job Objects with memory, process-count, and CPU caps plus kill-on-close. On Unix:
bubblewrap. Fail-closed means: if the sandbox cannot be set up, the command refuses
to run rather than running free (DSH's pattern).

**Plan mode** — A read-only run (`--plan`). The permission engine denies every write
shaped tool up front (monotonic-deny boundary), so the model can explore and propose
but never mutate anything.

**Trust gate** — Opening a workspace whose `.epsilon/config.toml` changed (or is new)
pauses the TUI until you answer `trust` or `trust no`. The config is pinned by a
content hash, so silent edits cannot smuggle new behavior past you.

**Auth store** — `epsilon auth login <provider>` writes your key once to
`~/.config/epsilon/auth.json` (opencode's schema). Stored keys win over environment
variables; both work. Nothing else in Epsilon stores credentials.

## Sessions and memory

**Session** — The persistent record of one conversation, stored as JSONL under
`<workspace>/.epsilon/sessions/`. Sessions belong to the workspace, not to a global
folder.

**JSONL tree** — The session format: append-only lines forming a tree (branches are
kept, never overwritten). Append-only means crashes corrupt nothing and replays are
exact (ADR-006).

**Compaction** — When a session passes the token threshold, old message ranges are
pruned from what the *model* sees, the tail is clamped, and a summary amends prior
summaries. Long sessions stay within context without losing the transcript.

**Shadow view** — What compaction actually produces: the human-readable JSONL keeps
every original line, while the model sees the compacted projection. Audit intact;
context small.

**Turn lease** — An advisory lock file letting the CLI, TUI, and server share one
session file safely. A crashed process's leftovers can be taken over after 30
seconds. Cross-process coordination pattern from Hermes.

**Crash repair** — On load, dangling tool calls become synthetic error results and
failure-stopped turns get explicit markers. Idempotent: repairing twice changes
nothing.

## Extending

**Skill** — A folder with a `SKILL.md` file teaching the agent a procedure. The
catalog (name plus description) sits in the prompt; the body loads only when needed.
Follows the Agent Skills spec (ADR-015).

**Extension** — A separate OS process that registers extra tools, launched from a
manifest. It speaks NDJSON-RPC (newline-delimited JSON) over stdio and receives
filtered agent events. Isolation means a crashing extension cannot take the agent
down (ADR-009).

**MCP (Model Context Protocol)** — A standard protocol for exposing tools to agents.
Epsilon bridges MCP servers over stdio using the official rmcp SDK, so their tools
appear in the normal registry (ADR-013).

**Health probe** — Before calling an MCP tool, the bridge checks the connector is
alive (at most once per 30 seconds, TTL-cached). A dead server fails fast with an
honest error instead of hanging mid-call. Pattern from Hermes.

## Frontends

**TUI** — The terminal UI (ratatui): live transcript, approval dialogs, model picker,
command palette. Plain `epsilon` launches it.

**Print mode** — Headless one-shot runs (`-p "prompt"`): no UI, reply to stdout.
What scripts and CI use.

**Serve mode** — An HTTP server (`--serve`) streaming events over SSE, with REST
endpoints for prompt, steer, and approve. Guarded by a bearer token and Host check.

**RPC mode** — NDJSON-over-stdio for embedding Epsilon in editors.

**Model picker** — The `/models` dialog: stage one lists providers, stage two lists
that provider's models with fuzzy filtering. Favorites (Ctrl+F to star, F2 /
Shift+F2 to cycle) and recents sit above the full list. Two-stage flow and favorites
from opencode; cycling semantics from pi.

**Kill ring** — The editor's cut/copy memory (Ctrl+W kills a word, Ctrl+Y yanks it
back, Alt+Y cycles through saved kills). Emacs behavior, ported in M16b.

**Paste marker** — Pasting a large block into the editor inserts a short marker
(`[paste #1 +40 lines]`) that expands to the real text only on submit, keeping the
editor usable.

## How we build

**Brick** — One self-contained crate built and tested before the next one depends on
it. The lego-brick methodology: contracts first, bricks second, integration last
([`docs/qa/02-lego-brick-methodology.md`](qa/02-lego-brick-methodology.md)).

**Contracts crate** — `epsilon-contracts`: shared types and traits only. Zero logic,
zero internal dependencies. Every other crate speaks its vocabulary, which is what
makes boundaries compile-enforced rather than review-enforced.

**ADR (architecture decision record)** — A numbered entry in
[`design/epsilon-decisions.md`](../design/epsilon-decisions.md) recording a decision,
its alternatives, and why it won. Behavior-level changes require one.

**Dev log** — One markdown file per milestone under [`docs/milestones/`](milestones/):
what shipped, which decisions were made, which bugs the tests caught, and the raw
verification output.

**Wire fixture** — Recorded provider HTTP exchanges replayed as tests, so wire-parsing
code is exercised against real response shapes without network access.

**SlowProvider** — A test double that makes time pass deterministically, used instead
of sleeps so timing-sensitive tests never flake.
