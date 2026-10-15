# Provenance

Filed 2026-10-15 as part of wave 19 (`19-settings-changes-2026-10-15.md`, which holds the verdicts). This is the audit child's report as written to scratch during delegation `deleg_ade17047`, unedited except for this header. Repo-side claims in it are leads until `19-...` says re-verified.

---

# Epsilon audit: code quality of in-flight stack + docs honesty

Repo: `C:/Users/Hp/Desktop/My Agent Space/Epsilon`. READ-ONLY audit, no cargo run.
Subject: working tree `main` HEAD `9258db1` + 11 dirty files (the `/changes` + `/settings` stack).
Labels: **read** = seen in file output; **inferred** = conclusion drawn from reads; **computed** = machine-counted.

## Dirty stack (from `git status --porcelain` + `git diff --stat`, read)

11 files, +270/−33. No `Cargo.toml`/`Cargo.lock`, no `keys.rs`, no `panels.rs`, no test files.

- `crates/epsilon-contracts/src/command.rs` — `Command::Changes(String)` variant, `/changes` SLASH_ITEM, `"changes"` head, parse arm, `/settings` description rewrite
- `crates/epsilon-core/src/config.rs` — `save_tui_theme_in`
- `crates/epsilon-runtime/src/lib.rs` — `ChangeRow`, `list_changes`
- `crates/epsilon-session/src/lib.rs` — re-export `timestamp_unix_secs`
- `crates/epsilon-session/src/store.rs` — `timestamp_unix_secs`
- `crates/epsilon-tools/src/checkpoint.rs` — `CheckpointStore::changed_since`
- `crates/epsilon-tui/src/app.rs` — 7 new `App` fields, `draw_changes` call + import
- `crates/epsilon-tui/src/app/commands.rs` — `open_changes`, `setting_rows`, `apply_setting`, `/settings` arm rewrite
- `crates/epsilon-tui/src/app/session.rs` — field initializers
- `crates/epsilon-tui/src/app/view.rs` — `overlay_open` disjuncts
- `crates/epsilon-tui/src/ui/theme.rs` — `ThemeMode::next`

## Ranked findings

### 1. LIVE (read) — BLOCKER: the stack does not compile; 4 referenced symbols have no definition

`grep -rn "draw_changes|draw_settings|session_since_unix|next_preset" crates/` returns **only uses, zero definitions**:

- `crates/epsilon-tui/src/app.rs:1930-1931` — `use panels::{... draw_settings ... draw_changes ...};`
- `crates/epsilon-tui/src/app.rs:2126` — `draw_changes(f, app, f.area());`
- `crates/epsilon-tui/src/app/commands.rs:1968` — `session_since_unix(agent)`
- `crates/epsilon-tui/src/app/commands.rs:2060` — `let next = next_preset(&self.settings_preset);`

The dirty set contains no `panels.rs` / `keys.rs` / helper changes, so these were never written in this stack, not merely placed elsewhere (whole-`crates/` grep). `crate::picker::Command::Changes` is fine — `picker.rs:32-34` re-exports contracts `Command`, which the diff extends. But the four above are hard errors; no gate leg can pass on this tree.
Fix shape: add `draw_changes`/`draw_settings` in `panels.rs` (reuse `Panel`/`list_row`/`hint_row` per the `draw_think_picker` pattern at `panels.rs:1626`), key arms in `keys.rs`, and define `session_since_unix` (presumably `timestamp_unix_secs` over the session header) + `next_preset` — or revert the stack to green.

### 2. LIVE (read) — STALE: `/changes` and `/settings` doc strings describe behavior that does not exist yet

- `command.rs:191` — `"files this session changed — enter previews; `/changes all` is the workspace"`: there is no Enter handler; `keys.rs` contains zero `changes_*`/`settings_*` handling (only `think_pick_open` at `keys.rs:1127,2910`).
- `commands.rs:1952-1956` (`open_changes` docs) — `"Enter previews through [describe_checkpoint] and changes nothing... A panel that restored on Enter would overwrite on one keystroke"`: no Enter path exists to preview or restore.
- `app.rs:499-502` (field docs) — `"The panel owns the keyboard while open, the same way the think picker does"`: false on this tree; the think picker owns keys via `keys.rs`, the new panels own nothing.
- `command.rs:217` — `"/settings": "change model, thinking, theme, and permissions here"`: the panel has state but no draw and no keys, so nothing can be changed through it yet.
- The old `/settings` notice arm (global/project paths, recents, favorites, edit-toml guidance) was deleted in the same diff, so the fallback is gone too.
Accurate counter-examples (still LIVE): the `Command::Changes` variant docs (scope-word refusal rule matches `open_changes`'s guard), and all backend docs (`changed_since`, `list_changes`, `timestamp_unix_secs`, `save_tui_theme_in`, `ThemeMode::next`) match their implementations.
Fix shape: either land finding #1 (docs become true) or reword to in-flight phrasing until then.

### 3. LIVE (read) — PARTIAL: no tests, no key handling for any new behavior

`grep` for `open_changes|setting_rows|apply_setting|changed_since|list_changes|timestamp_unix_secs|save_tui_theme_in|Command::Changes|/changes` across `crates/*/src` hits only the 8 dirty source files — no test module references anything new. Established pattern per feature (think picker: `draw_think_picker` + `keys.rs` arms + frame/test coverage) is 0/3 for both panels.
Fix shape: key arms mirroring `keys.rs:1127-1163`, panel draws mirroring `panels.rs:1626+`, plus golden/frame tests; `setting_rows`/`apply_setting` are `pub(super)` so unit tests can drive them without a terminal.

### 4. LIVE (read) — duplication across the now-three picker paths (lower value, fix shapes included)

- State triples: `think_pick_open/index` vs new `changes_open/index/rows/scope` vs `settings_open/index/preset` (`app.rs`, constructors in `session.rs`) — third copy of the open/index idiom with no shared helper. `overlay_open` (`view.rs:577-589`) grows by disjunct per panel.
- `apply_setting` arms 0/1 (`commands.rs:2039-2056`) re-emit the picker-open sequence (`picker_open=true, picker_index=0, input.clear(), cursor_chars=0`, plus `request_live`) already owned by the model/think pickers.
- `save_tui_theme_in` (`config.rs:~701`) is the third read-modify-write-TOML sibling after `save_hide_thinking_in` (`config.rs:607`) and `save_thinking_level_in` (`config.rs:667`) with no shared helper.
- Explicitly NOT a defect: `ThemeMode::next` vs `flipped` — the docs (`theme.rs`) state the split (2-stop toggle vs 3-stop chooser). `list_changes` over `changed_since` over `paths()` is one thin layer each, acceptable.
Fix shape: a `read_modify_config(config_dir, f)` helper in `config.rs`; an `open_picker(kind)` helper for the settings arms; leave `next`/`flipped` alone.

### 5. LIVE (read), consequence inferred — PARTIAL nit: bare `/changes` silently degrades scope

`commands.rs:1962-1972`: when the word is empty, `since = session_since_unix(agent)`; if that is `None` (unparseable header), scope is labeled `"this workspace"` and rows = everything, with no notice — the user asked for *this session* and is shown the workspace without being told. (The label honestly describes what is shown; the silence is the defect.)
Fix shape: one `push_notice` on the `None`-without-`all` path. One-line fix, low user value.

### 6. LIVE (read + computed) — docs honesty CONFIRMED: scoreboard and gate log match

- Pi matrix: `python scripts/check-parity-counts.py --quiet` → `parity-counts: pi-tui 339 rows; 0 problem(s)`, exit 0. Independent recount of ID-rows: 91 done / 110 partial / 138 deliberately different / 0 missing — exactly the `STATUS.md` scoreboard (`339 rows: 91 done / 110 partial / 0 missing / 138 deliberately different`) and the matrix's own `| **Total** | **339** | **91** | **110** | **0** | **138** |` rows. (Naive `grep -o` overcounts — status words appear in prose; the checker's ID-row parse is authoritative.)
- Gate: every `test result:` line in `audit/evidence/gate-production-20261015-c.log` re-summed in Python → **73 binaries, 2144 passed / 0 failed / 15 ignored**, matching STATUS's `15/15 legs, 2,144 passed / 0 failed / 15 ignored across 73 test binaries`. Tail ends `===== GATE PASS =====`. The `One check failed` line is transcript text inside a demo fixture, not a gate result. `console-probe-{,-typed-}production-20261015.json` both exist.
- Scope note (inferred, important): scoreboard + gate describe the **committed** tree. The dirty stack is uncommitted and (finding #1) uncompilable, so the gate verdict is stale *for the working tree* through no fault of the docs — the gate must be re-run after the stack lands. All 10 fast gate legs (`scripts/check-*.py`) pass on the dirty tree (each exit 0, verified just now), since none of them typecheck Rust.

### 7. LIVE (read) — `missing_docs`, `unsafe`, exact-deps: clean on the new surface

- New pub items (8): `Command::Changes` (+`SLASH_ITEMS`/`COMMAND_HEADS` entries), `save_tui_theme_in`, `ChangeRow` + both fields, `list_changes`, `timestamp_unix_secs` (def + re-export, with `#[must_use]`), `CheckpointStore::changed_since`, `ThemeMode::next`, `pub(super) setting_rows`/`apply_setting` — every one carries docs. Enforcement-level caveat (read): workspace lint is `missing_docs = "warn"` (`Cargo.toml:117`), not deny — so this is hygiene by convention, not gate-enforced.
- `unsafe`: no new `unsafe` blocks; the three hits are pre-existing comment/doc mentions (`golden.rs:3475`, `theme.rs:448`, `store.rs:3081`).
- Exact workspace deps (ADR-016): no `Cargo.toml`/`Cargo.lock` in the dirty set; the touched helpers reuse pinned `toml` (`Cargo.toml:92`) and existing edges only.

## Compact rank (user value × fix shape)

1. Stack doesn't compile (4 missing symbols) — land the missing draws/keys/helpers or revert; blocks everything.
2. `/changes`+`/settings` strings overclaim unbuilt UI — true once #1 lands; else reword.
3. Zero tests + zero key handling for new panels — follow the think-picker pattern.
4. Third picker-state copy + third TOML helper copy — extract two small helpers.
5. Silent session→workspace scope fallback — one notice line.
6. STATUS scoreboard + gate log verified accurate (for the committed tree); re-run gate post-stack.
7. missing_docs/unsafe/deps clean; note `missing_docs` is warn, not deny.
