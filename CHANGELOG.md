# Changelog

All notable changes to Epsilon. Milestones map 1:1 to dev logs in `docs/milestones/`.

## [Unreleased]
### M27 - Weekly catalog-drift CI job
- **Monday regeneration against live models.dev**: a scheduled workflow runs
  `scripts/regen_catalog.py`, formats, and opens/refreshes ONE pull request
  (`catalog-drift/latest`) when the bundled catalog drifted. Silent when it
  did not. Never pushes main; the refresh rides the normal CI gates like any
  change. Manual fire available via workflow_dispatch.
- **Audit follow-up (same milestone)**: F2 favorite cycling is now gated by
  the M22 project-config trust review, like Ctrl+P already was — since M26 a
  cycle also mutates the thinking level and persists config, so it must not
  act against unreviewed rules. Gate-blocked regression test added.

### M26 - Per-favorite thinking levels
- **Ctrl+F remembers the level**: starring a model stores your current
  thinking level for it (`[picker.favorite_thinking]` in the global config);
  unstarring drops the memory. Star notice names what it remembered.
- **F2 / Shift+F2 apply it** (pi scoped-models semantics): a remembered level
  overrides the session level on cycle; favorites without a memory inherit.
  Memory is mirrored on `App` (loaded at startup, refreshed on toggle) so
  cycling never touches disk mid-keypress.

### M25 - Footer thinking-level indicator
- **`think:<level>` in the status bar** (pi pattern): renders between cost and
  model whenever extended thinking is active, hidden while Off. `/help`
  updated to match.
- **Displayed state = executed state**: the level is mirrored into one `App`
  field written only by `set_thinking` (the same choke point that updates the
  agent); a key-path test asserts mirror == agent after Ctrl+T cycles.

### M24 - MCP health probes + fallback chains (DIRECTION-02/03 closed)
- **MCP health probes**: `Connector::is_healthy` (stdio detects a dead child
  via rmcp); the bridge probes at most once per 30s TTL before each call, so
  a dead server fails fast with "process is not running... re-attach with
  --mcp" instead of hanging or erroring opaquely mid-call.
- **Provider fallback chains**: repeatable `--fallback <model>` composes
  `epsilon_llm::fallback::FallbackProvider(primary, candidates...)`.
  Retryable failures (rate limit / transient 5xx / network) walk the chain;
  auth failures stop it. Per-turn primary restoration: every call starts at
  the head, so a recovered primary is used again automatically.

### M23 - Picker favorites + favorite cycling (DIRECTION-06)
- **Star any model** with Ctrl+F inside the picker (opencode's
  `model_favorite_toggle`); ★ column in both picker stages; persisted in
  config `[picker] favorite` (cap 16, loud error when full).
- **Favorites section above Recents** in stage-1 rows, recents filtered to
  exclude favorited ids (opencode dialog order); star follows the id into the
  full catalog list too.
- **F2 / Shift+F2** cycle favorites forward/backward from the editor (pi
  `cycleModel` semantics: wrap-around, non-listed current enters forward,
  every switch persists + updates recents). Empty list gives guidance.
- Live force-refresh moved from Ctrl+F to Ctrl+G (auto-fetch on open already
  covers the common case). Hint bar updated.
- Suite: 239 tests passing.

### M22 - opencode auth store + trust re-prompt (DIRECTION-01)
- **`epsilon auth` subcommand** (opencode auth.json port): `login <provider>
  [key]`, `list` (masked), `logout <provider>`. Keys live in
  `~/.config/epsilon/auth.json` in opencode's exact schema; 0600 on unix.
- **Precedence**: stored key wins, env var falls back (`catalog::resolve_api_key`);
  auto-real detection, picker auth column, live `/models` fetching, and request
  auth all follow one resolver. Store lives in epsilon-llm (pi keeps provider
  auth inside `packages/ai`; dependency law forbids llm -> core).
- **Trust re-prompt** (pi trust-manager semantics): `load_project_config`
  returns an explicit `TrustDecision`; the TUI holds all submits on a changed
  `.epsilon/config.toml` until you type `trust` (accept) or `trust no` (quit).
  Headless modes keep the banner policy; the old hash persists until accepted.
- Suite: 234 tests passing.

### M21 - Thinking fidelity sweep (DIRECTION-04)
- **Shared ladder** (`epsilon-contracts::ThinkingLevel`): off/minimal/low/
  medium/high/xhigh/max with cycle + parse semantics ported from pi.
- **Per-wire native mapping**: Anthropic adaptive thinking + `output_config.effort`;
  OpenAI-compat family dispatch (gpt/o-series + kimi `reasoning_effort`,
  OpenRouter nested `reasoning`, zai GLM `thinking{clear_thinking}`, deepseek
  `thinking{enabled}`); Google `generationConfig.thinkingConfig` with
  `includeThoughts` + level. `Off` sends nothing on every wire. Fixture tests
  pin all three request shapes.
- **Session state**: `Agent::set_thinking_level`/`thinking_level`; the level
  rides every run like the model id; compaction summarizer stays Off (ADR-007).
- **TUI surface**: `/think <level>` (bare = report, unknown = inline correction)
  and Ctrl+T to cycle the ladder with an immediate notice.
- Suite: 226 tests passing.

### M20 - Catalog regeneration + live picker refresh
- **Generated catalog** (DIRECTION-05): `scripts/regen_catalog.py` fetches
  models.dev and writes `epsilon-llm/src/catalog/catalog_data.rs` — 383 rows
  across the nine direct-API providers (was ~40 hand-curated), with auditable
  selection rules (tool-calling text models, dated snapshots excluded,
  first-provider-wins dedupe) and registry-faithful prices.
- **Live fetch fixed**: M17's Ctrl+F passed a MODEL ID where `live_models`
  requires a PROVIDER name, so live listing never worked; the contract is now
  honored at the single shared call path.
- **Auto-refresh picker** (opencode/Hermes convention): opening `/models` (or
  Ctrl+M) and descending into a provider auto-fetch that provider's live list
  once per session; Ctrl+F force-refreshed at the time (since M23 that key
  stars a favorite and Ctrl+G refreshes). Live ids merge into both
  picker stages under their own provider group, no duplicates against bundled
  rows.
- **Clippy zero under `--features real --all-targets` too**: M19's fixture
  visibility widening had left 11 warnings in an ungated lane; docs added on
  wire structs, collapsible ifs collapsed, `Usage` initializer-built.
- Suite: 221 tests passing.

### M19 - Audit remediation: all 8 improvement plans executed
- **Provider stop-reason fidelity** (plan 001): OpenAI `[DONE]` maps the recorded
  `finish_reason` (`length` → MaxTokens, `content_filter` → Error) instead of always
  EndTurn; Anthropic `message_stop` consumes `message_delta`'s stop_reason; Gemini
  early-return now accumulates parts before the finish flag instead of dropping them.
- **Loop integrity** (plan 002): overflow-retry latch resets per run (recovery works
  more than once per process); drained-but-uninjected steering messages requeue on
  abort/budget/error exits; compaction clipping and `/search` snippets are
  char-boundary safe (CJK/emoji no longer panic or mojibake).
- **Serve-mode trust boundary** (plan 003): per-boot bearer token required on every
  route except `/healthz` + `/openapi.json`; loopback Host allow-list + cross-origin
  Origin rejection; unattended runs default to deny-with-guidance, `--yes` opts into
  auto-approve with a warning.
- **Secret-leak closure** (plan 004): engine deny evaluated BEFORE session-memory
  promotions; grep skips secret-class files (`.env*`, credentials, `.npmrc`, ...);
  gate subjects lexically normalized so `.env`/`./.env`/`src/../.env` match one rule.
- **Wire fixture harness** (plan 005): `tests/wire_fixtures.rs` drives all three
  adapters with raw JSON frames; CI gained a real-feature TEST lane.
- **Byte-safe SSE buffer** (plan 006): line reassembly is raw bytes; multi-byte chars
  split across TCP chunks survive.
- **Extension event forwarding wired** (plan 007): host evaluates its manifest's
  `events` filter at send time (hardcoded early-return had made it dead code).
- **CLI composition cleanup** (plan 008): `RunParams`/`SharedRun` structs replace the
  11-param composition; single Diagnostics sink; reasoned targeted dead_code allow;
  unused manifest deps dropped; AGENTS.md stale test count fixed.
- Suite: 220 tests passing. Plans live in `plans/` with the audit report at
  `reports/re-audit-2026-08-24.md`.

### M18 - Alignment pass: docs audit, pi parity closure, plan mode
- **Documentation audit**: M14 dev log backfilled from commit evidence
  (`docs/milestones/M14-harness-maturity.md`); beginner primer added
  (`docs/qa/06-agent-harness-primer.md`); stale genai-era claims purged from README,
  architecture doc, ADRs, and qa notes; Rust requirement corrected to >= 1.88;
  architecture-doc overclaims (edit mutation queue, tracing/OTLP) corrected to
  shipped reality.
- **Session search** (pi `search/scanning.ts` parity): case-insensitive scan over
  user/assistant/tool-result text with snippets, newest-first, limit and
  active-branch options; `/search <query>` TUI command; re-exported through
  epsilon-core so the TUI keeps zero direct session dependency. 7 new tests.
- **Plan mode** (opencode `plan` agent import): `--plan` runs deny write/edit
  outright plus git commit/push commands; monotonic-deny makes the boundary hold
  against project rules; startup banner states the posture. Live-verified against
  the faux provider including a control run.
- Reports: `reports/pi-alignment-2026-08.md` (layer-by-layer pi correspondence),
  `reports/research-pass-2-2026-08.md` (community signal + import decisions).

### M17 - pi-agent provider + TUI port (full attribution)
- **Provider layer rewritten as a faithful pi port**: direct per-provider HTTP
  (Anthropic Messages, OpenAI-compatible `/chat/completions`, Google
  `streamGenerateContent`) replaces the genai router crate entirely.
  Ported from pi-0.84.2 `packages/ai/src/api/` and `packages/ai/src/providers/`;
  SSE decoding follows pi's `iterateAnthropicEvents` wire handling; request shapes,
  auth headers (`x-api-key`+`anthropic-version`, `Bearer`, `x-goog-api-key`), usage
  extraction (`message_start`/`message_delta`, `include_usage`, `usageMetadata`),
  tool-call assembly (indexed OpenAI deltas, `input_json_delta` buffering) all cite
  their pi origin in doc comments. New dep: `reqwest =0.13.4` (already in-tree via
  rmcp); `genai` removed from the workspace.
- **Catalog v2** regenerated from a live models.dev fetch (2026-08): current model
  generations across anthropic/openai/deepseek/google/xai/groq/moonshot/z.ai/
  openrouter/ollama with REAL published prices replacing M3 placeholders — footer
  cost is now an estimate against public list prices. Each row carries a `wire`
  column (pi's `model.api`): routing reads capability data, never id substrings.
  Ids are provider-native (`kimi-k2.6`, `glm-5`, `grok-4.6`, `gpt-5.4`,
  `claude-opus-5`, `gemini-3.6-flash`).
- **Live listing** (`real` builds) now plain authenticated GET `/models` per provider
  (pi/opencode convention), replacing genai's resolver.
- **Slash autocomplete rebuilt on pi's pattern**: arrow-key navigable popup with
  descriptions (pi `AutocompleteItem`+`SelectList`), Up/Down move the highlight
  (history recall suspended while open), Enter applies-and-runs the highlighted
  command (pi applyCompletion→submit fall-through), Tab completes the highlight,
  Esc dismisses before quit/abort. Popup state tracks the buffer — it never lingers
  over unrelated text.
- **Keybind parity sweep** (opencode keybind.ts / pi keybindings.ts defaults):
  Ctrl+A/E line home/end, Ctrl+B/F char left/right, Alt+B/F and Alt+←/→ word jumps,
  Alt+Backspace word kill, Ctrl+D forward delete, Ctrl+K kill-to-line-end (kill-ring
  integrated), Ctrl+. redo (new redo rail in undo.rs; divergent push clears it).
  (An "F2 cycles recents" claim in this entry was aspirational and never shipped;
  F2 arrived in M23 cycling favorites.)
- Tests: epsilon-tui 79 green (12 new key-path regressions), epsilon-llm 24 green
  under `--features real` (SSE decoder fixtures, catalog routing/auth invariants,
  current-generation guards).

### M16c - Picker/palette parity (opencode patterns)
- Two-stage model picker: providers first (auth + active marks, model counts),
  Enter descends into that provider's models with live fuzzy filtering;
  Esc walks back stage-by-stage; Ctrl+F live-fetch retained
- Command palette (Ctrl+P): fuzzy-filtered list of all commands with keybinding
  hints, dispatched through the same parse path as typed slashes
- Draft stash: opening any dialog parks the in-progress draft and restores it on
  close (opencode prompt.stash)
- Fuzzy matcher ported from pi's fuzzy.ts (word-boundary/consecutive scoring),
  zero new dependencies
- Editor undo (Ctrl+Z): bounded snapshot stack restoring buffer + cursor before
  backspaces and kills (pi undo-stack.ts parity)

### M16b - Editor v2 (pi editor behaviors)
- Emacs kill-ring: Ctrl+U/Ctrl+W push kills, Ctrl+Y yanks; consecutive kills
  accumulate into one entry; repeated Ctrl+Y cycles entries yank-pop style
- Atomic paste markers: bracketed pastes become `[paste #N +L lines]` in the
  editor, expand to full text on submit; Ctrl+W removes a marker whole
- Multi-line buffer: Ctrl+J inserts a newline (reliable on legacy Windows
  consoles), Enter submits the whole draft; caret tracks line/column
- Persistent prompt history: `~/.config/epsilon/history` (JSON-lines, capped 500,
  corrupt-line tolerant), seeded into Up/Down recall across restarts

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

