# Plan 016: @-file autocomplete + export/import/share (W1/W2/W3)

> **Status: DONE — do not execute.** `docs/plans/README.md` records this plan's milestone as
> complete, and the work it describes has landed. The "Executor instructions" below are the
> original text, kept for the record; they are **historical**, not current guidance.
> Re-verify any claim here against the current tree before relying on it — several `pi-0.84.2`
> citations in this file were checked on 2026-09-26 and corrected.

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `docs/plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e200815..HEAD -- crates/epsilon-tui crates/epsilon-tools crates/epsilon-session`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: 013 (reuses slash registry + picker plumbing)
- **Category**: direction/dx
- **Planned at**: commit `e200815`, 2026-08-27
- **Issue**: —

## Why this matters

Typing a path into the prompt is the slowest part of the TUI loop — pi and opencode both autocomplete `@src/` so the model gets the right file without a tool-call correction. Epsilon also has no way to export a session (pi `export` html/jsonl, opencode `session export`). Closing W1+W3 makes the TUI feel finished without touching the loop, at low effort.

## Current state

Relevant files, each with one line on its role:
- `crates/epsilon-tui/src/picker.rs` — `SLASH_ITEMS`, `Command`, `parse_command`, `build_rows_with`, `slash_items` — slash autocomplete pattern
- `crates/epsilon-tui/src/app.rs` — `App { input, slash_open, slash_index, palette_open, workspace_root, ... }`, `handle_key`, `run_command`, `handle_submit`, input rendering, `theme.rs` usage
- `crates/epsilon-tui/src/state.rs` — `UiState` transcript lines, `clear_lines`, `usage`, `RenderPrefs`
- `crates/epsilon-tools/src/util.rs` — `walk_files(&root) -> Vec<PathBuf>` capped at `WALK_MAX_ENTRIES`, `resolve_in_workspace`, `read_text_capped`
- `crates/epsilon-tools/src/search.rs` — `glob_to_regex`, `GlobTool` — bare `**/` expansion, shared walker
- `crates/epsilon-session/src/store.rs` — `read_all() -> Vec<SessionEntry>`, `path()` — JSONL read for export
- `references/pi-0.84.2/packages/tui/src/components/editor.ts` — `@` file reference UX (trigger on `@` + word chars, filter files, Tab inserts path)
- `references/opencode-1.18.21/docs/cli` — `export` command pattern

Current UX to extend:
- `@` typed in `input` is plain text; no completion popup, no insertion.
- No `/export` handler beyond 013's notice stub.
- File walks use `util::walk_files` — reuse for at-file completion.

Repo conventions to honor:
- Picker rows cached in `App::picker_rows` — see `picker.rs:build_rows_with`. At-file rows should follow the same caching pattern (walk only when `@`-word changes, cap 200).
- File walks reuse `util::walk_files` — do not reimplement `ignore` handling at v1.
- Input helpers in `crates/epsilon-tui/src/editor_ops.rs` — keep `@`-word extraction near them if a helper is cleaner.
- Paste markers `[paste #N +L lines]` are atomic segments — `@`-completions inserting a path must not break marker atomicity.
- Formatting: `cargo fmt --all -- --check` clean; `cargo clippy` zero warnings.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Check | `cargo check -p epsilon-tui -p epsilon-tools -p epsilon-session` | exit 0 |
| Lib tests | `cargo test -p epsilon-tui --lib -p epsilon-tools --lib` | all pass |
| Full gates | `cargo fmt --all -- --check && cargo clippy --workspace --all-targets && cargo test --workspace --lib` | clean / 0 warnings / all pass |

## Suggested executor toolkit

- Read `references/pi-0.85.1/packages/tui/src/components/editor.ts:251–263,756–801` for the @-trigger (fires on `@` at a word boundary with pattern `[^\s]*`, and Tab/Enter both call the same `applyCompletion`). **Corrected 2026-09-26 — both specifics below were wrong:** the trigger pattern is `[^\s]*`, not `[\w./-]*` (`editor.ts:258| return new RegExp(`(?:^|[\s])[${…}][^\s]*$`)`), and there is **no** Tab/Enter asymmetry — `756–775` and `777–801` run identical `applyCompletion` calls. File *filtering* and the trailing space live in `references/pi-0.85.1/packages/tui/src/autocomplete.ts:412–418`, not `editor.ts`: `416| const isDirectory = item.label.endsWith("/");` and `417| const suffix = isDirectory ? "" : " ";` — so the space is **directory-conditional**, not an Enter-only behaviour.
- Keep ponytail heuristic: smallest diff — @-completion reuses `util::walk_files` + `picker.rs` fuzzy; export is one file write + one notice.

## Scope

**In scope**:
- `crates/epsilon-tui/src/picker.rs` (at-file row builder, export/import/share slash items if 013 didn't cover them)
- `crates/epsilon-tui/src/app.rs` (at-trigger state, completion popup, insert, export/import/share handlers, help polish if needed)
- `crates/epsilon-tui/src/editor_ops.rs` (only if a small helper for @-word extraction is cleaner there)
- `crates/epsilon-tools/src/util.rs` (only if a small helper like `at_file_candidates` needs a shared walker cap)
- Tests co-located

**Out of scope**:
- `crates/epsilon-llm/*`, `crates/epsilon-safety/*`, `crates/epsilon-core/*` — loop/safety/prompt frozen
- Image paste, LSP diagnostics, HTML templating deps — explicitly deferred
- New workspace deps

## Git workflow

- Branch: `advisor/016-atfile-and-export`
- Commit style: `feat(m33): @-file autocomplete + session export (W1/W3)`
- One commit is fine; push only when operator instructs.

## Steps

### Step 1: Confirm drift and baseline

Run drift check. Ensure 013 landed (slash registry expanded).

**Verify**: `grep -n "SLASH_ITEMS" crates/epsilon-tui/src/picker.rs | head` shows expanded registry. `cargo test -p epsilon-tui --lib 2>&1 | tail -n 20` → pass.

### Step 2: @-file autocomplete

- Trigger: when `input` contains `@` followed by `[\w./-]*` at word boundary, open an at-file picker overlay (reuse picker UI, new mode `at_file`). Source candidates via `util::walk_files(&workspace_root)` limited to 200 entries, filtered by substring after `@` via `fuzzy::fuzzy_match` (same as model picker). Show `path` + `is_dir` hint.
- Navigation: Up/Down moves highlight, Tab inserts `path` (replacing `@<filter>`), Enter inserts `path + " "` (pi: `@` refs are space-terminated), Esc dismisses and restores `@`-word as typed.
- Keep cheap: walk only when `@`-word changes, cache rows in `App` like `picker_rows`; cap 200 to avoid scanning `target/`.

**Verify**: `cargo test -p epsilon-tui --lib at_file` (new tests) → pass. Manual: type `read @src/` → file list filtered to `src/`; Tab completes.

### Step 3: /export and supporting lifecycle slashes

- `Command::Export(tail)` parses `tail.split_whitespace()`: first token `html|json` (default `html`), second token optional path (default `<workspace>/.epsilon/exports/<session_id>.{html,json}`).
- HTML: reuse `crates/epsilon-tui/src/markdown.rs` + `render_cache` lines, wrap in minimal HTML shell with `<pre>` transcript (no new dep). JSON: serialize `store.read_all()` via `serde_json::to_string_pretty`.
- Write file, `notice("exported to {path} ({bytes} bytes)")`. On failure, `notice("export failed: {e}")`.
- `/import <path>` → notice guidance: `"import: restart with --resume <path> (live import deferred)"`
- `/share` / `/copy` → honest stubs: `"share: use /export then publish the file (gist upload deferred)"`, `"copy: transcript copied? (clipboard deferred) — use /export for now"`

**Verify**: `cargo test -p epsilon-tui --lib export` → pass. After `/export html`, `ls .epsilon/exports/` shows file.

### Step 4: Gates

**Verify**: `cargo fmt --all -- --check` → 0. `cargo clippy --workspace --all-targets` → 0 warnings. `cargo test -p epsilon-tui --lib` → all pass including new tests.

## Test plan

Model after `picker::tests` and `app::tests::slash_*`:
- `at_file_filters_by_substring` — `input = "fix @src/ap"`, assert at-file rows filtered to paths containing `src/ap`.
- `at_file_tab_inserts_path` — Tab on `src/app.rs`, assert `input == "fix src/app.rs"` (replaces `@src/ap`).
- `export_writes_html_and_notices_path` — run `Command::Export("html")` with temp workspace, assert file exists and contains `<html`.

## Done criteria

Machine-checkable. ALL must hold:
- [ ] `grep -q "at_file\|AtFile" crates/epsilon-tui/src/app.rs` (or `editor_ops.rs`)
- [ ] `grep -q "Command::Export" crates/epsilon-tui/src/picker.rs crates/epsilon-tui/src/app.rs`
- [ ] `cargo test -p epsilon-tui --lib` exits 0; new at-file/export tests pass
- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets` exit 0
- [ ] No files outside the in-scope list modified (`git status`)
- [ ] `docs/plans/README.md` status row for 016 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:
- The code at the locations in "Current state" doesn't match the excerpts (drift).
- `@`-completion requires a new file-watcher crate or LSP integration (out of scope).
- Export needs a new HTML templating dep (keep the shell minimal).
- A step's verification fails twice after a reasonable fix.

## Maintenance notes

- If a file watcher lands later, wire it into at-file candidate caching (same 2s TTL as `branch_cache` in `app.rs`).
- If `GlobTool` ignore rules change, at-file walker must follow the same `ignore` crate config.
- Reviewers: check Tab/Enter insertion is undoable via Ctrl+Z (push EditorSnapshot before insert).
