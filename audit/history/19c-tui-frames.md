# Provenance

Filed 2026-10-15 as part of wave 19 (`19-settings-changes-2026-10-15.md`, which holds the verdicts). This is the audit child's report as written to scratch during delegation `deleg_ade17047`, unedited except for this header. Repo-side claims in it are leads until `19-...` says re-verified.

---

# Epsilon TUI product-design audit — captured frames + draw code

Scope: working tree at main HEAD `9258db1` + dirty /changes stack (11 files, `git diff --stat`).
READ-ONLY: nothing compiled, no cargo run. Compilability consequences below are
inferred from name resolution (repo-wide grep), labeled as such.

Frames read (all 2026-10-15, `audit/evidence/frames/`, 80x24 unless noted):
permission (80x24, 100x30), palette-open (both), brand-new, idle-with-content,
streaming-tool-running, tool-cards, compaction-opened, compaction-collapsed,
file-editor-find, file-editor-find-long, file-editor-undone-edit,
search-marks, search-marks-second, streaming (both), live-edge-short (100x30),
live-edge-scrolled, live-edge-streaming, streaming-tool-output,
katakana-sound-marks, high-contrast.
Console probes read: `console-probe-production-20261015.json`,
`console-probe-typed-production-20261015.json` (both 80x24, real `epsilon.exe`).

Draw/key code read: `app.rs` draw() 1992–2212, `app/panels.rs` picker/session-browser/
palette/at-file/timeline/stage2/dialog, `app/view.rs` hints()/overlay_open(),
`app/keys.rs` approval + viewer-find + picker + viewer-edit branches,
`app/commands.rs` open_changes/setting_rows/apply_setting/open_timeline_picker,
`ui/footer.rs` approval()+budget, `state.rs` settle paths, `frame_dump.rs` fixtures.

## Ranked findings

### 1. STALE — `/changes` panel can never draw: `draw_changes` is called and imported but defined nowhere
- `crates/epsilon-tui/src/app.rs:2125-2127`: `if app.changes_open { draw_changes(f, app, f.area()); }`
- `app.rs:1931`: `draw_think_picker, draw_changes, draw_timeline, draw_transcript,` (import added by dirty stack)
- Repo-wide grep `fn draw_changes` = 0 hits (worktree AND HEAD `9258db1`); `git diff --stat` shows the stack touches `app.rs` (+23/−2) but not `panels.rs`.
- Consequence (inferred, no cargo in read-only mode): E0432 unresolved import + E0425 unresolved call — the tree as it stands does not compile.
- User value: highest. `/changes` (this session / `all`) is the checkpoint surface, one Enter away from `/restore`; today the command sets `changes_open = true` (`commands.rs:1980`) and then nothing can ever paint it.
- Fix shape: add `draw_changes` in `panels.rs` (list rows over `changes_rows`, title names `changes_scope`, empty state for no checkpoints, hint row), or revert the three stack hunks (state fields, import, call).

### 2. STALE — `/settings` panel state exists but no frame reads it; `draw_settings` imported but undefined
- `commands.rs:2513-2517`: `Command::Settings => { self.settings_preset = ...; self.settings_index = 0; self.settings_open = true; }`
- `app.rs:1930` imports `draw_settings`; grep `fn draw_settings` = 0 hits; draw() (2100–2133) has branches for session_browser/picker/palette/auth/label/auth_pick/think_pick/changes/scope/timeline/tree/viewer/search — **no `settings_open` branch**.
- `setting_rows()` (`commands.rs:1984-2006`) and `apply_setting()` (2011–2071) exist, so half the feature is built against a screen that never appears.
- Fix shape: same as (1) — one `draw_settings` + one `if` branch, or revert.

### 3. STALE — no key handler owns the changes/settings keyboard (keys.rs has zero branches)
- Grep `settings|changes` in `app/keys.rs`: only prose comments (lines 1, 2646). Every sibling overlay has a branch (think_pick 1127, scope 1174, palette 1701, picker 1940).
- `overlay_open()` includes both flags (`view.rs:583-584`), so with either panel "open" the footer prints generic `list · type filter · ↑↓ move · enter choose · esc close` (`view.rs:499-519`) for keys that fall through to the composer — a drawn hint with no handler, the repo's own named defect class.
- Fix shape: handler branches (arrows/enter/esc, filter typing) mirroring scope/think pickers; the footer then tells the truth.

### 4. LIVE — model picker stage-1 title advertises a filter that does nothing
- Title: `format!("models {} filter: '{}'", dash, app.input)` (`panels.rs:80`); rows built **without** input: `build_provider_rows(&app.model_id, &app.customs)` (`panels.rs:62`, repeated `keys.rs:1981,2032`).
- Typing inserts into `input` and resets index (`keys.rs:2166-2170`), so the title quotes the query while the provider list never narrows. The code admits it: `return false; // stage-1 filter is display-only; no index fixups` (`keys.rs:2064`). Stage 2 filters properly (`build_models_for_provider(.., &app.input, ..)`).
- User value: medium. First screen of `/models`; looks broken on first typing.
- Fix shape: filter providers by `input` in stage 1, or drop `filter: '…'` from the stage-1 title.

### 5. STALE frame + PARTIAL risk — `tool-cards.80x24`: settled calls shown `· running` under an `idle` header
- Frame shows `▸ bash $ cargo test · running` with final output (`test result: ok. 74 passed`) and `▸ write … · running`, header `idle`.
- Fixture (`frame_dump.rs:277-307`) sends `ToolExecutionStart/Update/End` but no `AgentStart`, no `Message::ToolResult`. `UiState::apply` (`state.rs:1698-2068`) has **no `ToolExecutionEnd` arm** — the End events fall into the catch-all and cards stay `Running` (`card.rs:138`). Production settles via `Message::ToolResult → push_tool_result → settle_call` (`state.rs:1665,1833,1423-1432`); the file's own comments warn fixtures must "send what the product sends" (`frame_dump.rs:242-247,323-326`).
- So the frame depicts a screen production never shows — stale evidence, and it currently reads as a product defect.
- Residual PARTIAL risk (inferred): `ToolExecutionEnd` is emitted by core/runtime but consumed nowhere in the TUI; any End-without-ToolResult path (e.g. abort races) leaves `· running` on screen. Cancel-via-Esc IS covered (`settle_cards_for_abort → Stopped + "This call was never finished — the turn was stopped."`, `state.rs:948-974`).
- Fix shape: fixture sends `AgentStart` + `Message::ToolResult` (or drop the frame); optionally handle `ToolExecutionEnd` as a settle signal.

### 6. LIVE minor — same Enter key, two verbs on the palette screen
- Panel hint row: `("enter", "run")` (`panels.rs:744`); footer on the same frame: `list · … enter choose · …` (`view.rs:515`, confirmed in `palette-open.80x24.txt`).
- Fix shape: one verb (`run` fits a command palette; footer generic branch could stay `choose` for pickers only).

### 7. Judgment call, low — permission dialog speaks implementer jargon
- `cap  bash` / `what rm -rf target` (`panels.rs:1327-1341`, seen in `permission.80x24.txt` and both console probes' neighbors). `cap` (= capability namespace) and `what` (= one-line detail) are field names, not user words; this is the screen where a reader decides whether work runs.
- Fix shape: `capability`/`command` (or `allows`/`runs`); keep the 5-wide label column rule intentional.
- Checked and cleared on the same screen: 80x24 footer drops `e` (`approval · y allow once · a always · n deny`) while the dialog shows `e deny + guidance` — by design, footer hints are budgeted whole-or-nothing with location winning (`footer.rs:347-360,388-454`); the dialog itself always names `e`. Deny-only variant stays coherent with refused approve keys (`footer.rs:307-316` vs `keys.rs:525-540`).

### 8. Polish, low — viewer find stacks two `esc` rows with different targets
- `file-editor-find.80x24.txt`: `/needle█ 2 of 2 · enter/n next · p previous · esc close` over `type/enter edit · alt+s save · ctrl+z undo · esc cancel`. Both live, applied in sequence (find owns keyboard first, `keys.rs:787-796,821`; edit Esc is two-stage arm/discard, `keys.rs:651-666`). First Esc closes find; `esc cancel` cannot fire until it does.
- Fix shape: `esc close find` on the find row.

## Checked and clear (LIVE-good, verified against code)
- Header/footer accuracy on real console probes: `Faux (offline scripted) • thinking off • offline`, `ctx 5%*/164k`, branch `main`, ellipsis location, typed text echo (`❯ explain the layout module`, cursor tracked). No fabrication.
- Empty/loading/error/cancelled: session browser splits no-sessions vs filtered-out (`panels.rs:197-216`, names Ctrl+N/Tab); picker stage 2 `no matches — Backspace to widen` (`panels.rs:361-366`); `@` popup `no matching files` (`panels.rs:790-794`); think picker `no level matches` (`:1649-1654`); timeline empty → transcript notice `{label}: session is empty` (`commands.rs:2221-2224`); auth prompt masked + OAuth/title variants (`panels.rs:1565+`); label prompt Enter-clears vs Enter-saves wording (`:1554-1560`).
- Discoverability details that work: permission dialog repeats all four keys; timeline hint budget protects list rows (`panels.rs:1022-1064`); picker capacity measured from paint (`panels.rs:74-79`); scrolled footer names live `g` handler (`keys.rs:2779-2780`, frame `live-edge-scrolled`); guidance mode replaces (not joins) approval keys in dialog, footer, and composer focus (`panels.rs:1354-1378`, `view.rs:411-425`, `app.rs:2088`).
- Layout consistency 80x24 vs 100x30: identical structure, wider header detail (`R0 W0 CH n/a`) and full location only; no reflow defects observed. Narrow-screen tree overlay clears before drawing; palette clears its band (`panels.rs:694-706`).
