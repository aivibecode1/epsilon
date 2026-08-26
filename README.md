# Epsilon

<p align="center">
  <img src="assets/Logo.png" alt="Epsilon logo" width="800">
</p>

A coding-agent harness in Rust. You type a prompt, the model reads your files, edits
them, runs commands, and keeps going until the job is done, with every tool call
passing through permission rules and an OS sandbox first.

We built it by reading four open-source harnesses at source level and porting the
best of each. No code was copied. Every borrowed idea is cited where it appears.

| We took | From | What came over |
|---|---|---|
| The skeleton | **[pi](https://github.com/earendil-works/pi)** (Earendil Works) | Two-level loop, steering/follow-up queues, JSONL session trees, never-fail provider contract |
| The armor | **DeepSeek Harness (DSH)** | Fail-closed OS confinement, bounded tool pools, sticky turn reasons |
| The discipline | **Hermes Agent** ([Nous Research](https://github.com/NousResearch/hermes-agent)) | Byte-stable prompt tiers, replay fidelity, cross-process leases |
| The manners | **opencode** ([Anomaly](https://github.com/anomalyco/opencode)) | Wildcard permission gates, auth store, picker favorites, plan mode |

Full credit table with links and file-level detail lives at the bottom:
[Credits and references](#credits-and-references).

The result is a single static binary running pi's loop under DSH's safety rules. We
could not find another harness doing both, so we built one.

---

## What it does

Epsilon reads, writes, edits, greps, globs, lists, and runs shell commands inside its
declared workspace. Every call passes two gates in order: permission rules (last
match wins), then OS confinement. On Windows that is Job Objects with memory,
process-count, and CPU caps plus kill-on-close. On Unix it is bubblewrap. If the
sandbox cannot start, the call fails closed. It never runs free.

Long sessions auto-compact past a token threshold: old ranges get pruned from the
model's view while the transcript on disk keeps every line. Skills load from
`SKILL.md` directories on demand. Extensions run as separate processes. MCP servers
attach over stdio.

Three frontends share one runtime:

```sh
epsilon                                  # ratatui TUI (default), live events, approvals, model picker
epsilon -p "summarize this repo"         # print mode, headless, reply to stdout
epsilon --serve --port 8787              # HTTP/SSE server for editors and scripts
epsilon --rpc                            # NDJSON over stdio for embedding
```

Model selection reads a bundled catalog of 386 models (generated from
[models.dev](https://models.dev), with real prices), so browsing needs no network and
no API key. Keys come from `epsilon auth login` or environment variables. Talking to
real providers requires a build with `--features real`; everything else runs against
a scripted offline provider.

---

## Quick start

You need Rust 1.88 or newer. Install it from [rustup.rs](https://rustup.rs) if you
do not have it.

```sh
git clone <this-repo> epsilon && cd epsilon
cargo build --workspace          # compiles all 11 crates
cargo test --workspace           # full suite, offline and free
cargo run -p epsilon-cli -- --demo   # scripted demo, proves the loop works
```

The demo output looks like this:

```
=== run start ===
-- turn 0 --
assistant> I'll inspect the two files, then write the fix.
tool-call> echo
tool-call> echo
-- turn 1 --
assistant>
tool-call> fail
-- turn 2 --
assistant> One check failed; here is the summary of what I found.
=== run end ===

final reason: Completed
messages produced: 6
tokens in/out: 300/150
```

Builds, tests, and demos never touch a paid API. They run against the **faux
provider**, a deterministic scripted LLM, which is also how pi tests its own loop.
CI stays free and instant.

More ways to drive it once it builds:

```sh
cargo run -p epsilon-cli -- --tui                # interactive TUI explicitly
cargo run -p epsilon-cli -- --list-models        # show the bundled catalog
cargo run -p epsilon-cli -- --continue -p "next step"   # resume newest session
```

### Real providers (optional)

```sh
cargo build --release -p epsilon-cli --features real   # enables direct HTTP wires
epsilon auth login anthropic                           # stores a key once
target/release/epsilon                                 # finds it and runs REAL
```

`auth login` writes `~/.config/epsilon/auth.json` (opencode's schema). A stored key
wins over an environment variable; both work. Environment equivalents:
`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, and friends. To skip the
picker entirely:

```sh
target/release/epsilon --provider real --model claude-sonnet-4-5 -p "summarize this repo"
```

Providers speak their native wire protocol over HTTPS: Anthropic Messages,
OpenAI-compatible `/chat/completions`, Google `streamGenerateContent`. There is no
SDK crate in between. Routing reads each catalog row's declared wire type rather than
guessing from the model id. Retryable failures walk fallback chains given with
repeatable `--fallback <model>` flags; auth failures stop the chain.

---

## Architecture

One turn through the system:

```
prompt ──► Agent.run()
             │  drain steering queue (messages typed while busy)
             ▼
        convert_to_llm(history)          drop harness-only messages
             ▼
        StreamFn.stream(model, ctx)      NEVER fails; failures arrive in-band
             │  StreamEvent*             text/thinking/toolcall deltas
             ▼
        AssistantMessage (Done = authoritative final)
             │  stop_reason == MaxTokens?
        fail ALL tool calls              truncated args are unsafe (pi rule)
             ▼  otherwise
        execute_batch:
          preflight sequentially: resolve → validate → gate.ask()?
          doom-loop brake: ≥3 identical calls → ask
          parallel-safe calls → Semaphore(8)-bounded pool
          sequential tools    → barrier; results commit in MODEL order
             ▼
        ToolResultMessage* appended      transcript persists to .epsilon/sessions/*.jsonl
             │                           (append-only tree; compaction shadows ranges)
             └── loop while tools remain || steering queued
                  then drain follow-ups, else emit AgentEnd
```

### The eleven crates

Dependency direction is compile-enforced. Nothing depends upward.

```
contracts ← llm ← core ← {tools, safety, ext, mcp} ← {tui, server} ← cli
contracts ← session ← core
```

| Crate | Role | Key types |
|---|---|---|
| `epsilon-contracts` | Shared vocabulary: types and traits only, zero logic | `Message`, `StreamEvent`, `AgentEvent`, `SessionEntry`, `Tool`, `StopReason`, `Usage` |
| `epsilon-llm` | Multi-provider layer behind the never-fail contract: three direct HTTP wires, faux provider, retry, fallback chains, catalog | `StreamFn`, `EventStream`, `FallbackProvider`, `ProviderWire` |
| `epsilon-session` | Append-only JSONL tree sessions | `SessionStore`, `SessionLease`, repair helpers |
| `epsilon-core` | The loop: queues, batch executor, event bus, prompts, compaction | `Agent`, `LoopLimits`, `ToolRegistry`, `PermissionGate` |
| `epsilon-tools` | Built-in tools: read/write/edit/bash/grep/glob/ls | typed tool structs |
| `epsilon-safety` | Permission rulesets and sandbox backends | wildcard engine, `Sandbox` trait |
| `epsilon-ext` | Process-isolated extension host | NDJSON-RPC protocol, manifests |
| `epsilon-mcp` | MCP client bridging external tools | rmcp-based registry bridge, health probes |
| `epsilon-tui` | Terminal frontend: live transcript, dialogs, picker, palette | `UiState` reducer, editor ops |
| `epsilon-server` | HTTP/SSE frontend | `ServeState`, router, bearer guard |
| `epsilon-cli` | Binary: mode dispatch and runtime composition | CLI |

### Key mechanisms

- **Two-level loop** (pi verbatim). The inner loop runs while tool calls remain or
  steering is pending. The outer loop drains follow-ups when the agent would
  otherwise stop. Each queue has an explicit drain mode.
- **Never-fail provider contract.** `StreamFn` implementations cannot throw. Even
  panics become a terminal `Done { stop_reason }` through a blanket `PanicGuard`.
  The loop handles exactly one error shape.
- **Fail-all-on-truncation.** When output hits the token limit, every tool call in
  that message fails with a re-issue hint. Truncated JSON arguments look plausible
  and execute dangerously wrong, so none of them run.
- **Doom-loop brake.** Three identical calls in a row raise a permission ask instead
  of burning tokens.
- **Bounded parallel pool.** Independent tools run concurrently up to a semaphore
  cap; sequential tools act as barriers; results commit in model order so transcripts
  replay byte-exactly.
- **Shadowed model view.** Compaction rewrites what the model sees. The JSONL on disk
  keeps every original line. Audit intact, context small.
- **Turn leases.** An advisory lock file lets CLI, TUI, and server share one session
  file. Crash leftovers can be taken over after 30 seconds.
- **Crash repair.** Dangling tool calls become synthetic error results on load;
  failure-stopped turns get explicit markers. Idempotent by construction.
- **Byte-stable prompt tiers.** Identical inputs produce byte-identical prompts,
  hash-tested, so the provider's prompt cache keeps hitting. This is the economic
  core of the whole design.

---

## Testing strategy

The faux provider makes full-loop integration tests offline, deterministic, and fast.
Time-sensitive scenarios use `SlowProvider` instead of sleeps, so they never flake on
a loaded machine. Each crate owns integration tests covering its failure paths, not
just the happy path. Loop scenarios assert exact transcripts and event sequences.
Provider wire parsing is pinned by fixture tests recorded from real HTTP exchanges.
Pure functions (permission evaluation especially) get property-style coverage.

Real-provider checks exist but sit behind env-var-gated `#[ignore]` so nothing in a
normal test run ever costs money.

---

## Status

244 tests offline (258 with `--features real`), zero clippy warnings in both build
configurations. CI runs all gates on Windows, a bubblewrap confinement suite on
Linux, and an MSRV 1.88 check. A weekly job
([`.github/workflows/catalog-drift.yml`](.github/workflows/catalog-drift.yml))
regenerates the bundled model catalog against live models.dev every Monday and
opens a pull request when it drifted.

Every milestone has a dev log under [`docs/milestones/`](docs/milestones) with what
shipped, which decisions were made, and which bugs the tests caught.

| Milestone | Scope |
|---|---|
| M0–M5 | Docs baseline, 11-crate workspace, contracts vocabulary, LLM layer behind StreamFn, JSONL sessions, agent loop with runnable demo |
| M6–M8 | Real tools + permission ruleset engine, compaction + byte-stable prompt tiers, skills + extension host + MCP bridge |
| M9–M10b | TUI + serve/print/rpc frontends, extension event forwarding, sandbox backends (Windows Job Objects, bwrap suite), release packaging, tag v0.1.0 |
| M11–M13 | Model picker + bundled catalog, project trust hashing, retry executor, cost display, resume, RPC mode, live catalog fetch, recents, markdown transcript, expandable tool cards |
| M14–M15 | Plain `epsilon` launches the TUI with key auto-detection; slash-command repairs; usability overhaul porting pi/opencode interaction patterns |
| M16a–c | Render cache + virtualized transcript (~30–69x faster draws), pi footer parity, `--tui-bench`; Emacs kill-ring, paste markers, multi-line editor; two-stage picker + command palette |
| M17 | pi-agent port: genai removed, direct per-provider HTTP (Anthropic/OpenAI-compat/Gemini), catalog regenerated from models.dev with real prices, keybind parity sweep |
| M18 | Docs audit, beginner primer, session search (`/search`), plan mode imported from opencode (`--plan` read-only runs) |
| M19 | All 8 improvement plans executed: stop-reason fidelity on all wires, loop integrity fixes, serve-mode bearer auth, secret-leak closure, wire fixtures in CI, byte-safe SSE buffer |
| M20 | Catalog regeneration pipeline (`scripts/regen_catalog.py`, 383 models.dev rows), live `GET /models` fetch fixed and auto-firing on picker open |
| M21 | Thinking fidelity: shared ladder in contracts, native mapping per wire, `/think` + Ctrl+T, carried per session |
| M22 | opencode auth store (`epsilon auth login/list/logout`), trust re-prompt holds the TUI until answered |
| M23 | Picker favorites: Ctrl+F stars, favorites above recents, F2/Shift+F2 cycling (pi semantics) |
| M24 | MCP health probes (30s TTL, fail-fast honest errors), provider fallback chains (`--fallback`, repeatable, per-turn primary restoration) |
| M25 | Footer thinking-level indicator: `think:<level>` in the status bar (pi pattern), hidden while Off; mirrored at the `set_thinking` choke point so displayed state always equals executed state |
| M26 | Per-favorite thinking levels (pi scoped-models): Ctrl+F remembers the current level per starred model, F2/Shift+F2 apply it on cycle, unremembered favorites inherit |
| M27 | Weekly catalog-drift CI job: regenerates the bundled catalog against live models.dev every Monday and opens a refresh PR when rows changed (never pushes main) |
| future | — | — |

---

## Design decisions digest

Full reasoning lives in [`design/epsilon-decisions.md`](design/epsilon-decisions.md).
One line each:

| ADR | Decision |
|---|---|
| 001 | Rust on tokio (LTS channel), single binary |
| 002 | Port pi's loop shape verbatim; improve only at named seams |
| 003 | Provider layer: pi-style direct per-provider HTTP behind our own `StreamFn`; wire types never leak (M17 superseded the original genai wrapper) |
| 004 | Tools defined as typed structs; schemars-derived JSON Schema; `.txt` descriptions |
| 005 | Two-layer safety: permission gates over fail-closed confinement; honest per-OS limits |
| 006 | Sessions = append-only JSONL trees + offset index + advisory-lock turn leases |
| 007 | Compaction: prune → clamp tail → strict template → amend prior summaries → shadow view |
| 008 | Three-tier byte-stable prompt assembly; volatility guarded by hash tests |
| 009 | Third-party extensions run as isolated OS processes speaking NDJSON-RPC |
| 010 | Reliability stack: durable retry ledger, crash repair, kill ladders, integer-micro costs |
| 011 | Frontends: ratatui default; print/rpc/serve share one runtime |
| 012 | TOML configuration, declarative only; project trust with content-hash pinning |
| 013 | MCP via official rmcp SDK bridged into the normal tool registry |
| 014 | Eleven crates mirroring pi's seams, with a dedicated contracts crate |
| 015 | Agent-Skills-spec compliance: catalog in prompt, body loaded on demand |
| 016 | Supply-chain policy inherited from pi: exact pins, advisories in CI |
| 017 | Build order + debug tooling per brick; contracts freeze policy |
| 018 | Model selection: TUI picker over bundled generated catalog; flag > saved config > default precedence; auth detection only in the picker (credentials via the M22 auth store) |

---

## How to read this repository

New to coding agents entirely? Start with
[`docs/qa/06-agent-harness-primer.md`](docs/qa/06-agent-harness-primer.md): what a
harness even is, in plain language, no background assumed. Then keep
[`docs/glossary.md`](docs/glossary.md) open while you read; it defines every term
this project uses, from StreamFn to shadow view.

Joining as a developer? Read this README, then [`CHANGELOG.md`](CHANGELOG.md) for
what shipped when, then the short notes in [`docs/qa/`](docs/qa): why Rust, the lego
methodology, partial vs full ports, documentation organization. The developer
contract is root [`AGENTS.md`](AGENTS.md), which loads automatically for coding
agents. [`design/epsilon-architecture.md`](design/epsilon-architecture.md) sections 2
through 7 cover components and data flow, and
[`design/epsilon-skills.md`](design/epsilon-skills.md) inventories the skill library.
[`docs/milestones/`](docs/milestones) has the dev logs in order. Session-to-session
work orders live in [`docs/handoff/`](docs/handoff).

Researching harness design? [`reports/harness-analysis.md`](reports/harness-analysis.md)
audits pi, DSH, Hermes, and opencode at source level with file-anchored strengths and
weaknesses. [`reports/comparative-analysis.md`](reports/comparative-analysis.md) ranks
who does what best. The remaining reports in [`reports/`](reports) record alignment
passes and re-audits against those references.

Working in this repo as a coding agent? Root [`AGENTS.md`](AGENTS.md) again. The
latest entry in [`docs/milestones/`](docs/milestones) tells you the current state.
The runtime behavior contract Epsilon ships lives at
[`crates/epsilon-core/assets/agent-contract.md`](crates/epsilon-core/assets/agent-contract.md);
it describes the product, not this repo's development rules.

---

## Contributing

Every change must pass:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets   # zero warnings tolerated
cargo test --workspace                   # full suite green
```

House rules: dependencies are pinned exactly (`=x.y.z`) after checking feature names
against the crate's own manifest. No logic in `contracts`. Architectural violations
should be build errors, not review comments. Behavior changes need an ADR note.
Commits look like `feat(mN): ...`, `fix(mN): ...`, or `docs: ...`.

---

## Roadmap acceptance criteria

Original acceptance criteria, kept for the record:

| Milestone | Done when |
|---|---|
| M6 | Model reads/edits real files and runs gated bash here; rulesets evaluated last-match-wins with ask/deny/allow; every gate path tested |
| M7 | Long sessions auto-compact past 80% context; prompts byte-stable across identical inputs (hash-tested); summaries amend prior ones |
| M8 | A SKILL.md dropped into a directory appears in catalog and loads on demand; an extension process registers a tool; MCP server tools callable |
| M9 | Interactive TUI renders live events; `-p` completes headlessly; serve streams SSE; an extension receives filtered events over stdio |
| M10 | Linux confinement verified (bwrap); Windows bash children run inside a Job Object; ACL runner shipped or explicitly deferred with rationale; release binary <15MB, changelog + tag |

All five are closed. M10's ACL runner was deferred with rationale (ADR-005).

---

## Credits and references

Epsilon is a learning project built by reading four open-source coding agents and
reimplementing the best of what we found. No code was copied; every idea is cited
where it appears in this repo, and the Rust implementation is written from scratch.
The reference snapshots we read live in `references/` (pinned versions, unmodified).

| Harness | Source | What Epsilon took | Where it landed |
|---|---|---|---|
| **pi** (v0.84.2) by Mario Zechner / Earendil Works | [github.com/earendil-works/pi](https://github.com/earendil-works/pi) · [pi.dev](https://pi.dev) | The skeleton: two-level agent loop, steering and follow-up queues, never-fail `StreamFn` contract, fail-all-on-truncation, JSONL session trees with compaction shadow views, turn leases, model picker and cycling semantics, fuzzy matcher, thinking levels carried per session | `epsilon-core`, `epsilon-llm`, `epsilon-session`, `epsilon-tui` throughout |
| **DeepSeek Harness "DSH"** (v0.1.1-rc.2) by DeepSeek | snapshot in `references/` (no public repo URL verified at time of writing) | The armor: fail-closed OS confinement seam (Job Objects on Windows, bubblewrap on Unix), bounded tool pools, sticky turn reasons, plugin-style process isolation for extensions | `epsilon-safety`, `epsilon-ext`, loop executor in `epsilon-core` |
| **Hermes Agent** (2026.8.19) by Nous Research | [github.com/NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent) | The discipline: byte-stable three-tier prompt assembly, replay fidelity via request hashing, cross-process turn leases, TTL health probes for optional tools (M24), error-classified fallback chains with per-turn primary restoration (M24), incident-driven documentation culture | `epsilon-core` prompts, `epsilon-session` leases, `epsilon-mcp` probes, `epsilon-llm/src/fallback.rs`, `docs/qa/` |
| **opencode** (v1.18.21) by Anomaly (formerly SST) | [github.com/anomalyco/opencode](https://github.com/anomalyco/opencode) | The manners: wildcard permission rulesets with last-match-wins ask/deny/allow, auth store with stored-key-wins precedence, picker favorites and two-stage selection, doom-loop brake, plan mode, structured compaction UX | `epsilon-safety` rulesets, `epsilon-llm/src/auth.rs`, `epsilon-tui` picker, plan mode in `epsilon-core` |

Two more debts worth naming: the [models.dev](https://models.dev) catalog feeds our
bundled model data through `scripts/regen_catalog.py`, and this documentation follows
Wikipedia's "Signs of AI writing" guidance via the humanizer checklist. Full
source-level audits behind these choices:
[`reports/harness-analysis.md`](reports/harness-analysis.md) and
[`reports/comparative-analysis.md`](reports/comparative-analysis.md).

---

*This repo is a research-through-implementation project. The reference harnesses
under `references/` belong to their authors; we cite their lessons and don't copy
their code.*
