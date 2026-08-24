# Epsilon

**A coding-agent harness written in Rust** — a faithful port of [pi](https://pi.dev)'s proven
agent loop, session format, and tool pipeline, selectively hardened with the best ideas from
three other production harnesses we audited at source level:

| Inherits | From | What it adds |
|---|---|---|
| The skeleton | **pi** (Earendil Works) | Two-level loop, steering/follow-up queues, JSONL session trees, never-fail provider contract |
| The armor | **DeepSeek Harness (DSH)** | Fail-closed OS confinement seam, bounded tool pools, sticky turn reasons |
| The discipline | **Hermes Agent** | Byte-stable prompt tiers, replay fidelity, cross-process leases |
| The manners | **opencode** | Wildcard permission gates, doom-loop brake, structured compaction |

Result: *one static binary* with pi-grade simplicity **and** DSH-grade safety — a combination no
existing harness offers.

---

## Status

**Real tools, ruleset-gated, long-session capable, extensible, and fronted.** Epsilon reads,
writes, edits, greps, globs, lists, and runs shell commands inside its declared workspace — every
call passing through last-match-wins permission rules AND OS confinement (Windows Job Objects
with memory/process/CPU caps and kill-on-close; Unix bubblewrap; fail-closed when unavailable).
Long sessions auto-compact past the token threshold (prune, clamp tail, amend summaries) with
byte-stable prompt tiers. Skills load from `SKILL.md` directories; third-party extensions run as
isolated processes launched from manifests; MCP servers attach over the official rmcp SDK's stdio
transport. Three frontends share one runtime: an interactive ratatui TUI (`--tui`, live events +
approval dialogs + a pi-style `/models` picker), print mode (`-p`, headless), and serve mode
(`--serve`, HTTP/SSE for remote clients). Model selection uses a bundled catalog with env-var
auth detection; real providers plug in behind `--features real` + `--provider real`. All ten
planned milestones shipped; deferred items carry written rationale in the ADRs and milestone
ledgers.

| Milestone | Scope | State |
|---|---|---|
| M0 | Docs baseline, git, build-order ADR | ✅ shipped |
| M1 | 11-crate workspace, quality gates green from commit one | ✅ shipped |
| M2 | `epsilon-contracts` — pi's type vocabulary ported | ✅ shipped |
| M3 | `epsilon-llm` — StreamFn contract, faux provider, provider adapter, retry (M17: adapter is now pi-style direct HTTP) | ✅ shipped |
| M4 | `epsilon-session` — JSONL tree store, shadow views, leases, crash repair | ✅ shipped |
| M5 | `epsilon-core` — full agent loop + runnable demo | ✅ shipped |
| M6 | Real tools (read/write/edit/bash/grep/glob/ls) + permission ruleset engine | ✅ shipped |
| M7 | Compaction engine + three-tier byte-stable prompt builder | ✅ shipped |
| M8 | Skills, extension host (process-isolated), MCP bridge | ✅ shipped |
| M9 | TUI + HTTP/SSE frontends + rmcp stdio transport | ✅ shipped |
| M9a | Extension event forwarding (closes known-gap #1) + print mode -p | ✅ shipped |
| M9b | ratatui TUI (`--tui`), serve mode SSE (`--serve`), rmcp stdio transport | ✅ shipped |
| M10 | Hardening pass: sandbox backends incl. **Windows Job Objects + ACL runner** (closes known-gap #2, M8 ledger), release packaging | ✅ shipped |
| M10a | Sandbox seam (Job Objects / bwrap, fail-closed) + permission-event correlation fix; **ACL runner deferred with rationale** (ADR-005) | ✅ shipped |
| M10b | Release packaging: real providers, model plumbing, 8.8 MB binary, changelog + tag `v0.1.0` | ✅ shipped |
| M11 | Model picker + bundled catalog (`/models`, `--list-models`), config slice, skills/extensions runtime wiring | ✅ shipped |
| M12 | Completion pass: project config + trust hash, retry executor + run cancellation + cost display, --resume, --rpc mode, OpenAPI endpoint | ✅ shipped |
| M13 | Polish: live catalog fetch (Ctrl+F in /models), recents, markdown-lite transcript, expandable tool cards, bwrap test suite for Linux CI | ✅ shipped |
| M14 | Harness maturity: plain `epsilon` launches the TUI by default (`--demo` preserved), provider auto-detect from env keys with faux fallback + banner, slash-command repairs on the real key path (`/clear` `/skills` `/quit`, unknown-command errors) | ✅ shipped |
| M15 | TUI usability overhaul porting pi/opencode patterns — picker scroll window (fixes unreachable rows), unicode editor ops, pulldown-cmark transcript, spinner+elapsed, hint bar, mouse wheel, bracketed paste, slash autocomplete; 146 tests | ✅ shipped |
| M16a | TUI rendering efficiency: id-keyed render cache (markdown+wrap once per line), virtualized transcript (~30–69× faster draws, near-flat vs session length), cached picker rows, pi footer parity (cwd/branch/compact tokens/context %/cache-hit rate), `--tui-bench` mode | ✅ shipped |
| M16b | Editor v2: Emacs kill-ring (Ctrl+U/W/Y with accumulation + yank-pop), atomic paste markers (`[paste #N +L lines]`, expand on submit, marker-aware Ctrl+W), multi-line buffer (Ctrl+J newline), persistent prompt history (500 entries) | ✅ shipped |
| M16c | Picker/palette parity: two-stage model picker (providers → models, fuzzy-filtered), command palette (Ctrl+P) dispatching all commands, draft stash across dialogs, pi fuzzy matcher; 187 tests | ✅ shipped |
| M17 | **pi-agent port**: direct per-provider HTTP replaces genai (Anthropic/OpenAI-compat/Gemini wires), catalog v2 regenerated from live models.dev (current 2026 generations, REAL prices), slash autocomplete with arrow-key selection + descriptions (pi pattern), keybind parity sweep (Ctrl+A/E/B/F/D/K, Alt+B/F/D, Ctrl+. redo, F2 recents); full attribution in dev log | ✅ shipped |
| M18 | Alignment pass: docs audit (M14 log backfilled, beginner primer, stale genai/MSRV claims fixed), pi-alignment report + session search closed (`/search` command, 7 new tests), research pass 2 with community signal, **plan mode** imported from opencode (`--plan` read-only runs, monotonic-deny boundary, live-verified) | ✅ shipped |
| M19 | Audit remediation: all 8 improvement plans executed — provider stop-reason fidelity on all three wires, loop integrity (latch reset, steering requeue, UTF-8 safety), serve-mode bearer token + Host guard, deny-beats-memory + grep secret-file skip, wire fixture tests in CI, byte-safe SSE buffer, ext event forwarding wired, CLI `RunParams` composition cleanup | ✅ shipped |
| M20 | Catalog regeneration + live picker refresh: `scripts/regen_catalog.py` generates the catalog from models.dev (383 rows vs ~40 curated — DIRECTION-05 closed), live `GET /models` fetch FIXED (M17's Ctrl+F passed a model id where a provider name was required) and AUTO-FIRES on picker open / provider descent (opencode/Hermes convention), live ids merged into both picker stages | ✅ shipped |
| future | favorites beyond recents · interactive trust re-prompt · session repair wiring decision · weekly catalog-drift CI PR | open |

221 tests · zero clippy warnings (both feature configs) · every milestone committed
with a dev log under [`docs/milestones/`](docs/milestones).

---

## Why it exists

Four convictions, each traceable to evidence in our reference audit
([`reports/harness-analysis.md`](reports/harness-analysis.md)):

1. **Deterministic latency and real parallelism.** pi folds streaming tokens into a live UI;
   garbage-collector pauses work directly against that. Hermes' synchronous loop capped
   parallel tool execution structurally. Rust gives both for free.
2. **Safety needs two layers, not one.** opencode proved intent-level permission gates can be
   usable; DSH proved OS confinement can fail closed without dead-ending the agent.
   Every existing harness ships only one layer. Epsilon wires both together: a sandbox denial
   escalates into an approval prompt instead of failing the task.
3. **Prompt-cache preservation is an invariant, not an optimization.** Byte-stable prompt tiers,
   auxiliary calls isolated from the live cache prefix, replay fidelity via request hashing.
4. **Build like lego.** Contracts first, bricks second, integration last — so any bug reproduces
   by replaying a deterministic log instead of poking a live system. See
   [`docs/qa/02-lego-brick-methodology.md`](docs/qa/02-lego-brick-methodology.md).

---

## Quick start

Prerequisites: Rust ≥ 1.88 (edition 2024; the workspace declares `rust-version = 1.88`
and CI verifies it), any of stable-msvc/stable-gnu/musl targets.

```sh
git clone <this-repo> epsilon && cd epsilon
cargo build --workspace          # compile all 11 crates
cargo test --workspace           # 221 tests, offline & free
epsilon                          # interactive TUI (auto-detects your API keys)
cargo run -p epsilon-cli -- --demo   # scripted offline demo
cargo run -p epsilon-cli -- -p "summarize this repo"        # headless print mode
cargo run -p epsilon-cli -- --tui                           # interactive TUI
cargo run -p epsilon-cli -- --serve --port 8787             # HTTP/SSE server
cargo run -p epsilon-cli -- --list-models                   # bundled model catalog
cargo run -p epsilon-cli -- --continue -p "next step"   # resume newest session
cargo run -p epsilon-cli -- --rpc                           # NDJSON-over-stdio for editors
```

Demo output:

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

Everything runs against the **faux provider** — a scripted deterministic LLM — so builds,
tests, and demos are offline, instant, and cost nothing. This is deliberate: it is how pi tests
its own loop, and it means CI never touches paid APIs.

### Real providers (optional)

```sh
cargo build --release -p epsilon-cli --features real   # direct per-provider HTTP (pi-style)
export ANTHROPIC_API_KEY="..."               # or OPENAI_API_KEY / GEMINI_API_KEY; env only, never stored
target/release/epsilon                       # auto-detects your key and runs REAL
# or explicit:
target/release/epsilon --provider real --model claude-sonnet-4-5 -p "summarize this repo"
```

Providers speak their native wire protocol over HTTPS (Anthropic Messages,
OpenAI-compatible `/chat/completions`, Google `streamGenerateContent`) — no SDK crate
in between. Routing reads each model catalog row's declared wire type, never id
substrings. Keys are read from the environment only.

---

## Architecture

One turn through the system:

```
prompt ──► Agent.run()
             │  drain steering queue (messages typed while busy)
             │
             ▼
        convert_to_llm(history)          drop harness-only messages
             │
             ▼
        StreamFn.stream(model, ctx)      NEVER fails; failures arrive in-band
             │  StreamEvent*              text/thinking/toolcall deltas
             ▼
        AssistantMessage (Done = authoritative final)
             │
             ▼  stop_reason == MaxTokens?
        fail ALL tool calls              truncated args are unsafe (pi rule)
             │
             ▼  otherwise
        execute_batch:
          preflight sequentially: resolve → validate → gate.ask()?
          doom-loop brake: ≥3 identical calls → ask
          parallel-safe calls → Semaphore(8)-bounded pool
          sequential tools    → barrier; results commit in MODEL order
             │
             ▼
        ToolResultMessage* appended      transcript persists to .epsilon/sessions/*.jsonl
             │                            (append-only tree; compaction shadows ranges)
             └── loop while tools remain || steering queued
                  then drain follow-ups, else emit AgentEnd
```

### The eleven crates

Dependency direction is compile-enforced: nothing may depend upward.

```
contracts ← llm ← core ← {tools, safety, ext, mcp} ← {tui, server} ← cli
contracts ← session ← core
```

| Crate | Role | Key types | Pi analogue |
|---|---|---|---|
| `epsilon-contracts` | Shared vocabulary: types + traits, zero logic, zero internal deps | `Message`, `StreamEvent`, `AgentEvent`, `SessionEntry`, `Tool`, `StopReason`, `Usage` | `packages/agent/types.ts` |
| `epsilon-llm` | Multi-provider LLM layer behind the never-fail contract: direct per-provider HTTP wires (Anthropic / OpenAI-compat / Google), faux provider, retry policy, catalog | `StreamFn`, `EventStream`, `FauxProvider`, `ProviderWire`, catalog | `packages/ai` |
| `epsilon-session` | Append-only JSONL tree sessions | `SessionStore`, `SessionLease`, repair helpers | `harness/session` |
| `epsilon-core` | The loop: queues, batch executor, event bus | `Agent`, `LoopLimits`, `ToolRegistry`, `PermissionGate` | `packages/agent` |
| `epsilon-tools` | Built-in tools *(M6)* | read/write/edit/bash/grep… | `coding-agent/src/core/tools` |
| `epsilon-safety` | Permission rulesets + sandbox backends *(M6+)* | wildcard engine, `Sandbox` trait | none in pi |
| `epsilon-ext` | Process-isolated extension host *(M8)* | NDJSON-RPC protocol, manifests | `core/extensions` |
| `epsilon-mcp` | MCP client bridging external tools *(M8)* | rmcp-based registry bridge | none in pi core |
| `epsilon-tui` | ratatui terminal frontend *(M9)*: live event transcript, approval dialog, steering | `UiState` reducer, `TuiConfig` | `packages/tui` |
| `epsilon-server` | HTTP/SSE frontend API *(M9)*: SSE events, prompt/steer/approve REST | `ServeState`, `router` | RPC mode |
| `epsilon-cli` | Binary: modes dispatch (demo / print / tui / serve), runtime composition | — | CLI |

---

## Key mechanisms

- **Two-level loop** (pi verbatim): inner loop runs while tool calls remain or steering is
  pending; outer loop drains follow-ups when the agent would otherwise stop. Each queue has an
  explicit drain mode (`all` vs `one-at-a-time`).
- **Never-fail provider contract**: `StreamFn` implementations cannot throw; failures arrive as
  a terminal `Done { stop_reason: Error | Aborted }`. A blanket `PanicGuard` converts even
  panics into that shape.
- **Fail-all-on-truncation**: when output hits the token limit, every tool call in that message
  fails with a re-issue hint instead of executing plausibly-corrupt JSON arguments.
- **Doom-loop brake**: three consecutive identical calls (counted across batches, run-scoped)
  raise a permission ask instead of burning tokens.
- **Bounded parallel pool**: parallel-safe tools run concurrently up to a semaphore cap;
  sequential tools act as barriers; results always commit in model order so transcripts replay.
- **Shadowed model view**: compaction rewrites what the MODEL sees while the human JSONL keeps
  every line — audit intact, context small.
- **Turn leases**: an advisory lock file lets CLI/TUI/server coexist on one session file;
  crash leftovers are stealable after 30s.
- **Crash repair**: dangling tool calls become synthetic error results; failure-stopped turns
  get typed `TurnEnd{Aborted}` markers. Idempotent by construction.

## Testing strategy

The faux provider makes full-loop integration tests offline, deterministic, and millisecond-fast
(pi's own pattern). Time-sensitive scenarios use `SlowProvider`, never sleeps-and-hope. Each
brick carries its own integration tests; loop scenarios assert exact transcripts, event
sequences, and sticky reasons. Property-style coverage arrives where pure functions warrant it
(permission evaluation, M6).

---

## Design decisions digest

Full reasoning lives in [`design/epsilon-decisions.md`](design/epsilon-decisions.md); one line each:

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

---

## How to read this repository

Pick your path:

**You are new to coding agents entirely**
1. [`docs/qa/06-agent-harness-primer.md`](docs/qa/06-agent-harness-primer.md) — what an
   agent harness is, in plain language, with zero assumed background.
2. Then follow the developer path below.

**You are a developer joining the project**
1. This README.
2. [`CHANGELOG.md`](CHANGELOG.md) — what shipped when, milestone by milestone.
3. [`docs/qa/`](docs/qa) — short narrative notes (why Rust, the lego methodology, what
   exists after each phase, the finished-product vision, documentation conventions,
   and a beginner's primer on agent harnesses).
4. [`design/epsilon-architecture.md`](design/epsilon-architecture.md) §2–§7 — components and
   data flow.
5. [`docs/milestones/`](docs/milestones) — dev logs in order; each records what was built,
   which bugs the tests caught, and why decisions were made.

**You are researching agent-harness design**
1. [`reports/harness-analysis.md`](reports/harness-analysis.md) — source-level audit of pi /
   DSH / Hermes / opencode with file-anchored strengths and weaknesses.
2. [`reports/comparative-analysis.md`](reports/comparative-analysis.md) — who does what best,
   reuse list, anti-pattern list.
3. [`design/epsilon-decisions.md`](design/epsilon-decisions.md) — how the audit became choices.

**You are a coding agent working in this repository**
1. Root [`AGENTS.md`](AGENTS.md) — commands, gates, laws of this repo. It loads automatically.
2. [`design/epsilon-architecture.md`](design/epsilon-architecture.md) §2 — crate boundaries you
   must not violate.
3. Latest [`docs/milestones/`](docs/milestones) entry — current state and next step.
4. The runtime behavior contract Epsilon will ship lives at
   [`crates/epsilon-core/assets/agent-contract.md`](crates/epsilon-core/assets/agent-contract.md);
   it describes the *product*, not this repo's development rules.

---

## Roadmap acceptance criteria

| Milestone | Done when |
|---|---|
| M6 | Model can read/edit real files and run gated bash on this repo; permission rulesets evaluated last-match-wins with ask/deny/allow; every gate path tested |
| M7 | Long sessions auto-compact past 80% context; prompts byte-stable across identical inputs (hash-tested); summaries amend prior ones |
| M8 | A SKILL.md dropped into a directory appears in catalog and loads on demand; an extension process can register a tool; MCP server tools callable |
| M9 | Interactive TUI renders live events; `-p` print mode completes headlessly; serve mode streams SSE; an extension process receives filtered agent events over its stdio protocol |
| M10 | Linux confinement verified (landlock/bwrap); **Windows: bash children run inside a Job Object (CPU/memory/child caps) and the ACL restricted-token runner ships or is explicitly deferred with rationale**; release binary <15MB, changelog + tag |

---

## Contributing

Every change must pass:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets   # zero warnings tolerated
cargo test --workspace                   # full suite green
```

Rules of the house: dependencies pinned exactly (`=x.y.z`) after verifying features against the
crate's own manifest; no logic in `contracts`; architectural violations should be build errors,
not review comments; behavior changes need an ADR note. Commit convention:
`feat(mN): …` / `fix(mN): …` / `docs: …`.

---

*Built as a research-through-implementation project. The reference harnesses under
`references/` remain the property of their respective authors; their lessons are cited, their
code is not copied.*






