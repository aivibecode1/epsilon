# Provenance

Filed 2026-10-15 as part of wave 19 (`19-settings-changes-2026-10-15.md`, which holds the verdicts). This is the audit child's report as written to scratch during delegation `deleg_ade17047`, unedited except for this header. Repo-side claims in it are leads until `19-...` says re-verified.

---

# Epsilon vs pi + DSH: settings / capability UX audit

Date: 2026-10-15. Method: read-only. Claims about a REFERENCE cite a path actually
read (terminal/grep output counts as read and is marked source-traced). Claims about
EPSILON cite an epsilon file:line actually opened. Anything else is marked inferred.
Epsilon HEAD: 9258db1 + dirty /changes stack (command.rs, config.rs, runtime lib.rs,
session store, checkpoint, tui app/commands/session/view, ui/theme).

Invariants framing everything (Epsilon `AGENTS.md:25,43-48`, opened): **one core**
(the agent loop is `epsilon-core`'s `Agent`; every mode rides the same machinery) and
**one composition point** (`epsilon-runtime` is the only place config becomes a running
agent). A recommended adoption must therefore live in core/tools and be wired once in
`epsilon-runtime`, never as a TUI-only feature.

---

## 1. Settings / configuration UX: pi vs Epsilon

### 1a. Pi's settings machine (source-traced)

- Generic component `packages/tui/src/components/settings-list.ts:7-24`: `SettingItem`
  `{id, label, description?, currentValue, values?, submenu?}`. Enter/Space cycles
  `values` or opens `submenu` (`:264-292` `activateItem`); submenu completion calls back
  with optional `navigateTo` (`:271-283`). Search filter optional (`:35-48,70-72`),
  scroll window (`:256-262`), selected-row description + hint line (`:163-174`).
- The catalogue built on it,
  `packages/coding-agent/src/modes/interactive/components/settings-selector.ts`,
  carries **~35 rows** (ids grepped, source-traced): `autocompact` (:482),
  `steering-mode` (:489), `follow-up-mode` (:497), `transport` (:504),
  `http-idle-timeout` (:511), `cache-warming-mode` (:519), `hide-thinking` (:527),
  `mermaid-rendering` (:534), `cache-miss-notices` (:541), `collapse-changelog` (:548),
  `quiet-startup` (:555), `install-telemetry` (:562), `default-project-trust` (:569),
  `double-escape-action` (:576), `tree-filter-mode` (:583), `warnings` submenu (:590),
  `model-thinking` stepped per-model submenu (:605-704), `tui-mode` (:706),
  `fullscreen-exit-output` (:713), `fullscreen-scrollbar` (:720),
  `fullscreen-copy-on-select` (:727), `fullscreen-wheel-scroll-lines` (:734), `theme`
  with `ThemeSubmenu` (:748-754), image rows `show-images`/`image-width-cells`/
  `auto-resize-images`/`block-images` (:761-788), `skill-commands` (:798),
  `show-hardware-cursor` (:808), `editor-padding` (:818), `output-padding` (:828),
  `autocomplete-max-visible` (:838), `clear-on-shrink` (:848), `terminal-progress`
  (:858); plus the auto-theme submenu `light-theme`/`dark-theme`/`apply`/`single-mode`
  (:343-388) and the warnings submenu `anthropic-extra-usage` (:155-163).
- Runtime change paths (`interactive-mode.ts`, all source-traced): `/settings`
  `:3146-3149` opens the selector; `/model [term]` `:3156-3161` opens the model
  selector with optional search; `/thinking [level]` `:3162-3167`; `/tree` `:3224-3228`
  session-branch selector; `/trust` `:3229-3232` project-trust selector.
  Builtins total **24** (`core/slash-commands.ts:19-44`).
- Model/thinking/theme/permissions at runtime:
  - Thinking levels `off/minimal/low/medium/high/xhigh/max` with token-budget
    descriptions (`settings-selector.ts:39-47`), settable globally, cyclable in-session,
    **and overridable per provider/model** via the `model-thinking` stepped submenu
    (`:605-704`, callbacks `onModelThinkingLevelChange/Remove` at `:115-116`).
  - Theme row opens `ThemeSubmenu` with a **live preview callback**
    (`SettingsCallbacks.onThemePreview?`, `:118`), a terminal-derived `SYSTEM_THEME_NAME`
    (`:208-214`), and **automatic light/dark pair mode** (`:219-232`).
  - Permissions = project trust: `/trust` selector + `default-project-trust`
    ask/always/never fallback (`:569-574`). No privilege-escalation gate; trust is a
    saved per-project decision, not a posture with a confirm step.

### 1b. Epsilon's settings machine (source-traced, opened)

- `crates/epsilon-tui/src/app/commands.rs:1984-2006` `setting_rows` returns **5 rows**:
  model / thinking / theme / thinking-blocks / permission. `apply_setting` `:2011-2071`:
  model reopens the picker (`:2013-2020`), thinking opens the think picker
  (`:2021-2027`), **theme cycles immediately** to the next palette and persists via
  `save_tui_theme_in` (`:2028-2042`), thinking-blocks toggles `hide_thinking` and
  persists (`:2043-2058`), permission cycles `next_preset` and routes through
  `run_preset_command` (`:2059-2068`), which gates privilege-raising switches behind
  `pending_preset` yes/no (`:1411-1444`, raise-only per D-080).
- Thinking levels are the same 7 (`crates/epsilon-contracts/src/message.rs:24-58`,
  `Off..Max`) and Epsilon additionally remembers **per-favorite** thinking
  (`app.rs:260-264` `[picker.favorite_thinking]`, save path `:791-816`) and mirrors the
  live level into the footer (`app.rs:576-579`; STATUS `e4c8d81` row). Theme is **3
  palettes** (`ui/theme.rs:161-163` dark/light/high-contrast), no preview, no auto pair.
- Command registry is the opposite story: `COMMAND_HEADS`
  (`epsilon-contracts/src/command.rs:429-...`, opened) carries **~40 heads** vs pi's 24
  — restore, changes, tasks, preset, skills, catalog, help/login etc. — with a
  reverse-drift check and per-mode capability as data (AGENTS.md invariant 4). The
  breadth gap is in the *settings panel*, not the command set.

### 1c. Where Epsilon is missing or inferior (ranked inside §4)

1. **Per-model thinking defaults** — pi `model-thinking` submenu
   (`settings-selector.ts:605-704`) vs Epsilon per-*favorite* memory only
   (`app.rs:791-816`). Same shape, narrower key: unstarred models have no default.
   Small adoptable shape (extend the favorite-thinking store key to provider/model).
2. **Theme UX** — pi preview + system theme + auto light/dark pair
   (`settings-selector.ts:208-232,748-754`) vs Epsilon blind 3-cycle
   (`commands.rs:2028-2042`, `theme.rs:161-163`). Medium shape; preview first, auto
   pair second.
3. **Panel visibility of existing behavior** — auto-compact runs ("loop also
   auto-compacts on pressure", `commands.rs:1863`) but has no row; pi has an
   `autocompact` row (`settings-selector.ts:482`). Tiny shape: one row over existing
   state. (Capability presence itself is source-traced; whether a config key exists
   behind it is unopened — panel-only claim.)
4. **Steering / follow-up queue modes** — pi core (`agent-session.ts:200-201,303-304,
   1588-1595,2156-2185`, grep-traced) with settings rows (`settings-selector.ts:489-502`).
   Epsilon panel has no equivalent; whether the core has the queue is unopened
   (inferred gap at panel level only — do not claim a core gap).
5. Deliberately not recommended: transport/idle-timeout/cache-warming (provider
   plumbing, low user value), fullscreen rows (Epsilon is fullscreen-only — inferred),
   mermaid/images/editor-padding (display prefs, low value), warning toggles.
6. Where Epsilon is **ahead** (do not "adopt" backwards): permission presets with a
   raise-only confirm gate (`commands.rs:1411-2044`) vs pi trust ask/always/never;
   one-registry commands (AGENTS.md invariant 4); footer names the live model +
   thinking persists across resume (STATUS `e4c8d81`).

---

## 2. Question tool / plan mode / changes panel

### 2a. Pi question tool: an optional example extension, blocking-only, TUI-only

- `packages/coding-agent/examples/extensions/questionnaire.ts:84-92` registers tool
  `questionnaire` (multi-question, options + "type something", tab UI). **Not core**:
  lives under `examples/extensions/` (README lists it as a sample at `:36`).
- Hard TUI-only guard: non-TUI mode gets `"Error: UI not available"` (`:93-95`).
  No timed/deferred answering, no headless/RPC answerer, no schema/answerer split.

### 2b. Pi plan mode: also an example extension, enforcement-thin

- `packages/coding-agent/examples/extensions/plan-mode/index.ts:1-30` header:
  `/plan` command **or Ctrl+Alt+P toggle**, read-only exploration; `:22`
  `PLAN_MODE_TOOLS = ["read","bash","grep","find","ls","questionnaire"]`,
  edit/write disabled, bash restricted to an allowlist (`isSafeCommand`, covered by
  `test/plan-mode-utils.test.ts`), plan-step extraction + `[DONE:n]` + progress widget.
- Enforcement sits at the extension's tool list, not in a permission engine; nothing
  stops project rules or approvals from re-allowing, and there is no reviewed-exit
  handshake in this extension (no `exit_plan_mode`; the DSH one below is the shape
  that has it).

### 2c. Epsilon status on all three (source-traced)

- **Question tool: absent.** `crates/epsilon-tools/src/` lists checkpoint, edit_match,
  files, find, goal, goal_gate, image, lib, mutation, read_log, search, shell, skill,
  task, todo, util, webfetch — **no ask/question tool** (dir listing read). STATUS
  ranks it explicitly as reference capability still below the line (`STATUS.md`
  "reference capabilities ranked below the permission diff (`changes` panel, the
  question tool, plan mode)").
- **Plan mode: enforcement-strong, surface-thin.** `--plan` is a launch flag
  (`epsilon-cli/src/main.rs:103,196-200,355-356,482`, opened): write/edit and
  bash/shell denied outright. Composition in `epsilon-runtime/src/lib.rs:5029-5043`
  (opened) prepends `plan_ruleset()` and relies on **monotonic-deny**, so the boundary
  holds regardless of project rules or approvals — strictly stronger than pi's
  extension-level gating. But it is **launch-only**: no `plan` head exists in
  `COMMAND_HEADS` (`command.rs:429-...`, opened full list), and a grep for `plan`
  across `epsilon-tui/src` returns only comments/unrelated words (source-traced via
  grep) — no toggle, no indicator, no reviewed exit. Documented first in `32947dd`
  ("plan mode working and invisible", STATUS `:93-94`) — docs closed the invisibility,
  not a surface.
- **Changes panel: Epsilon leads, not lags.** `/changes` arm (`commands.rs:1919-1921`),
  `open_changes` session/workspace scopes (`:1957-1981`), `/restore <path> yes`
  two-press confirm with deliberate no-diff ceiling (`:2073+` doc block), checkpoint
  store in `epsilon-tools/src/checkpoint.rs`. Pi has **no equivalent**: grep over
  pi `coding-agent/src` for restore/checkpoint/changes/revert finds only sandbox-env
  restore, transcript tool-loadout restore, and compaction summaries (grep-traced) —
  file recovery in pi means git + the per-edit diff renderer (`core/tools/edit-diff.ts`,
  ls-traced) + `/tree` session branches + `/export`. The dirty-stack work extends a
  lead; adopt nothing from pi here.

---

## 3. DSH: one capability worth adopting

Surfaces read (READMEs opened): `interaction/` package map (slash commands without a
model round trip; one-shot approvals failing closed; presets bundling sandbox+approval;
the question/answer seam), `tool-ask-user` (blocking default; **`mode: timed`** lets a
deadline release the model while the question stays answerable; composes with
`ctx.userQuestions`; **a live child agent cannot call it**; callers provide the answer
UI), `plan/plan-mode` (`/plan`, `/plan <msg>` with attachments, `/plan off`;
deployment guidance rendered as a `plan:policy` prompt section; finished plan presented
for approval via **`exit_plan_mode`**; survives resume/forks; explicitly does **not**
restrict tools — sandbox+approval do that), `permission-presets` (preset table bundling
sandbox+approval; `custom` shown-but-unselectable; selection owned separately from
execution).

- **Adopt: `ask_user_question` (blocking first, timed deferred).** Why this one:
  (a) STATUS already ranks the question tool as wanted, so it has owner demand; (b) it
  improves on *both* references — pi's questionnaire is extension-only/TUI-only/
  blocking-only, DSH adds the validated schema + answerer waterfall + timed mode + an
  explicit non-delegation rule; (c) it fits the invariants with the smallest shape —
  **one core**: a native tool in `epsilon-tools` awaited by the `epsilon-core` loop, so
  TUI, headless, HTTP/SSE and RPC inherit it identically (AGENTS.md:43-45); **one
  composition point**: `epsilon-runtime` wires the answerer (TUI custom UI in the
  spirit of pi's `ctx.ui.custom`, RPC/HTTP answerers in theirs), exactly one wiring
  site (AGENTS.md:46-48). Fail closed with no answerer (DSH: "blocking calls fail";
  matches Epsilon's deny-default gates), keep the live-child restriction, defer timed
  mode to a second slice.
- **Do not adopt now:** DSH permission-presets (Epsilon's 4 presets + raise-gate +
  RulesetGate layering already cover the user-visible shape; DSH's marginal additions
  are `custom`-detection and catalog-driven defaults — lower value, larger shape);
  DSH plan-mode reviewed exit (valuable but second: it needs the question/approval seam
  anyway, so it composes *on top of* the ask tool — sequence it after).

---

## 4. Ranked gaps (user value × smallest adoptable shape)

| Rank | Gap | Value | Shape | Notes |
|---|---|---|---|---|
| 1 | `ask_user_question` tool, blocking first | Highest — unblocks mid-run clarification in **every** mode | Small — one `epsilon-tools` tool + answerer trait + single `epsilon-runtime` wiring + per-mode UI | DSH seam; pi questionnaire proves the TUI half; timed mode deferred |
| 2 | Plan-mode reviewed exit (`/plan` toggle + `exit_plan_mode`-style approval + visible indicator) on top of the `--plan` deny layer | High — safe exploration with a decision point | Medium — prompt section + toggle + review handshake; enforcement already exists (`lib.rs:5029-2043`) | DSH plan-mode shape; pi extension shows the toggle/todo half only |
| 3 | Per-model thinking defaults (widen favorite-thinking key to provider/model) | High for multi-model users | Small — store + settings-row/submenu extension | Pi `model-thinking` submenu is the spec |
| 4 | Theme preview (+ auto light/dark pair later) | Medium — daily comfort, error prevention | Medium — preview callback in the theme row; palettes stay 3 | Pi `ThemeSubmenu` is the spec |
| 5 | `autocompact` row over existing behavior | Low-medium, near-free visibility | Tiny — one row, no new machinery | Pi row `:482`; Epsilon behavior exists (`commands.rs:1863`) |
| — | Changes panel | Epsilon ahead; keep extending the lead | — | No pi/DSH adoption here |

Caveats (inferred, not traced): whether Epsilon core has steering/follow-up queues;
whether an autocompact config key backs the behavior; exact TUI plan indicator absence
beyond grep + heads list. Verify before building rows 3–5; row 1 needs no such check.
