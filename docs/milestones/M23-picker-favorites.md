# M23 — Picker favorites + favorite cycling (DIRECTION-06)

Date: 2026-08-25. DIRECTION-06 from the audit program's direction list, built
as an opencode/pi hybrid after re-reading both references:

- **opencode** (`packages/tui/src/context/local.tsx`, `dialog-model.tsx`):
  favorites are a persisted list toggled inside the model dialog (Ctrl+F),
  shown as a "Favorites" section ABOVE "Recent", with recents filtered to
  exclude already-favorited ids; `cycleFavorite(±1)` walks the list.
- **pi** (`agent-session.ts cycleModel`, `--models` scoped cycling): cycling
  wraps modulo the list length and entering from a non-listed model starts at
  the forward end; every switch persists the default model.

Epsilon takes opencode's storage/toggle/section UX and pi's cycling
semantics.

## What shipped

### config (`epsilon-core`)
`[picker] favorite = [...]` in the global `config.toml`, insertion order,
capped at 16 (`FAVORITE_MAX`). `toggle_favorite_model_in` returns whether the
toggle REMOVED (true) or ADDED (false); refuses empty ids and a full list.
Separate `[picker]` key from `recent`, so neither write clobbers the other
(test pins that).

### picker (`epsilon-tui`)
- `build_rows_with(filter, current, recents, favorites, live)` — new
  favorites-aware builder; `build_rows` keeps its old signature delegating
  with an empty favorites list so existing tests/call sites stay valid.
- Section order: active → **favorite** → recent → live → catalog. Favorites
  are excluded from the recents pass (no double listing). The star marker
  follows the id into the full catalog section too (`is_favorite` on
  [`PickerRow`]).
- Stage-2 rows render a ★ column (yellow) for favorited models.

### app keys
- **Ctrl+F in the picker** now toggles the star on the highlighted row
  (opencode's exact `model_favorite_toggle` binding). The old Ctrl+F job
  (force live refresh) moved to **Ctrl+G**; opening the picker still
  auto-fetches live listings without any key (M20), so nothing is lost.
- **F2 / Shift+F2**: cycle favorites forward/backward from anywhere in the
  editor (pi's `app.model.cycleForward` semantics over opencode's favorite
  list). With no favorites: inline guidance instead of a crash. Cycling goes
  through `apply_model`, so it persists the choice and updates recents like
  any other switch.
- Hint bar updated: `Ctrl+F favorite ★ · Ctrl+G refresh`.

## Test-caught bugs / gotchas

- First draft reused the M21-era channel bug pattern (`self.notice` for a
  reply that must render this frame) in bare `/think`; caught by the M22 test,
  fixed there. This milestone's helpers push directly into UiState from the
  start — the lesson finally stuck.
- A patch collapsed two lines of an existing test into one during editing;
  cargo fmt flagged it. Re-fixed immediately (lesson: run fmt before trusting
  any hand-edited region).

## Verification

fmt clean · clippy 0 warnings both configs · offline + real suites green ·
demo completes. New tests: favorites toggle round-trip + recents-coexistence
(config), section ordering + star marking, F2 cycle forward/wrap/backward +
empty guidance (TUI key paths).

## Known limits / next

- No per-favorite thinking level yet (pi's scoped models carry
  `thinkingLevel` per entry); natural follow-up if wanted.
- Favorites cap is 16 with an explicit error when full rather than silent
  eviction — matches the project's loud-over-silent posture.
