# Project history

Epsilon grew milestone by milestone. Each one landed as a single commit with a
dev log under [`milestones/`](milestones) and an entry in the root
[`CHANGELOG.md`](../CHANGELOG.md). This file holds the timeline and the
reference tables that used to bloat the README: what shipped when, the original
acceptance criteria, and the decision-log index.

## Milestone timeline

| Milestone | Scope |
|---|---|
| M0 | Docs baseline, git, build-order ADR |
| M1 | 11-crate workspace, quality gates green from commit one |
| M2 | `epsilon-contracts`: pi's type vocabulary ported |
| M3 | `epsilon-llm`: StreamFn contract, faux provider, provider adapter, retry (genai wrapper era) |
| M4 | `epsilon-session`: JSONL tree store, shadow views, leases, crash repair |
| M5 | `epsilon-core`: full agent loop + runnable demo |
| M6 | Real tools (read/write/edit/bash/grep/glob/ls) + permission ruleset engine |
| M7 | Compaction engine + three-tier byte-stable prompt builder |
| M8 | Skills, extension host (process-isolated), MCP bridge |
| M9a | Extension event forwarding + print mode `-p` |
| M9b | ratatui TUI (`--tui`), serve mode SSE (`--serve`), rmcp stdio transport |
| M10 | Hardening pass: sandbox backends incl. Windows Job Objects, release packaging |
| M10a | Sandbox seam (Job Objects / bwrap, fail-closed) + permission-event correlation fix; ACL runner deferred with rationale (ADR-005) |
| M10b | Release packaging: real providers, model plumbing, 8.8 MB binary, tag `v0.1.0` |
| M11 | Model picker + bundled catalog (`/models`, `--list-models`), config slice, skills/extensions runtime wiring |
| M12 | Project config + trust hash, retry executor + cancellation + cost display, `--resume`, RPC mode, OpenAPI endpoint |
| M13 | Live catalog fetch, recents, markdown-lite transcript, expandable tool cards, bwrap test suite for Linux CI |
| M14 | Plain `epsilon` launches the TUI by default, provider auto-detect from env keys with faux fallback, slash-command repairs |
| M15 | TUI usability overhaul porting pi/opencode patterns: picker scroll window, unicode editor ops, pulldown-cmark transcript, spinner, hint bar, mouse wheel, bracketed paste, slash autocomplete |
| M16a | Rendering efficiency: id-keyed render cache, virtualized transcript (~30-69x faster draws), cached picker rows, pi footer parity, `--tui-bench` |
| M16b | Editor v2: Emacs kill-ring, atomic paste markers, multi-line buffer, persistent prompt history |
| M16c | Two-stage model picker, command palette (Ctrl+P), draft stash across dialogs, pi fuzzy matcher |
| M17 | pi-agent port: direct per-provider HTTP replaces genai (Anthropic/OpenAI-compat/Gemini wires), catalog v2 regenerated from live models.dev with real prices, slash autocomplete with descriptions, keybind parity sweep |
| M18 | Docs audit, beginner primer, pi-alignment report + session search (`/search`), plan mode imported from opencode (`--plan`) |
| M19 | All 8 improvement plans executed: stop-reason fidelity on all three wires, loop integrity fixes, serve-mode bearer token + Host guard, secret-leak closure, wire fixture tests in CI, byte-safe SSE buffer, extension event forwarding wired, CLI composition cleanup |
| M20 | Catalog regeneration pipeline (`scripts/regen_catalog.py`, 383 models.dev rows), live fetch fixed and auto-firing on picker open |
| M21 | Thinking fidelity: shared ladder in contracts, native mapping per wire (Anthropic adaptive, OpenAI reasoning_effort families, Google thinkingConfig), `/think` + Ctrl+T, carried per session |
| M22 | opencode auth store (`epsilon auth login/list/logout`, stored-wins/env-fallback), trust re-prompt holds the TUI until answered |
| M23 | Picker favorites: Ctrl+F stars, favorites above recents, F2/Shift+F2 cycling (pi semantics) |
| M24 | MCP health probes (30s TTL, fail-fast honest errors), provider fallback chains (`--fallback`, repeatable, per-turn primary restoration) |
| M25 | Footer thinking-level indicator (`think:<level>`, pi pattern, hidden while Off); displayed state pinned to executed state via the set_thinking choke point |
| M26 | Per-favorite thinking levels (pi scoped-models): starring remembers the level, F2 applies it on cycle |
| M27 | Weekly catalog-drift CI job (Monday regen vs live models.dev, PR when drifted); audit follow-up put F2 cycling under the trust gate |
| M28 | Custom OpenAI-compatible providers (`[provider_custom.<name>]`, `<name>/<model>` ids): user-declared endpoints ride the existing wire routing with stored/env keys |
| M29 | Steering queue badge + drain-on-abort restore to editor |
| M30 | Todo tool + checklist rendering; edit diff preview + TUI expandable unified diff |
| M31 | Reference gap closure: /compact, /timeline, /fork, MEMORY.md tier, wildcard parity, webfetch deferred honestly |

Nothing is currently scheduled under "future"; new items enter the roadmap when
someone names one.

## Original acceptance criteria

Written at M0, kept for the record. Every criterion below is closed; M10's ACL
runner was the one deliberate deferral, with rationale in ADR-005.

| Milestone | Done when |
|---|---|
| M6 | Model reads/edits real files and runs gated bash here; rulesets evaluated last-match-wins with ask/deny/allow; every gate path tested |
| M7 | Long sessions auto-compact past 80% context; prompts byte-stable across identical inputs (hash-tested); summaries amend prior ones |
| M8 | A SKILL.md dropped into a directory appears in catalog and loads on demand; an extension process registers a tool; MCP server tools callable |
| M9 | Interactive TUI renders live events; `-p` completes headlessly; serve streams SSE; an extension receives filtered events over stdio |
| M10 | Linux confinement verified (bwrap); Windows bash children run inside a Job Object; ACL runner shipped or explicitly deferred with rationale; release binary <15MB, changelog + tag |

## Decision-log index

Full reasoning lives in [`design/epsilon-decisions.md`](../design/epsilon-decisions.md);
one line per ADR:

| ADR | Decision |
|---|---|
| 001 | Rust on tokio (LTS channel), single binary |
| 002 | Port pi's loop shape verbatim; improve only at named seams |
| 003 | Provider layer: pi-style direct per-provider HTTP behind our own `StreamFn`; wire types never leak (M17 superseded the original genai wrapper); config-declared custom endpoints since M28 |
| 004 | Tools defined as typed structs; schemars-derived JSON Schema; `.txt` descriptions |
| 005 | Two-layer safety: permission gates over fail-closed confinement; honest per-OS limits |
| 006 | Sessions = append-only JSONL trees + offset index + advisory-lock turn leases |
| 007 | Compaction: prune, clamp tail, strict template, amend prior summaries, shadow view |
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
| 018 | Model selection: TUI picker over bundled generated catalog; flag > saved config > default precedence |
