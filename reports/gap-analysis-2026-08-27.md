# Gap Analysis — Epsilon vs references (2026-08-27)

Method: 8 parallel scouts read every crate (`epsilon-contracts`, `epsilon-llm`, `epsilon-session`, `epsilon-core`, `epsilon-tools`, `epsilon-safety`, `epsilon-ext`, `epsilon-mcp`, `epsilon-tui`, `epsilon-cli`, `epsilon-server`) and all 4 reference trees (`pi-0.84.2`, `opencode-1.18.21`, `hermes-agent-2026.8.19`, `deepseek-harness-dsh-v0.1.1-rc.2`). Every claim below was verified against source during this pass — not carried from older reports.

Companion reports: `reports/harness-analysis.md` (strengths/weaknesses per harness), `reports/comparative-analysis.md` (who does what best), `reports/pi-alignment-2026-08.md` (layer-by-layer correspondence), `reports/research-pass-2-2026-08.md` (community signal + plan-mode import).

## 1. What Epsilon already does well (preserve)

| Area | Evidence | Keep |
|---|---|---|
| Pi 2-level loop verbatim + 3 named improvements | `crates/epsilon-core/src/lib.rs` (steering/followup queues, `QueueMode::OneAtATime`/`All`, `drain_steering_all` on abort, doom-loop brake) | Yes — ADR-002 |
| Byte-stable 3-tier prompt | `crates/epsilon-core/src/prompt.rs` (STABLE/CONTEXT/VOLATILE, hash-tagged project instructions, hash-tested) | Yes — Hermes discipline, ADR-008 |
| Never-fail StreamFn contract | `crates/epsilon-llm/src/stream_fn.rs` + `providers/{anthropic,openai_compat,google}.rs` (M17 direct HTTP, no genai) | Yes |
| JSONL tree + offset index + leases + repair | `crates/epsilon-session/src/{store,lease,repair}.rs` (in-memory index fixes pi's growth, `reindex`/`truncate_to_index`, `SessionLease`) | Yes |
| Fail-closed confinement | `crates/epsilon-safety/src/sandbox.rs` + `sandbox/jobobject.rs` (Job Objects on Windows, bwrap on Unix, `NullSandbox` refuses, denial signatures) | Yes |
| Ruleset engine + plan-mode monotonic deny | `crates/epsilon-safety/src/engine.rs` (`PermissionEngine::layered`, `plan_ruleset()`, `Action::Deny` immediate) | Yes |
| Model catalog v2 (386 rows, real prices) + live fetch + drift CI | `crates/epsilon-llm/src/catalog*`, `scripts/regen_catalog.py`, `.github/workflows/catalog-drift.yml` | Yes |
| Picker (2-stage, favorites, recents, custom providers) + palette + slash autocomplete | `crates/epsilon-tui/src/picker.rs` + `app.rs` (`picker_provider`, `palette_open`, `slash_open`/`slash_index`) + `fuzzy.rs` (pi matcher) | Yes |
| Thinking fidelity (shared ladder, per-wire mapping, `/think`, footer `think:<level>`) | `crates/epsilon-contracts/src/message.rs` (`ThinkingLevel`), `crates/epsilon-llm/src/providers/*`, `crates/epsilon-core/src/config.rs` (per-favorite memory) | Yes |
| Render cache + virtualized transcript + sticky scroll + todo/edit diff polish (M30) | `crates/epsilon-tui/src/{render_cache,scroll,state}.rs`, M30 commits | Yes |

## 2. Gaps classified

### Must have — clear gaps to close

| # | Gap | Source | Evidence | Fix shape |
|---|---|---|---|---|
| G-M1 | Manual `/compact` not exposed | pi `harness/compaction` (`/compact` command) + opencode `plan`/`compact` | `crates/epsilon-tui/src/picker.rs` `SLASH_ITEMS` has 8 entries, no compact/fork/timeline; `crates/epsilon-core/src/compact.rs` exists but only auto-triggers (pressure/overflow) | Add `Command::Compact(Option<String>)` + `SLASH_ITEMS` entry, `app.rs:run_command` arm that calls `Agent::compact` or queues compaction summary; ~40 lines |
| G-M2 | No persistent memory file (Hermes strongest idea) | hermes `MEMORY.md`/`USER.md` + `memory_provider.py` / `system_prompt.py` (byte-stable injection, prefetch, `is_trivial_prompt` gate) | `crates/epsilon-core/src/prompt.rs:load_project_instructions` only loads `AGENTS.md`/`CLAUDE.md` from workspace root; no `MEMORY.md` tier anywhere | Extend prompt loading to include `MEMORY.md` (project) and `~/.config/epsilon/MEMORY.md` (global) in CONTEXT tier, content-hash-tagged like other instructions; document in `agent-contract.md`; ~30 lines |
| G-M3 | Permission wildcards under-powered vs opencode | opencode `packages/core/src/util/wildcard.ts` (regex `*`→`.*`, `?`→`.`, backslash-normalized, `si` flags) | `crates/epsilon-safety/src/wildcards.rs` (74 lines) documents V1 simplification (`*` matches separators, `?` one char) and defers `**` / boundary rules to M10; tests only cover simple cases | Upgrade `glob_match` to opencode's regex semantics (normalize `\`→`/`, escape regex metas, `*`→`.*`, `?`→`.`), add `**` no-op (already `.*.*`), add backslash-normalized tests; keep no new dep |
| G-M4 | Session tree navigation not surfaced | pi `harness/session` (tree with `branch_summary`/`BranchBounds`/`navigate_to`, HTML export) + opencode `dialog-timeline.tsx` / `dialog-fork-from-timeline.tsx` | `crates/epsilon-session/src/store.rs` has `navigate_to(entry_id)`, `branch_entries()`, `BranchSummary` entry kind — but `crates/epsilon-tui` and `crates/epsilon-cli` never call `navigate_to`/`branch_entries` except internal `model_view`; no `/fork`, `/timeline`, `/branch` command | Add `Command::Fork` / `Command::Timeline` at minimum: `/fork` copies current branch to new session file and switches, `/timeline` lists branch entries (seq + snippet) for inspection; both use existing store APIs; TUI picker could gain a timeline dialog later |
| G-M5 | Built-in set diverges from docs (webfetch listed, not shipped) | `design/epsilon-architecture.md` §4 lists `webfetch` in built-in set; `crates/epsilon-safety/src/engine.rs:default_ruleset` has `webfetch` rule; `crates/epsilon-tools/src/lib.rs:register_builtins` registers only 8 tools (read/write/edit/bash/grep/glob/ls/todo) | Doc-code drift — architecture promises a tool the registry never registers | Either ship a minimal `webfetch` tool (reuse `epsilon-llm/src/http.rs` fetch + spill locator) or correct §4 to mark it deferred; shipping is truer to opencode's webfetch parity and the safety rule already exists |

### Worth adopting — meaningful improvement, fits architecture

| # | Idea | Source | Why useful | Cost |
|---|---|---|---|---|
| W1 | `@` file-reference autocomplete in prompt (pi `@` fuzzy file picker) | pi `packages/tui/src/components/prompt` (`@` autocomplete) | Cuts tool-call round-trips for file reads; educational "reducer sees `@` prefix" pattern | Small: reuse `epsilon-tools/src/search.rs` glob walker + `fuzzy.rs` matcher, inject mention into prompt |
| W2 | Session search pagination + result jump | pi `harness/session/search` (typed hits, newest-first, capped) + current `/search` | Current `epsilon-session/src/search.rs` + `Command::Search` already exist but hit rendering is a flat list; paging + `Enter` to jump to seq would make it usable on long sessions | Small: picker dialog for hits |
| W3 | Export session to HTML/JSON (pi `export-html`) | pi `packages/coding-agent/src/core/export-html` | Teaching artifact: replay JSONL outside the TUI; matches Hermes trajectory save | Small: CLI `--export <path>` reusing `store.read_all()` + template |
| W4 | Tool-output spill locator polish (DSH locator) | DSH `fs/fs-sandbox` + `crates/epsilon-tools/src/util.rs` spill threshold | Already implemented but threshold not surfaced in TUI hint bar; footer could show spill path | Tiny |

### Interesting but unnecessary — skip for v1

| Idea | Source | Why skip |
|---|---|---|
| Full Hermes curator / learning-graph / `/learn` pipeline | hermes `curator.py`, `learning_graph.py`, `learn_prompt.py`, `learning_mutations.py` | Requires auxiliary-model loop, skill maintenance daemon, trajectory store — high complexity, low teaching value at this stage; the simple `MEMORY.md` injection (G-M2) captures 80% of benefit |
| Opencode Effect-based plugin/tool system | opencode `packages/opencode/src/tool/tool.ts`, `packages/core/src/effect` | Epsilon's trait-based `Tool` + process-isolated `epsilon-ext` is intentionally divergent (ADR-009); adopting Effect would fight the Rust idiom |
| Background bash / tmux jobs | pi "use tmux" stance, opencode `background` package | Serve/rpc already enable remote orchestration; a background-bash tool needs approval-story design first (recorded in research-pass-2) |
| Telemetry / OTLP tracing crate | pi `packages/telemetry`, architecture §13 future note | Event bus + session log ARE the observability surface (already corrected in architecture doc); tracing crate is future work |
| OAuth flows (Copilot/Codex/Claude Max), Bedrock, image generation endpoints | pi `packages/ai/src/providers/all.ts` (40+ providers) | Env-var auth covers the teaching case; OAuth adds large non-educational surface |
| Self-modifying plugin toolsets (DSH cordis) | DSH `packages/sandbox/*` | Intentionally deferred in architecture §14 |

### Do not adopt

| Idea | Source | Why it would hurt |
|---|---|---|
| In-process jiti extensions with wide UI-takeover API | pi `packages/coding-agent/src/core/extensions` | Breaks ADR-009 isolation; same-process fragility observed in pi/opencode |
| Effect + LayerNode everywhere (opencode) | opencode core | Fights Rust ownership model; harms readability for learners |
| `${VAR}` code evaluation / `!!js` config | DSH anti-pattern | Violates ADR-012 declarative-only config |

## 3. What was intentionally not adopted (with rationale)

- Hermes `/learn` + curator + learning graph — see "Interesting but unnecessary" above; simple `MEMORY.md` tier is the right granularity for this project's educational purpose.
- Opencode/plugins/Effect idioms — diverge by design; Epsilon's trait + process isolation is the teaching point.
- Telemetry crate — architecture doc already corrected to honest "event bus + JSONL" observability.

## 4. Remaining gaps after this pass (if only Must-haves ship)

- Session tree UI will be minimal (slash commands, no full timeline picker dialog) — polish can land as a follow-up without architecture change.
- `@` autocomplete and export are nice-to-have UX, not correctness gaps — safe to defer behind the Must-haves.
- `webfetch` decision (ship vs doc-correct) should be taken explicitly in the commit that closes G-M5.

## 5. Credit table (what we take from whom)

| Adoption | Source | File/area | Why it fits |
|---|---|---|---|
| Steering/follow-up queues, JSONL trees, slash registry | pi | `epsilon-core/src/lib.rs`, `epsilon-session/src/store.rs`, `epsilon-tui/src/picker.rs` | Skeleton — ADR-002 |
| Byte-stable tiers, hash-tagged instructions, shadow view | Hermes | `epsilon-core/src/prompt.rs`, `epsilon-session/src/store.rs:ModelViewEntry` | Discipline — ADR-008 |
| Fail-closed sandbox, bounded pools, doom-loop, spill locators, denial signatures | DSH | `epsilon-safety/src/sandbox.rs`, `epsilon-core/src/exec.rs`, `epsilon-tools/src/util.rs` | Armor — ADR-005 |
| Rulesets (wildcard, monotonic deny, plan mode), auth store, favorites, command palette | opencode | `epsilon-safety/src/{engine,wildcards}.rs`, `epsilon-llm/src/auth.rs`, `epsilon-tui/src/picker.rs:build_provider_rows` | Manners — ADR-012/018 |
| Persistent `MEMORY.md` injection (G-M2) | Hermes `MEMORY.md`/`USER.md` | `epsilon-core/src/prompt.rs` (new) | Hermes strongest habit |
| Manual `/compact` + `/fork`/`/timeline` (G-M1/G-M4) | pi session/compaction | `epsilon-tui/src/{picker,app}.rs`, `epsilon-session` | Pi completeness |
| Opencode wildcard parity (G-M3) | opencode `wildcard.ts` | `epsilon-safety/src/wildcards.rs` | Opencode precision |
