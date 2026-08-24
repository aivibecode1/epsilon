# Changelog

All notable changes to Epsilon. Milestones map 1:1 to dev logs in `docs/milestones/`.

## [Unreleased]
### M16a - Rendering efficiency + footer parity
- Virtualized transcript: only visible rows materialized per draw; markdown+wrap
  happens once per line in an id-keyed render cache (draws ~30–69× faster and
  near-flat as sessions grow — bench via new `--tui-bench <n>` mode)
- Picker rows cached per filter; navigation no longer re-filters on every keypress
- pi footer parity: cwd (~-shortened) + git branch, compact token format (k/M),
  context-window usage with `?` right after compaction, cache-hit rate (CH%),
  session cost; threshold coloring >70%/90%
- Own unicode-width word-wrap replacing Paragraph::wrap (style-preserving,
  hard-splits oversized words, CJK-correct)

### M15 - TUI usability overhaul (pi/opencode patterns)
- Picker viewport fix: centered scroll window (pi select-list) + position indicator —
  long lists were previously unreachable past the box height
- Editor: Ctrl+←/→ word jumps, Home/End, Delete, Ctrl+U/Ctrl+W kills (unicode-aware)
- Markdown rendering for assistant messages via pulldown-cmark (headings, bold/italic,
  inline code, fenced blocks, lists, quotes)
- Braille spinner + elapsed seconds while running; contextual key-hint bar
- Mouse wheel scrolling; bracketed paste support; slash-command autocomplete (Tab)
- Visual prefixes (❯ user, ✓/✗ tool results); central theme palette

### M13 - Polish
- Live per-provider model listing in `/models` (Ctrl+F; genai resolver behind `real`)
- Persisted recently-used models (`[picker] recent`, shown as a picker group)
- Markdown-lite transcript rendering (fenced code blocks styled, inline markers stripped)
- Expandable tool cards: `x` toggles full output vs truncated head
- bwrap integration tests (`#[cfg(unix)]`) — runtime verification path for Linux CI
- Fixed latent `--features real` compile break in the CLI (GenaiAdapter re-export)


### M12 - Completion pass
- Project config layer: `.epsilon/config.toml` ([model], [[rules]], [[extensions]],
  [[mcp_servers]]) with SHA-256 trust hash + change banner (completes ADR-012 core)
- Reliability: `Retrying` StreamFn executor (pre-stream failures, classified, jittered
  backoff, on_retry hook); run-scoped cancellation reaching both shell paths (Esc aborts);
  session cost in the TUI footer
- `--resume <path>` / `-c/--continue` (newest session file; shadowed-view restore)
- rpc(stdio) mode (`--rpc`) - NDJSON ops in, AgentEvent frames out
- `GET /openapi.json` on serve mode (hand-maintained 3.1 spec + drift-guard test)

### M11 - Model picker, catalog, runtime wiring
- pi-style /models TUI picker (filter, provider groups, custom ids) + /model, /help, --list-models
- Bundled catalog: 18 models / 10 providers with env-key auth hints
- [model] id persistence in ~/.config/epsilon/config.toml (first ADR-012 slice)
- Skills + extensions now load from conventional directories (closes M8 reachability debt)
## [0.1.0] - 2026-08-23

The first complete harness: pi's proven loop and session format, DSH-grade two-layer
safety, Hermes-grade prompt discipline, opencode-grade manners — one static binary,
three frontends, offline-testable end to end.

### M0–M2 · Foundation
- Docs baseline, reference audit (pi / DSH / Hermes / opencode), 17 ADRs.
- 11-crate workspace with compile-enforced dependency direction; quality gates green
  from commit one (`fmt`, zero-warning `clippy`, offline tests).
- `epsilon-contracts`: pi's type vocabulary ported — messages, stream events, agent
  events, tool descriptors, sticky turn-end reasons; types only, zero logic.

### M3 · Provider layer
- Never-fail `StreamFn` contract: provider failures arrive in-band as
  `Done{stop_reason: Error}`; a `PanicGuard` converts even panics.
- Scripted faux provider (offline, deterministic, free) + genai 0.6 adapter for real
  providers behind the `real` feature; retry policy keyed by error kind.

### M4 · Sessions
- Append-only JSONL tree store with shadowed model view, cross-process turn leases
  (30s steal), crash repair (synthetic tool results, typed `TurnEnd{Aborted}` markers).

### M5 · The loop
- pi's two-level loop verbatim: steering inner / follow-up outer, explicit queue modes.
- Named improvements: semaphore-bounded parallel tool pool with model-order commits;
  fail-all-tool-calls on truncation; doom-loop brake at 3 identical calls.

### M6 · Real tools + permissions
- read/write/edit/bash/grep/glob/ls against the declared workspace; path-escape
  downgrades to an `external_dir` capability that asks before running.
- Last-match-wins wildcard rulesets (defaults ← global ← project) with ask/deny/allow,
  once/session memory, deny-with-guidance fed back to the model.

### M7 · Compaction + prompts
- Auto-compaction past the token threshold: prune old tool outputs → clamp tail →
  isolated summarizer call → amend prior summaries → overflow retry once per run.
- Three-tier byte-stable prompt assembly (STABLE/CONTEXT/VOLATILE), hash-tested.

### M8 · Skills, extensions, MCP bridge
- Agent-Skills-spec catalog in CONTEXT tier, bodies loaded on demand.
- Process-isolated extension host over NDJSON-RPC with timeouts, crash budget, and
  manifest-filtered event forwarding.
- MCP tools bridged into the normal registry under `mcp__<server>__<tool>`.

### M9 · Frontends
- **M9a**: extension event forwarding (closes known-gap #1); print mode `-p`.
- **M9b**: ratatui TUI (`--tui`) — live streaming transcript, approval dialogs, steering
  from the input box; serve mode (`--serve`) — axum HTTP/SSE with prompt/steer/approve
  REST; rmcp stdio transport verified against a real spawned MCP server binary.

### M10 · Hardening & release
- **M10a**: OS confinement seam (ADR-005 Layer 2): Windows Job Objects with
  memory/active-process/CPU-rate caps, kill-on-close, breakaway refused; Unix bubblewrap
  backend; fail-closed policy selection (`Auto|Off|Custom`, `--sandbox` flag);
  `[confinement]` denial signatures distinguish blocks from failures. Permission events
  now carry correlatable request ids. Windows ACL restricted-token runner deferred with
  rationale (ADR-005 amendment).
- **M10b** (this release): real-provider wiring via `--provider real --model <id>`
  (genai auth from env), configurable model id plumbed through prompt tiers and
  compaction summarizer, release profile tuned, binary 8.8 MB (<15 MB criterion),
  changelog + `v0.1.0` tag.

### Verification posture
104 offline tests, zero clippy warnings, byte-stable prompts hash-tested, full-loop
scenarios run against scripted providers — CI never touches paid APIs.

