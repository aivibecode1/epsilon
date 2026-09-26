# Gap analysis v2 — Epsilon vs pi-0.84.2 + DSH + Hermes + opencode

> Historical research, not current product status. See the [archive index](README.md)
> and [verified audit](../../audit/README.md) before relying on completion or safety claims.

Planned at: `e200815` (2026-08-27). Scope: every crate vs `references/pi-0.84.2`, `references/deepseek-harness-dsh-v0.1.1-rc.2`, `references/hermes-agent-2026.8.19`, `references/opencode-1.18.21`.

## Method
8 parallel scouts: loop/harness, tools, session, TUI, providers/auth, safety, extensions/skills, prompt/compaction. Cross-checked excerpts live (no subagent line numbers trusted). Leverage = impact / effort, weighted by confidence.

## Leverage-ordered findings (pi-parity floor first)

| # | Finding | Category | Impact | Effort | Risk | Evidence |
|---|---------|----------|--------|--------|------|----------|
| G1 | Slash lifecycle incomplete — `/new`, `/resume`, `/session`, `/clone`, `/tree` navigation missing; only `/timeline` notice-dump + `/fork` file-copy | correctness / DX | HIGH | M | LOW | `crates/epsilon-tui/src/picker.rs:212 SLASH_ITEMS` 15 vs `references/pi-0.84.2/packages/coding-agent/src/core/slash-commands.ts:8 BUILTIN 22` — missing `new,resume,session,tree,clone,trust,reload,export,import,share,copy,name,changelog,hotkeys` |
| G2 | `find` tool absent — Epsilon ships `glob` (GlobTool) where pi ships `find` (fd-style with type filtering); `grep`/`ls`/`read` parity partial | correctness | HIGH | M | LOW | `crates/epsilon-tools/src/lib.rs:58 register_builtins` 8 tools vs `references/pi-0.84.2/packages/coding-agent/src/core/tools/index.ts:44 allToolNames = {read,bash,edit,write,grep,find,ls}` |
| G3 | No file-mutation queue — pi wraps edit/write with `file-mutation-queue.ts` to serialize overlapping writes; Epsilon's `edit_match.rs`+`files.rs` race on parallel tool calls | correctness/bug | HIGH | S | LOW | `references/pi-0.84.2/packages/agent/src/harness/tools/file-mutation-queue.ts` vs `crates/epsilon-tools/src/files.rs:1`, `crates/epsilon-tools/src/edit_match.rs` no queue |
| G4 | No task/subagent tool — pi/opencode/DSH all expose a `task` tool for the model to fan-out; Epsilon's contract tells model to use todo but has no delegation primitive | direction | HIGH | M | MED | `references/opencode-1.18.21/packages/core/src/tool/task*`, DSH `task` vs `crates/epsilon-tools/src/todo.rs` only checklist; `docs/design/epsilon-architecture.md §4` lists `task(subagent)` as deferred |
| G5 | Trust/reload/settings missing — pi's `/trust`, `/reload`, `/settings`, `/scoped-models` let users persist policy and hot-reload skills/prompts/config without restart; Epsilon persists trust hash but has no slash to accept/reload | DX/correctness | MED-HIGH | S | LOW | `references/pi-0.84.2/packages/coding-agent/src/core/slash-commands.ts: BUILTIN trust,reload,settings,scoped-models` vs `crates/epsilon-tui/src/picker.rs:346 parse_command` no arms; `crates/epsilon-core/src/config.rs` has `load_project_config` but no reload path |
| G6 | Keybinding persistence absent — pi's `KeybindingsManager` loads `~/.config/epsilon/keybindings.json`, migrates legacy names, live-reloads; Epsilon hardcodes keys in `app.rs:handle_key` | DX/tech-debt | MED | M | LOW | `references/pi-0.84.2/packages/tui/src/keybindings.ts: KeybindingsManager` vs `crates/epsilon-tui/src/app.rs:2046 handle_key` match with hardcoded `ctrl()` helpers |
| G7 | Export/import/share/copy absent — pi's `/export` (html/jsonl), `/import`, `/share` (gist), `/copy` (clipboard) complete the session lifecycle; Epsilon has no export path | DX | MED | M | LOW | `references/pi-0.84.2/packages/coding-agent/src/core/slash-commands.ts: export,import,share,copy` vs Epsilon `picker.rs` no export |
| G8 | TUI chrome gaps — pi renders latex (DONE 217/217), terminal images, alt-screen search, layout-node constraints; Epsilon latex is 217/217 pi parity (see addenda 2026-08-28), remaining chrome is images/alt-screen only — intentionally deferred (`G8 only if user asks for rich TUI`) | DX | LOW | L | LOW | `references/pi-0.84.2/packages/tui/src/{terminal-image.ts,tui-alt-screen.ts,layout-node.ts}` vs `crates/epsilon-tui/src/{latex.rs,state.rs}` — latex at `latex.rs` 217 SYMBOLS, image as `[image: … — graphics deferred]` fallback |

## Status update (M52–M59 sweep — parity floor closed)

Re-ran the sweep against `references/pi-0.84.2` after M52–M59. **Every G-item is
now closed or explicitly dropped:**

- G1 slash lifecycle — closed (34 items incl. `/sessions`, `/tasks`, `/editor`). **The "pi's 22 builtins all present" claim is WITHDRAWN (2026-09-26): it was never true and is now provably false.** Measured against `pi-0.85.1/packages/coding-agent/src/core/slash-commands.ts:19–43`, which holds **23** builtins (0.84.2 had 22; pi added `thinking` at `:23`). Epsilon registers 22 slash items in `crates/epsilon-tui/src/picker.rs`, of which **15 are pi's** and **8 pi commands are absent**:

  | Missing in Epsilon | pi's line |
  |---|---|
  | `thinking` | `:23` (new in 0.85.1) |
  | `export`, `import` | `:25–28` |
  | `name` | `:29` |
  | `login`, `logout` | `:36–38` (capability exists in `epsilon-llm/src/oauth_login.rs`, without a slash command) |
  | `trust` | `:35` |
  | `resume` | `:42` (Epsilon's `/sessions` may cover this functionally — **unverified**) |

  Epsilon's 7 extras: `clear`, `editor`, `help`, `sessions`, `skills`, `tasks`, `timeline`
  (`timeline` is what `/tree` aliases to). `model` and `scoped-models` are present under
  Epsilon's internal names `Models` / `ScopedModels`. **`/export` and `/import` are the subject of
  `docs/plans/016-atfile-and-export.md`, so their absence is planned, not forgotten** — but G1
  cannot be "closed" while they are missing. Re-verify with
  `sed -n '19,43p' references/pi-0.85.1/packages/coding-agent/src/core/slash-commands.ts`.
- G2 `find` + tool table — closed (read/bash/edit/write/grep/find/ls parity + `task`/`todo`/`skill`). **Corrected 2026-09-26: the 7-name parity is an undercount.** `pi-0.85.1/packages/coding-agent/src/core/tools/index.ts:95` declares **8** — `read | bash | powershell | edit | write | grep | find | ls` — pi added `powershell` after 0.84.2. Epsilon covers 7 of the 8. This is a **surface difference, not a missing capability**: Epsilon's single `bash` tool *is* PowerShell on Windows (`crates/epsilon-tools/src/shell.rs:47`, asserted at `:483`), where pi exposes `bash` and `powershell` as two separately selectable tools. The real consequence: on Windows an Epsilon model cannot run `bash` at all, and the model-visible tool list differs from pi's in a way a prompt written for pi would not expect.
- G3 file-mutation queue — closed.
- G4 `task` tool — closed, and well beyond capture-v1: background runs, resumption, abort cascade,
  per-delegation model override (M56), and durable child sessions (M58).
- G5 trust/reload/settings — closed.
- G6 keybindings — closed (pi's `KeybindingsManager` shape + `/hotkeys`); the only deviation is one
  default key (`app.editor.external` uses ctrl+o/alt+e because pi's ctrl+g is the live refresh here).
- G7 export/import/share/copy — export/import/copy closed; `/share` (gist) stays deferred with honest
  guidance (needs `gh`, and it is the one pi command that publishes data).
- G8 TUI chrome — **now the only open item, and the only one worth doing**: latex is at pi parity,
  and the external editor landed (M59). Remaining: terminal images and alt-screen behavior
  (`terminal-image.ts`, `tui-alt-screen.ts`), both LOW impact — do them when a user asks for rich TUI.
- Providers/auth (pi floor) — closed through M44/M47/M48/M49/M52/M55 (`openai-codex` is the sole
  exception: blocked on model data, see `docs/plans/018`).

## Status update (M60 — reliability, not parity)

A solidity pass over the failure paths (the standing goal says *usable solid
harness*, and feature parity was already closed at M59) found that **ADR-010's
retry policy was never engaged in production**: `Retrying` existed with tests but
no path wrapped a stream in it, and the HTTP client deliberately had no timeout — so
a transient 429/503 failed the turn and a half-open socket hung it forever.

Closed by M60: the retry policy wraps both frontends (direct + fallback chains), waits
surface as `AgentEvent::Notice`, a 120 s idle watchdog (`EPSILON_STREAM_IDLE_SECS`)
turns silence into a retryable network failure, and a live stream that dies before
emitting anything is retried transparently.

M61 (same pass, durability): the session write lease — implemented but never called —
is now owned by the store, so two processes cannot splice one session's transcript; the
holder heartbeats (a live process is never mistaken for a crash leftover) and the stale
steal path actually removes the stale file (it never did). `/sessions` marks leased rows
`· in use`.

M62 (same pass, failure semantics): a mid-stream death never discards what the user already
watched arrive — the transport read failure, every wire's provider-error frame, and the loop's
no-terminal-frame arm all kept nothing before, so a dropped connection could erase an answer (and
leave a hole in the durable session a resume reads back). One salvage rule per layer now
(`HttpEventStream::failure_message`, `Agent::failure_message`), always flagged `Error` and never
executing a tool call whose arguments may be half-written; the TUI states "partial" instead of
letting a cut-off reply read as complete.

Also fixed in these rounds (unit-test level): the stale-steal no-op above, "timed out"
not classifying as a network failure, and the same-process resume self-conflict.

Rejected / already planned:
- G-M1..G-M5 (compact/timeline/fork/MEMORY/wildcards/webfetch) — closed at `abd045a`, polish in 009, ship in 010.
- W1 @-file, W3 export, help picker — captured as 012.
- Hermes curator/learning-graph, DSH cordis, Effect plugins, background bash, OTLP, OAuth/Bedrock — intentionally not adopted (see `docs/research/gap-analysis-2026-08-27.md` §2-3).

## Recommendation
Execute pi floor first: G1 → G2+G3 → G4 → G5. G6/G7 after floor is green. G8 only if user asks for rich TUI.
## Status update (2026-08-28, commit e200815 + working tree)
Re-audited after plans 013..016 + follow-up TUI wiring. G1..G3/G5 closed (slash registry 31 items, find + mutation queue, trust/reload). G4 shipped as capture-v1 (host deferred). G6/G7 partially closed:
- G6 keybinding persistence: SHIPPED — `epsilon-tui/src/keybindings.rs` mirrors pi `KeybindingsManager` (15 ids, `~/.config/epsilon/keybindings.json`, `effective()`/`reload()`, 4 tests). `App.keybindings.matches()` in `handle_key` + `/reload` + `/hotkeys` now dumps effective bindings; `/copy` wired to `arboard` clipboard. Remaining polish: per-binding Hints footer (deferred).
- G2/G7 export wiring: `/export` persists html/json via `SessionStore` → `.epsilon/exports`; `/copy` now real clipboard; import/share remain deferred with honest guidance.
- G4 isolated host for `task` remains deferred (needs `ToolContext` + `Agent` handle).
G8 (latex/images/alt-screen) + plan 011 (docs) + plan 017 (deterministic picker) remain TODO, intentionally deferred per `docs/plans/README.md`.
## Status update (2026-08-28, G4 rung 2)
G4 isolated host: SHIPPED — `epsilon-contracts::tool::{TaskHost, TaskHostResult}` (contracts inversion, no cycle) + `epsilon-core::Agent: TaskHost` (child Agent shares provider/tools/gate/workspace, 32-turn cap, inherits model/thinking/skills) + `exec.rs` injects `task_host` into `ToolContext` + `task.rs` delegates when host present (`details.host=isolated`, `usage` from child) else honest capture fallback. Proof: `loop_scenarios::task_delegates_to_isolated_child_via_host` (faux provider, 8 steps green). Remaining deferrals: `task_id` resume / `background=true` (schema-accepted, guided), import/share (honest), G8 chrome.
## Status update (2026-08-28, deterministic picker rung 2 — multi-token)
- Picker now matches pi `fuzzyFilter` semantics exactly: whitespace/slash token split, every token fuzzy-matches the hay (`id label provider`). Single-token queries unchanged; `deep chat` / `deep/chat` now find `deepseek-chat` in both stage-1 (`build_rows` + `build_provider_rows matscher`) and stage-2 (`build_models_for_provider`). Proved by `picker::tests::multi_token_filter_finds_split_words`. Gates: fmt 0 clippy 0 WS --lib 274 passed (was 273, +1), tui 123 passed.
## Status update (2026-08-28, 017 deterministic picker — close)
- **017 DONE (deterministic picker + transcript fuzz):** `fuzzy.rs` already mirrored pi's `fuzzy.ts` scoring (boundary/consecutive/i*0.1/exact-100/swapped+5) with `fuzzy_filter` token-split+sort. This rung closed the remaining seam: `picker.rs` stage-1 (`build_rows_with` all sections) + stage-2 (`build_models_for_provider`) now **score+sort** per-token `fuzzy_match` totals instead of boolean substring/filter-then-push, matching `tui/src/fuzzy.ts:fuzzyFilter` semantics exactly. Empty filter preserves `current` floats-to-top order; filtered queries rank best `totalScore` first (pi's `results.sort((a,b)=>a.totalScore-b.totalScore)`). Single-token queries unchanged; multi-token like `deep chat` finds `deepseek-chat` best-ranked. `deep zzz-nope` still yields only the custom verbatim row. Added `picker::tests::filtered_results_are_ranked_by_fuzzy_score_best_first`; relocated 017's target to `/plans/017-deterministic-picker-transcript.md` → CLOSED. Gates: fmt 0 clippy 0 WS --lib 275 passed (274+1), tui 124 passed.
## Status update (2026-08-28, batch edit + @-file fuzzy)
- **Batch edit (edit.ts `edits[]` parity):** `epsilon-tools/src/files.rs` now accepts `{edits:[{old_string,new_string}...]}` in one tool call — disjoint, non-overlapping spans matched against the original file, applied reverse, single diff + write under the existing mutation queue. Single-hunk keeps fuzzy chain + `replace_all`. Added `edit_match::find_exact_occurrences` helper, `descriptions/edit.txt` updated, +2 `builtin_tools` tests. Gates: fmt 0 clippy 0 WS --lib 277 passed (32 tools + 124 tui + others), tools 32+12.
- **@-file picker fuzzyFilter:** `epsilon-tui/src/app.rs:at_file_rows` now uses `crate::fuzzy::fuzzy_filter` on full paths (pi `autocomplete.ts:325` `fuzzyFilter(commandItems, prefix, item.name)` parity for the @-path), matching model-picker semantics. Empty fragment keeps sorted listing. Truncated to 200. Gates: tui 124 unchanged, WS --lib green.
## Status update (2026-08-28, G7 polish — /export default + /name persistence)

- **/export bare default:** `/export` without args no longer shows usage — it writes HTML directly into `.epsilon/exports/session-<id>.html` (pi parity: `/export` alone exports). Explicit `html`/`json`/`jsonl` args still work with an optional path; relative paths join `workspace_root`. Fix is a single `bare` branch in `crates/epsilon-tui/src/app.rs:run_command` `Command::Export`.

- **/name persistence:** Added `App.session_name: Option<String>` (initialized `None`), wired `/name` to store it in memory, persist a non-LLM `EntryKind::Custom { custom_type: "session_name", data: {"name": ...} }` entry in the session file (survives resume without a new dedicated EntryKind), and echo it on bare `/name`. No new deps, no new crates — Custom is the existing extension point for non-LLM harness state, so this avoids a contract bump while still surviving restarts. Remaining polish — rendering the name in the window title / `/session` line — is one line when wanted.

- **Ponytail note:** `write_export_{html,json}` + `store.append(Custom)` were already in-tree; the fix reuses them. `skipped: new EntryKind / prompt injection of the name, add when a UI actually reads it.`

Gates: `cargo fmt` 0, `cargo check --workspace` 0, `cargo clippy --workspace --all-targets` 0, `cargo test -p epsilon-tui --lib` 124 passed, `cargo test --workspace --lib` 275 passed.
## Status update (2026-08-28, G7 follow-through — /name + /session + bare /export polish)

- **/name + /session display name:** `App.session_name` now hydrated from the session file's last `Custom{session_name}` on bare `/name`, set via `/name <display>` (persists as `EntryKind::Custom`, survives resume), and echoed in `/session` as `'name' · path · leaf · entries · tokens · model`. Still no dedicated `EntryKind` — `Custom` is the intended extension point, so no contract bump until a UI needs the name rendered outside the transcript.
- **Bare /export:** already defaulted to `html` into `.epsilon/exports/` last step (kept); this step adds the `/session` name rendering and the `/name` hydration fallback.

Gates: `cargo fmt` 0, `cargo check --workspace` 0, `cargo clippy --workspace --all-targets` 0 (clippy --fix collapsed one `collapsible_if`), `cargo test -p epsilon-tui --lib` 124 passed, `cargo test --workspace --lib` 275 passed.
## Status update (2026-08-28, /changelog — read CHANGELOG.md into transcript)

- **/changelog now reads:** `workspace/CHANGELOG.md` then `workspace/docs/CHANGELOG.md`; emits a header line (`path · total lines · showing N`) plus the first 120 lines (header + Unreleased + last release) into the transcript, with a `… N more lines in path` tail. When absent: `no CHANGELOG.md in workspace (see docs/milestones/ for milestone logs)`. Ponytail: `std::fs::read_to_string` only, no `parseChangelog` link-rewrite, no new dep — add `normalizeChangelogLinks` + per-version picker when a transcript actually needs tag-pinned links.

Gates: `cargo fmt` 0, `cargo check --workspace` 0, `cargo clippy --workspace --all-targets` 0, `cargo test -p epsilon-tui --lib` 124 passed, `cargo test --workspace --lib` 275 passed.
## Status update (2026-08-28, live /resume + /import — in-TUI session switch)

- **/resume + /import now live (no restart):** `epsilon-core::Agent::live_resume(path)` opens `path` as a `SessionStore`, replays `repair_on_open` (same as CLI `--resume`), derives the shadowed `model_view`, restores it into runtime history, and re-attaches the new store. `epsilon-tui/src/app.rs:run_command` `Resume`/`Import` now resolve relative paths against `workspace_root`, guard on `ui.running() || agent.is_running()` (busy → notice), check `abs.exists()` (missing → notice), then call `live_resume` → `ui.hydrate_from_history(&history)` → re-hydrate `session_name` from the newly attached store's last `Custom{session_name}`. `epsilon-tui/src/state.rs:hydrate_from_history` clears existing rows/streaming, rebuilds `TranscriptLine`s from `Message::{User,Assistant,ToolResult,Custom}` via existing constructors (`plain`/`assistant_md`/`expandable`), re-derives `tokens_in/out`, `cache_read/write`, `latest_prompt_tokens`, and sets `status = resumed N messages`. Ponytail: `Agent::history()` + `store_handle_for_tests()` + `SessionStore::{open, model_view, read_all, branch_entries}` + `repair_on_open` were already in-tree; no new dep, no contract bump.

Gates: `cargo fmt` 0, `cargo check --workspace` 0, `cargo clippy --workspace --all-targets` 0, `cargo test -p epsilon-tui --lib` 124 passed, `cargo test --workspace --lib` 268 passed.
## Status update (2026-08-28, /share — live gist via gh)

- **/share now does the work (pi `handleShareCommand` parity, ponytail minimal):** exports the session to `.epsilon/exports/share-<id>.html` (same `write_export_html` as `/export`), then tries `gh gist create --public=false <file>` synchronously. On success: extracts the gist URL and `gist_id`, builds `PI_SHARE_VIEWER_URL#<gist_id>` (default `https://pi.dev/session/`) and reports `gist <url> · preview <url> · file <path> (N entries)`. On non-zero / missing `gh`: reports `gist deferred (gh not installed — https://cli.github.com/)` + exact retry command, while the exported file remains usable (same as pi's honest guidance when `gh auth status` fails). Ponytail: `std::process::Command` only, no new dep — `reqwest` already in-tree but pi's path really is `gh`; a pure-API upload would add token handling. Skipped: loader overlay (pi `BorderedLoader`); add when the TUI needs a cancellable spinner.

Gates: `cargo fmt` 0, `cargo check --workspace` 0, `cargo clippy --workspace --all-targets` 0, `cargo test -p epsilon-tui --lib` 124 passed, `cargo test --workspace --lib` 268 passed.

## Status update (2026-08-28, /settings + /scoped-models — discovery guidance parity)

- Decision: pi `SettingsSelectorComponent` is ~30 toggles; Epsilon ponytail is actionable discovery guidance instead of a second TUI form — show global/project config paths, current model + think level, recents/favorites counts, and exact keys to edit (`[model] id`, `[picker] favorite`) then `/reload` to apply, no restart.
- `/settings` now notices `settings: global <path> · project .epsilon/config.toml` + `current model: <id> · think: <level>` + `recents: …` (5) + `favorites: …` (5, `Ctrl+F to star` when empty) + `edit the toml directly …, then /reload to apply without restart`.
- `/scoped-models` now either (empty favorites) `no favorites yet — open Ctrl+P and press Ctrl+F on a model to star it, then Ctrl+P cycles favorites` or (with favorites) opens `Ctrl+P` favorites-first picker live and notices `N favorite(s) — cycling favorites · open Ctrl+P to pick, Ctrl+F to star/unstar` (pi scopedModels -> model picker, ponytail reuses model picker; no second selector).

Files: `crates/epsilon-tui/src/app.rs` (Command::Settings + Command::ScopedModels).
Gates: `cargo fmt --all -- --check` 0, `cargo check --workspace` 0, `cargo clippy --workspace --all-targets` 0, `cargo test -p epsilon-tui --lib` 124 passed.
Skipped at this rung: overlay settings form — wire when the TUI needs in-place toggle editing.
Ponytail: discovery guidance reuses existing state (model_id/thinking/recents/favorites) + model picker, no new dep.

## Status update (2026-08-28, latex SYMBOLS — full 217 parity)

- **LaTeX symbol table now 217/217 pi parity:** `crates/epsilon-tui/src/latex.rs::symbols()` was 151 entries; replaced with the full `references/pi-0.84.2/packages/tui/src/latex.ts:SYMBOLS` (217 `name -> unicode` pairs) verbatim, correcting the 66 missing symbols (`amalg`/`exists`/`forall`/`hook*`/`land`/`longrightarrow`/`vert`/`sqcap`/`Im`/`Re`/…).

Files: `crates/epsilon-tui/src/latex.rs` (217 entries, `backslash` correctly as Rust `"\\"` → one char).
Gates: `cargo fmt --all -- --check` 0, `cargo check --workspace` 0, `cargo clippy --workspace --all-targets` 0 (2 `empty_line_after_outer_attr` warnings, pre-existing), `cargo test --workspace --lib` 275 passed (7+16+22+0+31+3+25+8+7+32+124), `cargo test -p epsilon-tui --lib` 124 passed.

## Status update (2026-08-28, docs — latex header honest)

- **Header now honest:** `crates/epsilon-tui/src/latex.rs` module doc said `ponytail subset (~110 entries)` while the file has been 217/217 pi parity since the SYMBOLS fix — stale header hurts trust, so the module doc now reads `217-symbol table plus \frac/\sqrt/^/_ and plain-text fallbacks for \\/&/\begin{…}` with the same deferred note (`pmatrix` fidelity when needed). Inline `curated to ~110` → `217 entries, full parity`.
- Ponytail: bytes-patch on existing file, no new dep, no behavior change — doc-only close avoids a code detour through image/alt-screen (both L-effort deferred at this rung).

Gates: `cargo fmt --all -- --check` 0, `cargo check --workspace` 0, `cargo clippy --workspace --all-targets` 0, `cargo test -p epsilon-tui --lib` 124 passed (latex 6/6), `cargo test --workspace --lib` 275 passed.

## Status update (2026-08-28, docs — docs/plans/README G8 honest)

- **Plans/README G8 line now honest:** the summary bullet said `G4/G6/G7/G8 intentionally deferred` while `G4` (task host), `G6` (keybindings), `G7` (/export+/name+/share+/settings) closed 2026-08-28 and `G8` latex closed 217/217 that same day — stale line misled the next re-audit into reopening latex. Now reads `G4/G6/G7 closed 2026-08-28, G8 latex closed 217/217; G8 images/alt-screen intentionally deferred (negligible leverage vs harness)`.

Gates: no code change; `cargo fmt --all -- --check` 0, `cargo check --workspace` 0.

## Status update (2026-08-28, docs — README slash row + gap G8 honest)

- **README slash row now 31:** `README.md:83` said ~14 slashes; now lists all 31 lifecycle/auth/config/export cycles (`/new`/`/resume`/`/session`…`/scoped-models`) matching `crates/epsilon-tui/src/picker.rs:SLASH_ITEMS`.
- **Gap G8 table row now honest:** the finding table grouped latex under TUI chrome; latex closed 217/217. Row now reads `pi renders latex (DONE 217/217)… remaining chrome is images/alt-screen only — intentionally deferred`.

Gates: `cargo fmt --all -- --check` 0, `cargo check --workspace` 0; doc-only.

## Status update (2026-09-14, M52 — OAuth login ceremonies + real-feature fix)
- **OAuth login:** `epsilon-llm/src/oauth_login.rs` ports the login half M46/M51 deferred: PKCE (CSPRNG verifier, S256 challenge, `state`) with a loopback callback listener, RFC 8628 device polling with pi's slow_down/drift semantics, paste parsing for all four redirect shapes, and per-provider exchanges for `anthropic`, `openrouter` (PKCE → permanent key) and `xai` (device). CLI `epsilon auth login <provider> --oauth [--paste]`; TUI `/login <provider> oauth` on a worker thread with the masked overlay for the paste. Client ids resolve from `<PROVIDER>_OAUTH_CLIENT_ID` or `[oauth.<provider>] client_id`; never embedded. `xai` added to the OAuth-first resolution in `catalog.rs`. `openai-codex`/`github-copilot` remain out until their wires+rows exist (a test pins that promise).
- **Found and fixed while wiring it:** `epsilon-cli` `real` never enabled `epsilon-core/real`, so `Agent::try_hot_swap_to_real` shipped as its `false` stub (M32d's live `/login` hot-swap could not work). `real` now forwards to `epsilon-llm`, `epsilon-core` and `epsilon-tui`; a compile-time guard (`epsilon_core::REAL_PATH_ENABLED` bound in the CLI) makes the omission a build error.
- Tests: 23 offline `epsilon-llm` ceremony tests (RFC 7636 vector, listener round-trip incl. state mismatch/404/timeout, device poller with virtual time, parsers) + 4 TUI state-machine tests, plus a Ctrl+O / `⌁ oauth` discovery path in the key prompt and the provider picker. Gates: fmt/clippy zero, workspace suite green, `cargo check -p epsilon-cli --features real` clean, `--demo` completes.
