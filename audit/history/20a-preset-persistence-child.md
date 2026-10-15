# Provenance

Filed 2026-10-15 as part of wave 20 (`20-preset-persistence-2026-10-15.md`, which holds the verdicts). This is the implementation child's report as written to scratch during delegation `deleg_f655fc5f`, unedited except for this header. Repo-side claims in it are leads until `20-...` says re-verified.

---

# Wave 20 report — permission preset persists across restarts

## Files changed
- `crates/epsilon-core/src/config.rs` — `[permissions] preset` string key:
  `load_preset_in` / `load_preset` / `save_preset_in` / `save_preset` + tests.
- `crates/epsilon-runtime/src/lib.rs` — `ProjectConfig.preset` (`[permissions]
  preset`, raw string), `resolve_effective_preset` (flag > project > global >
  auto), `build_runtime` applies the effective preset to the gate + tests.
- `crates/epsilon-tui/src/app/commands.rs` — `run_preset_command` writes
  through on every successful switch.
- `crates/epsilon-tui/src/app.rs` — `persist_preset` helper (shared by the
  `/preset` path and the `pending_preset` yes-path); `project_dir_or_global`
  is the write dir.
- `crates/epsilon-tui/src/app/tests.rs` — write-through round trip,
  danger-gating, settings-panel failure-keeps-open.

## Design decision (DAG constraint)
`epsilon-safety` depends on `epsilon-core`, so `config.rs` cannot name
`epsilon_safety::Preset` (would be a dependency cycle). `config.rs` therefore
persists the preset **name as a string**; parsing/validation
(`Preset::parse`, unknown → fall through) lives in
`epsilon_runtime::resolve_effective_preset`, mirroring how
`load_thinking_level_in` (core) pairs with `resolve_default_thinking_level`
(runtime). The brief's "`load_preset_in` (unknown names → None)" is
implemented one layer up: unknown names never reach the gate.

## Per-item tests + RED proof + controls
- RED (tests-against-base): `cargo check -p epsilon-core -p epsilon-runtime
  -p epsilon-tui --tests` with tests added but no implementation failed to
  compile — `error[E0425]: cannot find function load_preset_in in module
  epsilon_core::config` (+ `resolve_effective_preset` in `epsilon_runtime`,
  5 errors in `epsilon-tui` lib test). New tests fail on base; nothing to
  run past the compiler.
- `preset_round_trips_and_leaves_other_keys_alone` (config): save/read-back,
  `[picker]` coexistence. GREEN.
- `preset_missing_blank_or_malformed_loads_as_none` (config): no file /
  unparseable / non-string / wrong table → None. GREEN. (Unknown *names*
  pass through this layer as strings — see "brief got wrong".)
- `preset_empty_refuses_to_persist_and_non_table_section_errors` (config).
  GREEN.
- `preset_flag_beats_project_beats_global_beats_auto` (runtime, pure fn, no
  home read). GREEN.
- `preset_unknown_names_fall_through_instead_of_inventing_a_policy`
  (runtime): `banana` at either layer falls through. GREEN.
- `a_projects_preset_is_read_from_its_own_config` (runtime project parse +
  malformed control). GREEN.
- `a_project_preset_file_reaches_the_composed_gate` (real `build_runtime`:
  project file `read-only`, no flag → gate runs `read-only`). GREEN.
- `an_explicit_preset_flag_outranks_the_project_file` (real `build_runtime`:
  flag `workspace-write` vs file `read-only` → gate runs `workspace-write`).
  GREEN.
- `preset_switch_writes_through_and_reloads_from_disk` (TUI: `/preset
  read-only` → file holds it → `resolve_effective_preset` returns it).
  GREEN.
- Negative control: with the `persist_preset` call in `run_preset_command`
  neutralised (`let _ = preset.name();`), the round-trip test above FAILS
  with exactly the reason it names: `assertion left == right failed: the
  switch is in the FILE, not only in the gate; left: None,
  right: Some("read-only")`. Call restored afterwards; test green again.
- `preset_danger_still_gates_and_persists_only_on_yes` (TUI): ask writes
  nothing, `no` writes nothing, `yes` persists `danger-full-access`; raising
  direction unchanged. GREEN. Pre-existing
  `preset_full_access_asks_and_only_yes_switches` also still green.
- `preset_settings_row_persists_and_a_failed_write_keeps_the_panel_open`
  (TUI): panel Enter persists `workspace-write`; with the dir blocked by a
  file, in-memory still switches, panel stays open, notice contains
  "not saved". GREEN.

## Verify (exact commands, mandated env)
- `cargo fmt -p epsilon-core -p epsilon-runtime -p epsilon-tui -- --check`:
  CLEAN (`FMT_CLEAN`).
- `cargo clippy -p epsilon-core -p epsilon-runtime -p epsilon-tui
  --all-targets -- -D warnings`: CLEAN, exit 0.
- Full suites: epsilon-core lib 223/223, epsilon-runtime lib 111/111,
  epsilon-tui lib 770 passed / 0 failed / 14 ignored (pre-existing ignores).
- Pre-existing `preset_carries_through_resolution` (launch passes flag
  through) still green — `resolve_launch` intentionally untouched.

## `preset never persists` repro before/after
- BEFORE (base `HEAD`): `git grep "permissions" HEAD` over the three crates
  shows no config key, no reader, no writer — `Agent::set_preset` swaps only
  the in-memory gate, so any restart returns to `auto` with no notice. There
  is no seam to demonstrate against; the RED compile errors above are the
  before-proof (the persistence path does not exist).
- AFTER: `/preset read-only` → `<dir>/config.toml` contains
  `[permissions] preset = "read-only"` → fresh `build_runtime` with no flag
  reports `current_preset() == "read-only"` (covered by the compose test).

## What the brief got wrong
1. "Four files" but only three paths listed; the work honestly needs five:
  `app.rs` (the `pending_preset` yes-path is a second switch site that
  `run_preset_command` does not cover — without it a confirmed
  `danger-full-access` would be the one switch that silently doesn't
  persist) and `app/tests.rs` (TUI tests live there, not beside
  `commands.rs`, which has no test module).
2. "`load_preset_in` (unknown names → None)" cannot live in `config.rs`:
  `epsilon-safety` (home of `Preset`) depends on `epsilon-core`, so core
  naming the type would be a dependency cycle. `config.rs` persists the
  raw name string; unknown→fall-through lives one layer up in
  `resolve_effective_preset`, mirroring the
  `load_thinking_level_in`/`resolve_default_thinking_level` split. No new
  dependency added, DAG intact.
3. "`--preset` flag still outranks file" needs a qualifier: the flag is
  `Preset::Auto` both when absent and when explicitly passed as `auto`
  (distinguishing them would need `main.rs` changes, outside scope), so an
  explicit `--preset auto` does not mask a persisted preset for one run.
  Documented on `resolve_effective_preset`; non-auto flags always win.
4. No `save_scope` exists in `commands.rs` — it lives in `app.rs`
  (`save_scope`, `/scoped-models` checklist). The failure shape was still
  followed: panel stays open + reason notice.

## Residuals
- A persisted `danger-full-access` reloads next launch with no re-gate
  (composition has no UI to ask). Uniform persistence was implemented per
  the brief; if the integrator wants danger to stay ephemeral, the seam is
  one line in `resolve_effective_preset` — say so, do not silently diverge.
- Unknown project name + permissive global file resolves to the global
  (mirrors the thinking-level fall-through). Deliberate; flagged because
  for a *permissions* key the direction of the fall-through matters.
- `save_preset` (global wrapper) has no production caller yet — kept for
  API symmetry with every other key in the module (all have `_in` + global
  pairs); clippy/fmt clean.
- Write dir is `project_dir_or_global` (model-default shape), so a workspace
  with `.epsilon/` gets a per-project preset and one without falls back to
  global — matching the project-first load precedence.
- Diff summary: 5 files, +567/−2, no commits; tree was clean at start and no
  unrelated file was touched (`git status --short` shows only the five).
