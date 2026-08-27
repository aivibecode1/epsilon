# Plan 009: Polish /compact, /timeline, /fork (G-M1/G-M4 follow-through)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat abd045a..HEAD -- crates/epsilon-tui crates/epsilon-core crates/epsilon-session`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none (follows m31 `abd045a`)
- **Category**: direction/tech-debt
- **Planned at**: commit `abd045a`, 2026-08-27
- **Issue**: —

## Why this matters

m31 exposed three slash commands (`/compact`, `/timeline`, `/fork`) that close
G-M1/G-M4, but the UX is intentionally rough: `/timeline` dumps 80-char snippets
into notices (not a navigable picker), `/fork` uses a synthetic `fork_` id rather
than a real `BranchBounds` lane, and `/compact` queues via the outer steering path
without a compact-specific spinner. A first-time user sees notices but not a
timeline they can act on. Polishing these three into picker-grade UX completes
the session-tree story and makes compaction observable — the two remaining
"rough edges" the gap analysis called out.

## Current state

Relevant files, each with one line on its role:
- `crates/epsilon-tui/src/picker.rs` — slash registry (`SLASH_ITEMS`) + `Command::{Compact,Timeline,Fork}` + `parse_command`; `SLASH_ITEMS` now holds 11 entries (m31 added compact/timeline/fork)
- `crates/epsilon-tui/src/app.rs` — `run_command` arms for `Compact` (queues or spawns `Agent::compact_now`), `Timeline` (lists `branch_entries()` with 80-char snippets), `Fork` (copies JSONL to `*-fork-<id>.jsonl`); running-state guard uses `UiState::running` check
- `crates/epsilon-core/src/compact.rs` — compaction engine + `CompactionRequest`/`CompactionSummary`; `Agent::compact_now(&mut produced)` is the real compaction entry point
- `crates/epsilon-session/src/store.rs` — `branch_entries() -> Vec<(Entry, BranchBounds)>`, `navigate_to(entry_id)`, `BranchSummary`; JSONL tree read path
- `crates/epsilon-core/src/lib.rs` — `Agent` owns `SessionStore` (via interior handle for TUI/tests), exposes `store_handle_for_tests()` pattern
- `design/epsilon-architecture.md` §6 — session tree; §7 compaction; §11 TUI frontends

Excerpts as of `abd045a`:

```rust
// picker.rs — parse_command additions (m31)
"compact" => Some(Command::Compact),
"timeline" | "branch" => Some(Command::Timeline),
"fork" => Some(Command::Fork),
```

```rust
// app.rs — Timeline arm (current: notice dump, 80-char truncation)
Command::Timeline => {
    let entries = store.lock().unwrap().branch_entries();
    // formats each entry.kind_name() + snippet truncated to 80 chars
    // pushes via self.notice(...)
}
```

Known current UX to replace: timeline-as-notices, fork id is `fork_<hex>` rather than
`SessionStore::fork()` lane id, compact lacks its own spinner/phase.

Repo conventions to honor:
- Every slash item must have a `SlashItem { name, summary, usage }` entry and a `parse_command` arm — see `picker::SLASH_ITEMS` as exemplar.
- App notices flow via `notices_tx`/`startup_notices` vectors then `UiState`; see `App::push_notice`.
- Compaction tests use `SlowProvider` for deterministic time, never bare sleeps — see `crates/epsilon-core/tests/compaction_scenarios.rs`.
- Faux-provider-first: new TUI behaviors need `--lib` tests that exercise `App::handle_submit`/`run_command` against a fake store, not against the real LLM.
- Formatting: `cargo fmt --all -- --check` must stay clean; imports via `crate::` in TUI.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Typecheck | `cargo check -p epsilon-tui -p epsilon-core -p epsilon-session` | exit 0 |
| Lib tests | `cargo test -p epsilon-tui --lib -p epsilon-core --lib -p epsilon-session --lib` | all pass |
| Full gates | `cargo fmt --all -- --check && cargo clippy --workspace --all-targets && cargo test --workspace --lib` | clean / 0 warnings / all pass |

## Suggested executor toolkit

- Skills: `improve` audit categories (correctness, UX) when diagnosing drift; `ponytail` laziest-solution heuristic when choosing between picker vs notice refactors.
- Reference: `design/epsilon-architecture.md` §6-7, `docs/qa/07-tui-keymap-audit.md` for slash UX parity.

## Scope

**In scope** (the only files you should modify):
- `crates/epsilon-tui/src/picker.rs` (slash descriptions, timeline picker rows)
- `crates/epsilon-tui/src/app.rs` (run_command arms, picker/timeline/fork wiring, compact spinner)
- `crates/epsilon-core/src/lib.rs` (only if a small ergonomic accessor for `branch_entries`/`fork` is needed; prefer reusing `store_handle_for_tests`-style lock)
- `crates/epsilon-session/src/store.rs` (only if `BranchBounds` needs a small display helper)
- Tests co-located in the modified crates

**Out of scope** (do NOT touch, even though they look related):
- `crates/epsilon-llm/*` — provider wires are unrelated.
- `crates/epsilon-tools/*` — tool set is orthogonal.
- Any change to webfetch or broader architecture doc beyond noting polish — that is plan 010.
- Session file format changes — JSONL entry shapes stay as-is.

## Git workflow

- Branch: `advisor/009-compact-timeline-fork-polish`
- Commit style: `feat(m32): polish compact/timeline/fork UX` (see `git log --oneline` for m31 style `feat(m31): ...`)
- Commits per step or per logical unit; push only when operator instructs it.

## Steps

### Step 1: Confirm drift and baseline tests

Run the drift check and baseline gates.

**Verify**: `git diff --stat abd045a..HEAD -- crates/epsilon-tui crates/epsilon-core crates/epsilon-session` matches expectation (no unexpected in-scope diffs beyond m31). Then `cargo test -p epsilon-tui --lib 2>&1 | tail -n 20` → all pass (98 at m31 baseline).

### Step 2: Timeline as a picker, not a notice dump

Replace the notice-dump `Command::Timeline` with a proper picker overlay:
- Add a `timeline` picker mode in `app.rs` that builds rows from `store.branch_entries()`, each row carrying `seq`, `BranchBounds` lane label, `kind_name()`, and an 80-120-char snippet (same truncation as current, but rendered as a picker row, not a notice).
- Selection action: `store.navigate_to(selected_seq)` then `self.refresh_view()` (or equivalent re-render of transcript from the branch).
- Keep `/timeline` as the trigger; alias `/branch` already parses to the same command.

**Verify**: `cargo check -p epsilon-tui` → exit 0. Manual: `/timeline` opens a picker listing branches; selecting an entry jumps the transcript.

### Step 3: Fork via real lane id, not synthetic fork_*

Replace the synthetic `format!("{stem}-fork-{}.{}", new_id("fork"), ext)` filename with the real lane id from `store.fork()` / `store.branch_entries()` / `SessionId` semantics (whichever the store exposes — prefer the smallest API that yields a real branch id). If `store.fork()` does not exist, add a one-method accessor that clones the current leaf via `SessionStore` without duplicating fork logic across TUI. Preserve the current UX: `notice("forked to {path}")`.

**Verify**: `cargo test -p epsilon-session --lib` → all pass. `cargo test -p epsilon-tui --lib` → all pass. `/fork` creates a file whose name contains the real branch/lane id, not `fork_`.

### Step 4: Compact spinner and state

`/compact` already queues vs spawns via the running-state guard. Add a compact-specific running phase:
- When `Compact` spawns `Agent::compact_now`, set `self.compact_running = true` (or reuse `spinner_phase` with a distinct label), push `notice("compacting context…")`, and on `compaction_finished` event clear the flag and push duration/tokens stats (already in the `CompactionSummary` produced by `compact.rs`).
- While compacting, subsequent `/compact` should notice `compact already running` rather than queue a second compaction.

**Verify**: `cargo test -p epsilon-core --lib compact` (or `compaction_scenarios`) → all pass. `/compact` while idle shows a transient "compacting…" indicator; `/compact` while already compacting is refused politely.

### Step 5: Polish slash registry copy

Update `SLASH_ITEMS` summaries for the three commands to user-facing prose:
- `/compact — summarize older context to reclaim token window (manual, complements auto-compact)`
- `/timeline — browse session branches by seq/lane/snippet, jump to any point`
- `/fork — copy current session JSONL to a new branch file`

Ensure `/help` reflects them.

**Verify**: `grep -n "SLASH_ITEMS" crates/epsilon-tui/src/picker.rs` shows all three with non-empty `summary`/`usage`. `cargo fmt --all -- --check` → exit 0.

## Test plan

New tests, model after existing tui tests in `crates/epsilon-tui/src/app.rs` (~2500+):
- `timeline_lists_branches_and_jumps` — seed a fake store with 3 branches, run `Command::Timeline`, assert picker rows == 3, simulate select of branch 2, assert `navigate_to` called / transcript seq advanced.
- `fork_creates_file_with_real_lane_id` — seed a non-empty session, run `Command::Fork`, assert file exists, filename contains a lane/branch id from the store (not the string `fork_`).
- `compact_queues_when_running_and_spawns_when_idle` — set `ui.running = true` vs `false`, assert the two distinct notices; when idle, assert `compact_now` spawned (via a fake agent double).
- Edge: `timeline_empty_is_notice` — empty session `/timeline` pushes `nothing to show` notice rather than opening an empty picker.

Pattern: `tab_completes_highlighted_candidate` in `app.rs:3092` for how picker/slash completion is tested.

Verification: `cargo test -p epsilon-tui --lib` → all pass including N new tests.

## Done criteria

Machine-checkable. ALL must hold:
- [ ] `cargo fmt --all -- --check` exits 0
- [ ] `cargo clippy --workspace --all-targets` exits 0, 0 warnings
- [ ] `cargo test -p epsilon-tui --lib` exits 0; new tests for timeline/fork/compact exist and pass
- [ ] `grep -rn "SLASH_ITEMS" crates/epsilon-tui/src/picker.rs` shows compact/timeline/fork with summaries
- [ ] `git status` shows no files outside the in-scope list modified
- [ ] `plans/README.md` status row for 009 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:
- The code at the locations in "Current state" doesn't match the excerpts (codebase drifted since `abd045a`).
- `SessionStore::branch_entries`/`navigate_to`/`fork` signatures differ from those in the plan in a way that requires touching session file format or persistence logic beyond adding a small accessor.
- Adding the timeline picker requires a new crate or a TUI framework swap (out of scope).
- A step's verification fails twice after a reasonable fix attempt.

## Maintenance notes

- If pagination is later added to the session store, the timeline picker's row builder must be revisited (same branch_cache pattern as footer).
- If compaction thresholds become configurable, the compact spinner label should include the threshold that triggered auto-compact vs manual.
- Reviewers should scrutinize the `navigate_to` jump: it must be a read-view change, not a destructive truncation of the JSONL.
