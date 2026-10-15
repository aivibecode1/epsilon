# Provenance

Filed 2026-10-15 as part of wave 19 (`19-settings-changes-2026-10-15.md`, which holds the verdicts). This is the audit child's report as written to scratch during delegation `deleg_ade17047`, unedited except for this header. Repo-side claims in it are leads until `19-...` says re-verified.

---

# Epsilon TUI settings/config workflows — file-edit workarounds & persistence defects

Working tree on disk: branch `main`, HEAD `9258db1`, plus 11 dirty files (the dirty stack adds
`/changes` + the `/settings` panel + `ThemeMode::next` + `save_tui_theme_in`).
READ-ONLY audit: no edits, no cargo. Every claim below is labeled
**read** (I opened the file and quote it) or **inferred** (my conclusion from read facts).
Anchor status (moved / as-described) is noted per item.

Verdicts: **LIVE** = real defect today · **STALE** = fixed or mis-described · **PARTIAL** = half-fixed.

---

## 0. Headline: the dirty `/settings` stack does not compile as it stands (LIVE)

Three undefined references, each verified by exhaustive repo-wide search (read):

- `apply_setting` arm 4 calls `next_preset(&self.settings_preset)`
  (`crates/epsilon-tui/src/app/commands.rs:2060`, read) — repo-wide search for
  `next_preset` returns **exactly 1 hit in the whole repo: the call site**. No definition
  in any crate, no import. (read: search total_count=1)
- `app.rs:1930-1931` (dirty, read via `git diff`) imports `draw_settings` and `draw_changes`
  from `panels`, and `app.rs:2125-2126` calls `draw_changes(...)` — but `panels.rs`
  defines 14 `draw_*` fns and **neither** (read: enumerated all 14).
- `keys.rs` has **zero** references to `settings_open`/`changes_open` (read: total_count=0),
  so neither panel has input wiring (no ↑/↓, no Enter→`apply_setting`, no Esc).
  The established pattern exists (`keys.rs:1127-1156`, think-picker arms, read) but was not
  extended. `overlay_open` (`view.rs:576-590`, read) does include both flags, so an opened
  panel would swallow keys with no handler to answer them.

Consequence: `setting_rows`/`apply_setting` (commands.rs:1984-2071, dirty, read) are
reviewable logic but **not runnable**. All row verdicts below are against the written logic,
not live behavior. Smallest fix shape: land the missing `draw_settings`/`draw_changes`,
`next_preset`, and the two `keys.rs` arms (one build-owner commit; estimate ~150 lines).

---

## 1. Settings rows, one by one (dirty `setting_rows`/`apply_setting`, read)

### Row 0 — model → opens picker → `apply_model` (app.rs:1410-1467, read). PARTIAL (good write, restart-read unverified)
- Enter changes live state (`agent.set_model_id`, `self.model_id`), persists via
  `project_dir_or_global()` → `save_model_id_in`, and reports honestly on all three
  outcomes: `"model switched to {id} — saved to {path}"` / `"(not saved: {e})"` /
  `"(not saved: no config directory resolved)"` (read).
- Save errors do not roll back live state — deliberate and disclosed in-code (read).
- Restart survival: startup `default_model` reads **global only**
  (`session.rs:93`: `default_model: epsilon_core::config::load_model_id()`, read; anchor
  as-described, unmoved). The write goes to `<ws>/.epsilon/config.toml` when that dir exists
  (`project_dir_or_global`, app.rs:845-855, read). Whether the launch path resolves the
  project file for the *initial* model was **not** verified (CLI-side, out of opened files)
  → inferred gap, needs build-owner check. Same write/read asymmetry is *proven* for rows 2/3 (§2).

### Row 1 — thinking → opens think picker → `set_thinking` (app.rs:1249-1278, read). LIVE-ish → PARTIAL
- Persists global standing default with error notices (`"thinking level not saved: …"`, read);
  startup loads global + project composition outranks it (session.rs:180-187, read).
- BUT per-project thinking default is explicitly hand-edit-only:
  app.rs:1257-1264 *"A repo opting into its own effort still edits the file by hand,
  exactly as it does for `[model] id` today"* (read). Deliberate, documented → PARTIAL:
  the global default persists; the project default is a file-edit workaround by design.

### Row 2 — theme → `next()` cycle + `save_tui_theme_in`. LIVE defect (write/read mismatch)
- Live change + persist + honest error notice (`"theme is {}, not saved: {e}"`, commands.rs:2028-2042,
  read). `save_tui_theme_in` itself is sound read-modify-write preserving unknown keys
  (config.rs:707-725, dirty-added, read).
- **Defect:** the write target is `config_dir_override.or(global)` resolved through
  `project_dir_or_global()` semantics (project `.epsilon/` wins when present), while the
  **only startup read is global**: `resolve_theme_mode` → `load_config_string("tui","theme")`
  → `load_config_string_in(&global_dir()?)` (config.rs:32-34, read, HEAD-unmoved).
  In any workspace with `.epsilon/`, a settings-saved theme lands in a file nothing reads →
  silently lost on restart. Smallest fix: write theme (and row 3) to **global always**
  (2-line change: replace dir resolution with `epsilon_core::config::global_dir()`), or add a
  project-over-global read in `resolve_theme_mode`.
- Silent no-op edge (minor, read): when no dir resolves, `if let Some(dir)` skips with **no
  notice** — live palette changes, disk doesn't, user is told nothing. Same shape row 3.

### Row 3 — thinking blocks → toggle + `save_hide_thinking_in`. LIVE defect (same mismatch)
- Toggle-then-save with honest error notice (commands.rs:2043-2058, read); startup loads
  global only (session.rs:60: `ui.hide_thinking = load_hide_thinking()`, read, unmoved).
- Same project-vs-global mismatch as row 2 (write may go project, read is global-only).
  Same fix shape (pin to global).
- Same silent no-op when no dir resolves (minor).

### Row 4 — permission → `next_preset` + `run_preset_command`. LIVE (two defects)
- (a) `next_preset` is undefined (§0) — the row cannot run at all.
- (b) Even once defined: `run_preset_command` (commands.rs:1411-1445, read) only calls
  `agent.set_preset(...)` (in-memory gate; `epsilon-safety/src/gate.rs:225-240`, read —
  swaps the ruleset layer, clears session memory, **no disk write anywhere**). No config key
  for the preset exists; nothing loads one at startup (`settings_preset` inits to `""`,
  session.rs:204, read). So the permission choice is **session-only and never survives
  restart** — the one settings row with zero persistence. Smallest fix shape (medium):
  new `[permissions] preset` key + `save/load` fns in config.rs (copy the
  `save_hide_thinking_in` idiom, ~30 lines) + load at startup + write in
  `run_preset_command`; plus the missing `next_preset` cycle helper (~10 lines).
- The privilege-escalation confirm (`pending_preset` + yes/no, commands.rs:1425-1432, read)
  is preserved through the settings path (panel closes first when raising, :2061-2063, read).
  Good — keep.

## 2. Bare `/theme` vs settings-row theme (commands.rs:1823-1851, read, HEAD-unmoved). PARTIAL
- Bare `/theme` deliberately does **not** persist (*"a cosmetic toggle should not quietly
  rewrite a file"*, :1824-1827, read) while the settings row **does**. Both comments state
  the split, so it is designed — but the user-visible notice (`"palette: dark → light"`,
  :1849-1850) never says the change is session-only. Expectation bug: set via `/theme`,
  restart, silently reverted. Smallest fix: append `(session only — /settings theme to keep)`
  to the notice (1 line).
- Stale error text (LIVE, trivial): unknown palette says
  `"use \`light\` or \`dark\`, or bare \`/theme\` to toggle"` (:1833-1835, read) while
  `from_name` also accepts `high-contrast|contrast|hc` (theme.rs:149-156, read). 1-line fix.

## 3. File-edit workaround surfaces (Q2: notices telling the user to edit config)

- **Old `/settings` guidance — STALE (fixed in tree).** HEAD told users
  `"edit the toml directly (model: [model] id, favorites: [picker] favorite), then /reload
  to apply without restart"` (read via `git diff`, removed hunk). The dirty panel replaces
  it. Verdict STALE *conditional on the panel landing* — today HEAD still ships it and the
  panel doesn't compile, so the user-visible product still has the workaround.
- **`[editor] command` — LIVE (literal match).** `editor.rs:36` (opened, read):
  `Err("no editor configured — set $EDITOR or $VISUAL, or add\n    [editor]\n
  command = \"code --wait\"\nto ~/.config/epsilon/config.toml")`. No in-TUI setter exists
  (read: `load_config_string("editor","command")` is the only access, editor.rs:97-98 /
  session.rs:423-424). Low rank: legitimate fallback, but it is exactly the
  "tells-user-to-edit-config" shape. Smallest fix: accept `/set editor <cmd>` writing
  `[editor] command` via the standard table idiom (~20 lines + a save fn).
- **Keybindings — LIVE file-edit-only mutation (no notice points there).**
  `KeybindingsManager` exposes `load/reload/matches/key_text/effective` and **no
  save/rebind API** (read: full `pub fn` enumeration, keybindings.rs). `/hotkeys` shows
  effective bindings read-only (commands.rs:2500-2511, read); a remap requires hand-editing
  `keybindings.json` + the skills-reload command picks it up without restart
  (`keybindings.reload()`, commands.rs:1747-1753, read). No notice documents this path.
  Ranked medium: smallest honest fix is a `/hotkeys` footer line naming the file + reload;
  a real rebind UI is large and not proposed.
- **Custom providers `[provider_custom.<name>]` — LIVE file-edit-only.**
  Exactly one reader (`custom_providers_in`, config.rs:47-73, read) and **zero writers
  repo-wide** (read: writer-pattern search empty). Visible in picker (app.rs:268 customs
  threaded into rows, read) but creatable only by hand-editing config.toml. Same rank as
  keybindings; smallest fix is an error/notice path, full add-flow is medium.
- **OAuth `client_id`/`enterprise_domain` — file-only by design, not a defect.**
  Single reader (`oauth_config_in`, config.rs:86-114, read); no setter. Enterprise SSO
  material belongs in a file, not a TUI prompt. Not ranked as a fix; noted for completeness.

## 4. Model selector, auth flow, permission presets (Q3)

- **Model selector: no workaround.** Picker + `apply_model` persist in-TUI with honest
  save/fail notices (see §1-row 0). Recents push is best-effort global and explicitly
  decoupled from the mirror (app.rs:1468-1500 comment, read). No file-edit instruction
  anywhere on this path.
- **Auth flow: no workaround, best-in-class.** Bare `/login` provider picker
  (commands.rs:840-852, read), masked in-TUI key prompt (853-868, read), OAuth ceremony
  on worker thread (app.rs:1339-1399, read), all persisting to `auth.json` with
  saved/not-saved notices (869-906, read). Nothing tells the user to hand-edit credentials.
- **Permission presets: session-only, no persistence (LIVE)** — see §1-row 4(b). This is the
  Q3 outlier: model/auth persist, permissions don't.

## 5. Keybindings display (commands.rs:~2500, anchor moved +12 lines by dirty hunk, read at 2500-2511)
As-described, at 2500 not 2488 (dirty `/changes` hunk shifted lines). Effective-bindings
listing works (sorted, read). No defect in display; the defect is mutation (§3).

## 6. Config load/save layer (config.rs, read whole file)
All writers use the same read-modify-write-table idiom preserving unknown keys
(`save_model_id_in:179, toggle_favorite:284, save_hide_thinking_in:607,
save_thinking_level_in:667, save_tui_theme_in:707-dirty, set_skill_disabled_in:426` —
all read). Reads are fail-open (None/empty/default, never fatal). No writer defect found;
the defects are all *which dir* the TUI passes in (§1 rows 2-4) and *absent* writers
(preset, keybindings, custom providers, editor).

## 7. Skills (`/skill on|off`, commands.rs:956-996, read). No defect
Validates existence, persists via `set_skill_disabled` (global), honest error notice.
In-TUI end to end. (Anchor: was "line ~958 area" — unmoved by dirty stack; the Skill arm
predates it.)

---

## Ranked findings (user value × fix size)

1. **Settings theme + thinking-blocks rows write project-dir, startup reads global-only** —
   LIVE, silent loss on restart in any workspace with `.epsilon/`. Fix: pin both rows to
   `global_dir()` (2 lines). (read both ends)
2. **`/settings` panel doesn't compile/run** (`next_preset`, `draw_settings`,
   `draw_changes` undefined; no `keys.rs` wiring) — LIVE, blocks the whole STALE verdict in
   §3 from reaching users. Fix: one build-owner commit (~150 lines). (read)
3. **Permission preset never persists (session-only)** — LIVE. Fix: medium (new config key +
   startup load + write in `run_preset_command` + `next_preset`). (read)
4. **Bare `/theme` silently session-only; error text omits high-contrast** — LIVE/trivial.
   Fix: 2 one-line notice texts. (read)
5. **Keybindings remap & custom-provider add are file-edit-only with no notice** — LIVE,
   low-medium. Fix (small): footer/hint lines naming file + reload; full in-TUI flows not
   proposed. (read)
6. **Per-project thinking/model defaults partly hand-edit-only** — PARTIAL by design
   (documented comments, read). No fix proposed beyond documenting in `/settings`.
7. **Old `/settings` "edit the toml directly" guidance** — STALE (removed in dirty tree).
   Ships until §0 lands. (read via diff)
8. **`[editor] command` error is the only literal "go edit config.toml" notice left** —
   LIVE, low value. Fix: `/set editor` (~20 lines). (read)
9. **Model/auth/skill surfaces: no defects found.** Model row persists honestly; auth is
   fully in-TUI; `/skill` persists. Stated so nobody re-audits them. (read)
