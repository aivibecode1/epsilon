# Wave 20 — permission preset persists (2026-10-15)

Provenance: one implementation child (delegation `deleg_f655fc5f`,
`completed`); its report filed beside this note as `20a-...`. Every
repo-side claim re-verified by the integrator first-hand (full diff read;
focused tests + clippy re-run; own negative control on the danger guard).

## What landed

`[permissions] preset` key (`config.rs`: `load_preset_in`/`save_preset_in` +
global wrappers, string layer — `Preset` lives in `epsilon-safety`, which
depends on core, so validation sits one layer up); `ProjectConfig.preset` +
`resolve_effective_preset` (flag > project > global > auto, unknown falls
through) applied in `build_runtime`, the one composition point, so every mode
inherits it; write-through on both TUI switch sites (`run_preset_command`
and the `pending_preset` yes-path) via one `persist_preset` helper.

## Integrator overrides (with reasons)

1. **`danger-full-access` is never persisted** (child had `yes` writing it
   down). A standing full-access file would reload the next launch with no
   question asked — the session-remembered-allow leak the ledger already
   records for `always allow bash`, and fail-closed forbids it. `yes`
   switches in memory and the notice says "this run only". Test renamed to
   `preset_danger_still_gates_and_never_persists` and re-pinned.
   Control: guard neutralised (`if false`) → the test FAILS for the reason
   it names; restored → green; `grep -rn "if false"` clean.
2. Unknown-project-name fall-through to global **accepted as documented**:
   both files are the user's own, and the direction is stated on the
   resolver. Usability wart, not a vulnerability.
3. Callerless `save_preset` global wrapper **kept**: every key in the module
   carries the `_in` + global pair; a 3-line convention entry is not dead
   code with better manners, it is the module's shape. Recorded so nobody
   "cleans" it into an asymmetry.

## Verification state

- RED: tests-against-base fail to compile (`cannot find function
  load_preset_in…`, per child); integrator's own control above for the
  danger guard.
- Focused (integrator-run): all 8 `preset_` TUI tests green; clippy
  `-D warnings` clean on the three crates; `cargo fmt --check` clean
  (one reflow of the new test).
- Battery: `gate.sh wave20-20261015` (fresh label; `CARGO_BUILD_JOBS=1`,
  debug info off) — verdict + per-binary reconciliation recorded in
  STATUS on completion; log copied to `audit/evidence/`.
- Residuals: explicit `--preset auto` cannot mask a persisted preset for
  one run (flag is `Auto` either way; needs `main.rs`, documented on the
  resolver); hand-written `danger-full-access` in a config file applies
  silently (explicit standing choice in the user's own file — accepted).

## Queued next (wave-19 backlog order)

Keybindings remap + custom-provider disclosure hints; `/set editor`;
picker stage-1 filter that filters; unconsumed `ToolExecutionEnd`; then
reference adoptions (ask tool first).
