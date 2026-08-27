# Plan 012: TUI @-file autocomplete + export + help polish

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat abd045a..HEAD -- crates/epsilon-tui crates/epsilon-tools`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: 009 (reuses its picker plumbing for timeline)
- **Category**: direction (Worth-adopting surface asymmetries)
- **Planned at**: commit `abd045a`, 2026-08-27
- **Issue**: —

## Why this matters

The TUI's picker is now two-stage for models and has slash autocomplete, but
- typing `@src/` does nothing (pi and opencode both autocomplete file refs),
- the transcript cannot be exported (pi `export` / opencode `session export`),
- `/help` is a one-liner notice. A user who wants to reference a file still
types the path blind, and a user who wants to share a session has no command
for it. These are small, high-leverage UX completions that make the TUI feel
finished without touching the loop or safety layers.

## Current state

Relevant files:
- `crates/epsilon-tui/src/picker.rs` — `SLASH_ITEMS`, `Command` enum, `parse_command`, `filter_palette`, `build_rows_with` (fuzzy on provider/model ids)
- `crates/epsilon-tui/src/app.rs` — `App` state (`input`, `picker_open`, `slash_open`, `palette_open`, `branch_cache`, `workspace_root`), `handle_submit`, `run_command`, hint bar, input rendering
- `crates/epsilon-tui/src/state.rs` — `UiState` transcript lines, `RenderPrefs`, `clear_lines` / `usage` helpers
- `crates/epsilon-tools/src/lib.rs` — `GlobTool`/`ReadTool` already walk the workspace (reuse for file completion)
- `design/epsilon-architecture.md` §11 — frontends; docs/qa/07-tui-keymap-audit.md — keymap
- `references/pi-0.84.2/packages/tui/src/components/editor.ts` — `@` file reference UX in the editor
- `references/opencode-1.18.21/docs/cli/` — `export` command pattern

Current UX to extend:
- `@` typed in `input` is plain text; no completion popup, no insertion.
- No `/export` slash; no export path in `App`.
- `/help` pushes 4 short notices (`commands: /models ...`) — dense but not navigable.

Repo conventions:
- Picker rows built via `crate::picker::build_rows_with` cached in `App::picker_rows`; slash popup uses `slash_items` / `slash_index`.
- File walks use `GlobTool`/`grep` via the registry — reuse the walker, don't reimplement `ignore` handling.
- Input helpers in `crates/epsilon-tui/src/editor_ops.rs` — word ops, kill-ring; keep `@`-completion near them.
- Paste markers `[paste #N +L lines]` are atomic segments — `@`-completions that insert a path must not break marker atomicity.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Check | `cargo check -p epsilon-tui` | exit 0 |
| Lib tests | `cargo test -p epsilon-tui --lib` | all pass |
| Full gates | `cargo fmt --all -- --check && cargo clippy --workspace --all-targets` | clean / 0 warnings |

## Suggested executor toolkit

- Read `references/pi-0.84.2/packages/tui/src/components/editor.ts` for @-trigger UX (trigger on `@` + word chars, filter files, Tab/Enter inserts `path `).
- Keep ponytail heuristic: smallest diff — @-completion reuses `GlobTool` walker + `picker.rs` fuzzy; export is one file write + one notice; help polish is one picker list, not a new crate.

## Scope

**In scope**:
- `crates/epsilon-tui/src/picker.rs` (at-file row builder, export slash item if needed)
- `crates/epsilon-tui/src/app.rs` (at-trigger state, completion popup, insert, export handler, help picker)
- `crates/epsilon-tui/src/editor_ops.rs` (only if a small helper for @-word extraction is cleaner there)
- Tests co-located in `picker.rs` / `app.rs`

**Out of scope**:
- `crates/epsilon-llm/*`, `crates/epsilon-safety/*`, `crates/epsilon-core/*` — loop/safety/prompt frozen.
- Image paste, LSP diagnostics, or HTML export — explicitly not in this plan.
- Any new workspace dep.

## Git workflow

- Branch: `advisor/012-tui-atfile-export-help`
- Commit style: `feat(m32): @-file autocomplete + /export + help picker`
- One commit is fine; push only when operator instructs.

## Steps

### Step 1: Confirm drift and add SLASH_ITEMS for /export

Run drift check. Add to `SLASH_ITEMS`:

```rust
SlashItem { name: "/export", summary: "export transcript to HTML/JSON", usage: "/export [html|json] [path]" }
```

Map in `parse_command`: `"export" => Some(Command::Export(tail))` (tail carries optional `html|json` + optional path).

**Verify**: `grep -n "SLASH_ITEMS\|Command::Export" crates/epsilon-tui/src/picker.rs` shows hits. `cargo check -p epsilon-tui` → exit 0.

### Step 2: @-file autocomplete (editor trigger → file picker → insert)

- Trigger: when `input` contains `@` followed by `[\w./-]*` at word boundary, open an at-file picker overlay (reuse picker UI, new mode `at_file`). Source candidates via a lightweight `glob "**/*"` limited to 200 entries, filtered by the substring after `@` via `fuzzy::fuzzy_match` (same as model picker). Show `path` + `is_dir` hint.
- Navigation: Up/Down moves highlight, Tab inserts `path` (replacing `@<filter>`), Enter inserts `path + " "` (pi behavior: `@` refs are space-terminated), Esc dismisses and restores the `@`-word as typed.
- Keep it cheap: walk only when `@`-word changes, cache rows in `App` like `picker_rows`; cap to 200 to avoid scanning `target/`.

**Verify**: `cargo test -p epsilon-tui --lib at_file` (new tests) → pass. Manual: type `read @src/` → file list filtered to `src/` entries; Tab completes.

### Step 3: /export handler (HTML + JSON)

- `Command::Export(tail)` parses `tail.split_whitespace()`: first token `html|json` (default `html`), second token optional path (default `<workspace>/.epsilon/exports/<session_id>.{html,json}`).
- HTML: reuse `crates/epsilon-tui/src/markdown.rs` rendering + `render_cache` lines, wrap in a minimal HTML shell with `<pre>` transcript (no new dep). JSON: serialize `store.read_all()` entries (same as JSONL) via `serde_json::to_string_pretty`.
- Write file, `notice("exported to {path} ({bytes} bytes)")`. On write failure, `notice("export failed: {e}")`.

**Verify**: `cargo test -p epsilon-tui --lib export` → pass. `ls .epsilon/exports/` shows the file after `/export`.

### Step 4: Help as a picker (not a notice dump)

Replace the current `Command::Help` notice burst with a help picker listing every `SLASH_ITEMS` entry plus keybinding hints from `palette_entries()`. Selecting an entry inserts its `name` into `input` (like the existing slash autocomplete Enter behavior) so the user can run it. Keep `Command::Help` backward compatible: `/help` opens the picker, not a notice.

**Verify**: `cargo test -p epsilon-tui --lib help_picker` → pass. `/help` opens a navigable list; selecting `/models` inserts `/models` into input.

### Step 5: Gates

**Verify**: `cargo fmt --all -- --check` → exit 0. `cargo clippy --workspace --all-targets` → 0 warnings. `cargo test -p epsilon-tui --lib` → all pass including new tests.

## Test plan

Model after `picker::tests` and `app::tests::slash_*` in `crates/epsilon-tui/src/app.rs`:
- `at_file_filters_by_substring` — set `input = "fix @src/ap"`, assert at-file rows filtered to paths containing `src/ap`.
- `at_file_tab_inserts_path` — simulate Tab on highlighted `src/app.rs`, assert `input == "fix src/app.rs"` (replaces `@src/ap`).
- `export_writes_html_and_notices_path` — run `Command::Export("html")` with a temp workspace, assert file exists and contains `<html`.
- `help_opens_picker_with_all_slashes` — run `Command::Help`, assert help picker rows == `SLASH_ITEMS.len()` and each has a summary.

Pattern: `tab_completes_highlighted_candidate` in `app.rs:3092` for completion tests.

## Done criteria

Machine-checkable. ALL must hold:
- [ ] `grep -q "Command::Export" crates/epsilon-tui/src/picker.rs crates/epsilon-tui/src/app.rs`
- [ ] `grep -q "at_file\|AtFile" crates/epsilon-tui/src/app.rs` (or `editor_ops.rs`)
- [ ] `cargo test -p epsilon-tui --lib` exits 0; new tests for at-file/export/help exist and pass
- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets` exit 0
- [ ] No files outside the in-scope list modified (`git status`)
- [ ] `plans/README.md` status row for 012 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:
- The code at the locations in "Current state" doesn't match the excerpts (drift).
- `@`-completion requires a new file-watcher crate or LSP integration (out of scope).
- Export needs a new HTML templating dep (it shouldn't — keep the shell minimal).
- Help picker conflicts with the existing slash autocomplete popup in a way that needs a TUI framework change.

## Maintenance notes

- If a file watcher lands later, wire it into at-file candidate caching (same 2s TTL as `branch_cache`).
- If `GlobTool` ignore rules change, at-file walker must follow the same `ignore` crate config.
- Reviewers should check that Tab/Enter insertion is undoable via Ctrl+Z (push `EditorSnapshot` before insert).
