# Plan 013: Complete slash registry to pi parity (G1)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `docs/plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e200815..HEAD -- crates/epsilon-tui crates/epsilon-session crates/epsilon-cli docs/design/epsilon-architecture.md`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: direction/tech-debt
- **Planned at**: commit `e200815`, 2026-08-27
- **Issue**: —

## Why this matters

Epsilon exposes 15 slash items where pi exposes 22 (`BUILTIN_SLASH_COMMANDS`). A user who types `/new`, `/resume`, `/session`, `/tree`, `/clone`, `/trust`, `/reload` hits `Unknown` instead of a session action. The session tree code already exists (`branch_entries`, `navigate_to`, store) but has no slash surface. Completing the registry to pi parity makes the harness discoverable and closes the largest leverage gap per `docs/research/gap-analysis-2026-08-27-v2.md` G1.

## Current state

Relevant files, each with one line on its role:
- `crates/epsilon-tui/src/picker.rs` — slash registry (`SLASH_ITEMS`, `Command`, `parse_command`, `slash_items`); currently 15 entries, missing 7+ pi builtins
- `crates/epsilon-tui/src/app.rs` — `App::run_command` arms for `Models, Model, Think, Compact, Timeline, Fork, Help, Clear, Skills, Search, Login, Logout, Quit, Unknown`
- `crates/epsilon-session/src/store.rs` — `branch_entries()`, `navigate_to(entry_id)`, `read_all()`, `leaf_id()`, `path()` — tree navigation primitives already shipped
- `crates/epsilon-cli/src/main.rs` + `runtime.rs` — `Args`, resume/session file wiring, `load_project_config`/`TrustDecision`
- `references/pi-0.84.2/packages/coding-agent/src/core/slash-commands.ts` — `BUILTIN_SLASH_COMMANDS` 22 entries (settings, model, scoped-models, export, import, share, copy, name, session, changelog, hotkeys, fork, clone, tree, trust, login, logout, new, compact, resume, reload, quit)
- `docs/design/epsilon-architecture.md` §6/§11 — session tree and TUI frontends

Excerpts as of `e200815`:

```rust
// crates/epsilon-tui/src/picker.rs:212
pub const SLASH_ITEMS: &[SlashItem] = &[
    SlashItem { value: "/models", description: "switch model (picker)" },
    SlashItem { value: "/model ", description: "set model by id, e.g. /model kimi-k2.6" },
    SlashItem { value: "/think ", description: "thinking effort: off..max (Ctrl+T cycles)" },
    SlashItem { value: "/compact", description: "compact context (summarize history)" },
    SlashItem { value: "/timeline", description: "list session branch entries" },
    SlashItem { value: "/fork", description: "fork session at current branch" },
    SlashItem { value: "/skills", description: "list discovered skills" },
    SlashItem { value: "/search ", description: "search this session's history" },
    SlashItem { value: "/clear", description: "clear transcript (totals kept)" },
    SlashItem { value: "/help", description: "command reference" },
    SlashItem { value: "/login ", description: "set API key for a provider (paste inside TUI)" },
    SlashItem { value: "/connect ", description: "connect a provider — alias for /login" },
    SlashItem { value: "/auth ", description: "provider auth — alias for /login" },
    SlashItem { value: "/logout ", description: "remove stored key for a provider" },
    SlashItem { value: "/quit", description: "exit the session" },
];
```

```rust
// crates/epsilon-tui/src/picker.rs:338 — parse_command match head
"compact" => Some(Command::Compact),
"timeline" | "branch" => Some(Command::Timeline),
"fork" => Some(Command::Fork),
"help" | "?" => Some(Command::Help),
"clear" => Some(Command::Clear),
// no arms for new/resume/session/tree/clone/trust/reload/export/import/share/copy/name/changelog/hotkeys
```

```rust
// crates/epsilon-tui/src/app.rs:720 — run_command has 14 arms + Unknown
fn run_command(&mut self, agent: &Arc<Agent>, command: crate::picker::Command) -> bool {
    match command {
        crate::picker::Command::Models => { ... }
        // ... Compact, Timeline, Fork, Help, Clear, Skills, Search, Login, Logout, Quit
        crate::picker::Command::Unknown(word) => { self.ui.push_notice(format!("unknown command '/{word}' — /help lists commands")); }
    }
}
```

Repo conventions to honor:
- Every slash item has `SlashItem { value, description }` and a `Command` variant + `parse_command` arm — see `picker.rs:212 SLASH_ITEMS` as exemplar.
- App notices via `self.ui.push_notice` (synchronous, renders next frame) for slash replies; `notices_tx` reserved for async events — see `app.rs:710`.
- Faux-provider-first: new TUI behaviors need `--lib` tests exercising `App::run_command`/`handle_submit` against a fake store, not the real LLM.
- Formatting: `cargo fmt --all -- --check` clean; imports via `crate::` in TUI.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Typecheck | `cargo check -p epsilon-tui -p epsilon-session -p epsilon-cli` | exit 0 |
| Lib tests | `cargo test -p epsilon-tui --lib -p epsilon-session --lib \` | all pass |
| Full gates | `cargo fmt --all -- --check && cargo clippy --workspace --all-targets && cargo test --workspace --lib` | clean / 0 warnings / all pass |

## Suggested executor toolkit

- Skills: `ratatui-tui` (`references/ai-harness.md`) for overlay/picker patterns; `ponytail` laziest-solution — registry parity first, full screen lifecycles later.
- Reference: `references/pi-0.84.2/packages/coding-agent/src/core/slash-commands.ts` for names/descriptions; `docs/design/epsilon-architecture.md` §6 for session tree invariants.

## Scope

**In scope** (the only files you should modify):
- `crates/epsilon-tui/src/picker.rs` (SLASH_ITEMS, Command, parse_command, slash_items)
- `crates/epsilon-tui/src/app.rs` (run_command arms, minimal handlers)
- `crates/epsilon-session/src/store.rs` (only if a small ergonomic helper like `branch_count` is needed; prefer existing APIs)
- Tests co-located in modified crates

**Out of scope** (do NOT touch):
- `crates/epsilon-llm/*` — provider wires unrelated
- `crates/epsilon-tools/*` — tool set is orthogonal (G2/G3)
- `crates/epsilon-safety/*` — trust persistence already exists; deep reload semantics are plan 015
- Session file format changes — JSONL shapes stay as-is
- New crates or workspace deps

## Git workflow

- Branch: `advisor/013-slash-registry-pi-parity`
- Commit style: `feat(m33): complete slash registry to pi parity (G1)` (see `git log --oneline` for `feat(m32d): ...` style)
- Commit per step; push only when operator instructs.

## Steps

### Step 1: Confirm drift and baseline tests

Run drift check and baseline gates.

**Verify**: `git diff --stat e200815..HEAD -- crates/epsilon-tui crates/epsilon-session crates/epsilon-cli` shows no unexpected in-scope diffs. `cargo test -p epsilon-tui --lib 2>&1 | tail -n 20` → all pass.

### Step 2: Expand SLASH_ITEMS + Command + parse_command to pi parity

In `crates/epsilon-tui/src/picker.rs`:

- Expand `SLASH_ITEMS` to cover every pi BUILTIN not yet present. Add at least: `/new`, `/resume`, `/session`, `/tree` (alias for timeline), `/clone`, `/trust`, `/reload`, `/export`, `/import`, `/share`, `/copy`, `/name`, `/changelog`, `/hotkeys`, `/settings`, `/scoped-models`. Keep existing items and order; append new ones after `/quit` or group sensibly — registry order is display order.
- Extend `Command` enum with matching variants (e.g. `New`, `Resume(Option<String>)`, `Session`, `Tree`, `Clone`, `Trust(Option<String>)`, `Reload`, `Export(Option<String>)`, `Import(String)`, `Share`, `Copy`, `Name(Option<String>)`, `Changelog`, `Hotkeys`, `Settings`, `ScopedModels`). Use `String` tails where pi takes an argument hint.
- Extend `parse_command` match to route each new head (case-insensitive via `to_ascii_lowercase` as existing code does). Preserve aliases: `branch` → Timeline/Tree, `q`/`exit` → Quit.

Keep descriptions one-line, pi wording where possible (e.g. `"Start a new session"`, `"Resume a different session"`, `"Navigate session tree (switch branches)"` — copy from pi's `slash-commands.ts` comments).

**Verify**: `cargo check -p epsilon-tui` → exit 0. `grep -c "SlashItem" crates/epsilon-tui/src/picker.rs` increments by ≥14. `cargo test -p epsilon-tui --lib picker` → pass.

### Step 3: Wire run_command arms — minimal honest behavior (ponytail ladder)

In `crates/epsilon-tui/src/app.rs` `run_command`, add arms for each new `Command`. Ponytail: laziest honest implementation that actually works — no new crate, no async lifecycle yet:

- `/new` → clear transcript + notice `"new session — transcript cleared (session file continues; use /fork for a branch file)"` (no store reset; store reset is a follow-up when CLI session lifecycle is refactored).
- `/resume [path]` → notice with guidance: if tail empty, `"usage: /resume <path> — or restart with --resume <file>"`; else attempt to validate path exists and notice `"resume: restart epsilon with --resume <path> to switch files (live switch deferred)"`.
- `/session` → notice with `store.path()`, `leaf_id`, `branch_entries().len()`, `history.len()` (same data TUI header already computes).
- `/tree` (and `/timeline` alias) → reuse existing Timeline logic (branch_entries listing); alias already parses to Timeline, so just ensure `/tree` maps there.
- `/clone` → alias to `/fork` behavior (copy JSONL to `*-clone-<id>.jsonl` or reuse Fork arm with a distinct notice).
- `/trust [yes|no]` → if tail empty, show current trust status via `runtime::current_project_config_hash`; if `yes`/`no`, notice guidance referencing `trust` typed at startup (M22) — do not duplicate hash-write logic.
- `/reload` → notice `"reloading skills + prompts..."` then call existing skill discovery (`agent.set_skills` reload is CLI-owned; TUI can at least re-read prompt tiers on next turn) — minimal: push notice and invalidate picker.
- `/export`, `/import`, `/share`, `/copy`, `/name`, `/changelog`, `/hotkeys`, `/settings`, `/scoped-models` → notices with honest deferred wording: `"export: use --export <path> or /fork for JSONL (HTML export in 012/016)"` etc. Must NOT return `Unknown`.

All new arms push via `self.ui.push_notice` and return `false` (only `Quit` returns `true`).

**Verify**: `cargo check -p epsilon-tui` → exit 0. `cargo test -p epsilon-tui --lib` → all pass. Manual: typing `/new`, `/session`, `/tree`, `/trust`, `/reload` no longer shows `unknown command`.

### Step 4: Update /help to reflect the completed registry

Ensure `Command::Help` lists every `SLASH_ITEMS` entry (or delegates to a helper that iterates `SLASH_ITEMS`). Update the notice text to not hardcode the old 8-item list. Keep existing key hints.

**Verify**: `grep -n "SLASH_ITEMS" crates/epsilon-tui/src/app.rs` shows help iterating the registry. `cargo fmt --all -- --check` → exit 0.

## Test plan

Model after `crates/epsilon-tui/src/app.rs` `slash_*` tests and `picker.rs` `parse_command` tests:

- `parse_command_covers_all_pi_builtins` — assert `parse_command("/new")`, `"/resume"`, `"/session"`, `"/tree"`, `"/clone"`, `"/trust"`, `"/reload"`, `"/export"`, `"/share"`, `"/copy"` each return `Some` and not `Unknown`.
- `run_command_new_and_session_are_not_unknown` — call `run_command` with `Command::New` / `Command::Session`, assert `ui` notices contain expected substrings and return `false`.
- `help_lists_every_slash_item` — run `Command::Help`, assert notices count ≥ `SLASH_ITEMS.len()`.

Pattern: `tab_completes_highlighted_candidate` in `app.rs:3092` for harness shape.

**Verify**: `cargo test -p epsilon-tui --lib` → all pass including 3 new tests.

## Done criteria

Machine-checkable. ALL must hold:
- [ ] `grep -c 'value: "/new"' crates/epsilon-tui/src/picker.rs` == 1 and similarly for `/resume`, `/session`, `/tree`, `/trust`, `/reload`, `/export`, `/share`, `/copy`
- [ ] `cargo test -p epsilon-tui --lib` exits 0; new tests for registry parity exist and pass
- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets` exit 0
- [ ] `grep -rn "Command::Unknown" crates/epsilon-tui/src/picker.rs` still present only for truly unknown inputs; no pi builtin maps to Unknown
- [ ] No files outside the in-scope list modified (`git status`)
- [ ] `docs/plans/README.md` status row for 013 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:
- The code at the locations in "Current state" doesn't match the excerpts (drift since `e200815`).
- `SLASH_ITEMS`/`Command`/`parse_command` signatures differ in a way that requires a TUI framework swap or new crate.
- `SessionStore` APIs needed for `/session` (`path`, `leaf_id`, `branch_entries`) are absent — report, do not add persistence logic.
- A step's verification fails twice after a reasonable fix.

## Maintenance notes

- If plan 015 (trust/reload) ships, revisit `/trust` and `/reload` arms to wire real hash-accept and live config reload instead of notices.
- If a full session-switch lifecycle lands (new SessionStore file + Agent reattachment), `/new` and `/resume` should switch from notices to real store swaps — keep the slash names stable.
- Reviewers should check that no new command silently prompts the model with literal `/typo` — every leading `/` must map to `Some(Command::*)`.
