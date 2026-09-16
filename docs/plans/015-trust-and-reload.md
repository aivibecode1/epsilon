# Plan 015: Trust ack + live reload (G5)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `docs/plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e200815..HEAD -- crates/epsilon-tui crates/epsilon-core crates/epsilon-cli docs/design/epsilon-architecture.md`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: 013 (reuses its `/trust` and `/reload` slash arms)
- **Category**: dx/correctness
- **Planned at**: commit `e200815`, 2026-08-27
- **Issue**: —

## Why this matters

Epsilon persists a trust hash for `.epsilon/config.toml` but has no slash to ack it — pi's `/trust` and `/reload` let users persist policy and hot-reload skills/prompts/config without restarting the process. After editing config/skills, the user currently restarts epsilon to pick up changes. Trust ack + reload closes G5 with <100 lines: a prompt tier re-read, a skill re-scan, and a hash-accept write, all gated behind the existing `TrustDecision` type.

## Current state

Relevant files, each with one line on its role:
- `crates/epsilon-cli/src/runtime.rs` — `load_project_config(workspace) -> (ProjectConfig, TrustDecision, Vec<String>)`, `current_project_config_hash`, `accept_project_config_hash`, `TrustDecision::{Absent,Trusted,Unchanged,Changed{previous}}`, `skill_roots`, `extension_roots`, `discover_extension_manifests`
- `crates/epsilon-core/src/config.rs` — global config load/save, `global_dir()`
- `crates/epsilon-core/src/prompt.rs` — `load_project_instructions(workspace) -> Vec<(String,String)>` (AGENTS.md/CLAUDE.md + MEMORY.md tier), hash-tagged CONTEXT tier
- `crates/epsilon-core/src/skills.rs` — `load_skills(roots)` discovery
- `crates/epsilon-core/src/lib.rs` — `Agent::set_skills`, `Agent::skill_names`, `Agent::attach_session_store`
- `crates/epsilon-tui/src/picker.rs` — `Command::{Trust(Option<String>), Reload}` (added by 013) + `parse_command` arms
- `crates/epsilon-tui/src/app.rs` — `App::run_command` with trust/reload notice stubs from 013; `App` holds `Arc<Agent>`, `workspace_root: PathBuf`
- `references/pi-0.84.2/packages/coding-agent/src/core/slash-commands.ts` — `BUILTIN trust:"Save project trust decision", reload:"Reload keybindings, extensions, skills, prompts, themes, and context files"`

Excerpts as of `e200815` (after 013 lands, trust/reload are notice stubs):

```rust
// crates/epsilon-cli/src/runtime.rs
pub fn load_project_config(workspace: &Path) -> (ProjectConfig, TrustDecision, Vec<String>) { /* reads .epsilon/config.toml + .epsilon/trust.json, returns TrustDecision::Changed{previous} if hash mismatch */ }
pub fn accept_project_config_hash(workspace: &Path, hash: &str) -> Result<(), String> { /* writes .epsilon/trust.json with new hash */ }
pub fn current_project_config_hash(workspace: &Path) -> Option<String> { /* sha256 of current config file */ }
```

```rust
// crates/epsilon-tui/src/app.rs — run_command trust/reload stubs from 013
Command::Trust(_) => { self.ui.push_notice("trust: ..."); }
Command::Reload => { self.ui.push_notice("reloading skills + prompts..."); self.invalidate_picker(); }
```

Repo conventions to honor:
- Trust hash lives in `.epsilon/trust.json` with key `config_sha256`; write is via `accept_project_config_hash` — see `runtime.rs`.
- Skills are `Arc<Agent>::set_skills(Vec<Skill>)` — see `crates/epsilon-core/src/lib.rs:332 set_skills`.
- Prompt tier re-read is `load_project_instructions(&workspace_root)` — see `prompt.rs`.
- Notices via `self.ui.push_notice` (slash replies render next frame).

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Check | `cargo check -p epsilon-tui -p epsilon-core -p epsilon-cli` | exit 0 |
| Lib tests | `cargo test -p epsilon-tui --lib -p epsilon-core --lib` | all pass |
| Full gates | `cargo fmt --all -- --check && cargo clippy --workspace --all-targets` | clean / 0 warnings |

## Scope

**In scope**:
- `crates/epsilon-tui/src/app.rs` (upgrade trust/reload arms from notices to real actions)
- `crates/epsilon-core/src/skills.rs` (only if a small helper like `reload_skills` is cleaner there; otherwise inline in app.rs)
- Tests co-located

**Out of scope**:
- `crates/epsilon-llm/*`, `crates/epsilon-safety/*`, `crates/epsilon-tools/*`
- Keybinding persistence (G6), session export (G7) — separate plans
- Changing `TrustDecision` enum or session file format
- New crates or workspace deps

## Git workflow

- Branch: `advisor/015-trust-reload`
- Commit style: `feat(m33): wire /trust ack and /reload live refresh (G5)`
- One commit; push only when operator instructs.

## Steps

### Step 1: Confirm drift and baseline

Run drift check. Ensure 013 landed (grep for `Command::Trust` and `Command::Reload` in `picker.rs` succeeds).

**Verify**: `grep -n "Trust\|Reload" crates/epsilon-tui/src/picker.rs | head` shows arms. `cargo test -p epsilon-tui --lib 2>&1 | tail -n 20` → pass.

### Step 2: Wire /trust to accept the hash

In `crates/epsilon-tui/src/app.rs` `Command::Trust(tail)`:

```rust
Command::Trust(tail) => {
    let arg = tail.as_deref().unwrap_or("").trim().to_ascii_lowercase();
    if arg.is_empty() {
        let (cfg, decision, _) = epsilon_cli::runtime::load_project_config(&self.workspace_root);
        // show status + guidance
        match decision { Changed{previous} => self.ui.push_notice(format!("trust: config changed (prev {}…); /trust yes to accept, /trust no to keep untrusted", &previous[..8])), _ => self.ui.push_notice("trust: config is trusted") }
        return false;
    }
    if arg == "yes" || arg == "y" {
        if let Some(hash) = epsilon_cli::runtime::current_project_config_hash(&self.workspace_root) {
            match epsilon_cli::runtime::accept_project_config_hash(&self.workspace_root, &hash) {
                Ok(()) => self.ui.push_notice(format!("trust accepted — hash {}… saved", &hash[..8])),
                Err(e) => self.ui.push_notice(format!("trust accept failed: {e}")),
            }
        } else { self.ui.push_notice("trust: no config file to accept"); }
    } else if arg == "no" || arg == "n" {
        self.ui.push_notice("trust: not accepted — config stays untrusted (edits still require /trust yes)");
    } else { self.ui.push_notice("usage: /trust [yes|no]"); }
}
```

Handle `tail: Option<String>` vs `String` as 013 actually defined — normalize via `tail.as_deref()`.

**Verify**: `cargo check -p epsilon-tui` → 0.

### Step 3: Wire /reload to live-refresh skills + prompt tier

In `Command::Reload`:

```rust
Command::Reload => {
    let roots = epsilon_cli::runtime::skill_roots(&self.workspace_root);
    let (skills, diags) = epsilon_core::skills::load_skills(&roots); // or existing discovery helper
    agent.set_skills(skills.clone());
    self.ui.push_notice(format!("reload: {} skill(s) loaded{}", skills.len(), if diags.is_empty() { "" } else { format!(", {} warning(s)", diags.len()) }));
    // prompt tier will be re-read on next turn's build() — no explicit invalidation needed
    self.invalidate_picker();
}
```

If `load_skills` is async or takes different args, adapt — prefer calling the same discovery path CLI used at startup (`runtime::discover_skills` or `skills::load`). Keep it <20 lines; if discovery is CLI-only, inline the `skill_roots` walk.

**Verify**: `cargo check -p epsilon-tui -p epsilon-core -p epsilon-cli` → 0. `cargo test -p epsilon-tui --lib` → pass.

### Step 4: Update /trust + /reload SLASH_ITEMS descriptions to live wording

If 013 used deferred wording (`trust deferred / reload deferred`), update to `"Save project trust decision for future sessions"` and `"Reload skills, prompts, and config without restarting"` (pi wording). Otherwise keep as-is.

**Verify**: `grep -n "trust\|reload" crates/epsilon-tui/src/picker.rs` shows live descriptions.

### Step 5: Tests

Add in `crates/epsilon-tui/src/app.rs` `#[cfg(test)]`:

- `trust_yes_persists_hash` — create temp `.epsilon/config.toml`, run `/trust yes`, assert `trust.json` now contains `config_sha256` matching `current_project_config_hash`.
- `reload_refreshes_skills` — create temp skill root with a `SKILL.md`, attach agent with 0 skills, run `/reload`, assert `agent.skill_names()` now non-empty.

Use `tempfile::tempdir` pattern from `crates/epsilon-core/tests/*`.

**Verify**: `cargo test -p epsilon-tui --lib` → all pass including 2 new.

### Step 6: Doc one-liner

In `docs/design/epsilon-architecture.md` §6 or §11 add: `Trust is accepted via /trust yes (persists hash), and /reload re-scans skills and re-reads prompt tiers without restart.` One sentence.

**Verify**: `grep -q "/trust yes" docs/design/epsilon-architecture.md` → hit.

### Step 7: Gates

**Verify**: `cargo fmt --all -- --check` → 0. `cargo clippy --workspace --all-targets` → 0 warnings. `cargo test --workspace --lib` → all pass.

## Test plan

- New tests per Step 5.
- Verification: `cargo test --workspace --lib 2>&1 | tail -n 20` → all pass.

## Done criteria

Machine-checkable. ALL must hold:
- [ ] `grep -q "accept_project_config_hash" crates/epsilon-tui/src/app.rs`
- [ ] `grep -q "set_skills\|load_skills" crates/epsilon-tui/src/app.rs`
- [ ] `cargo test -p epsilon-tui --lib` exits 0 with new trust/reload tests passing
- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets` exit 0
- [ ] No files outside the in-scope list modified (`git status`)
- [ ] `docs/plans/README.md` status row for 015 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:
- 013 did not land (no Trust/Reload variants) — dependency not met.
- `runtime::load_project_config` / `accept_project_config_hash` signatures differ, requiring trust.json format change.
- Skill discovery helper is not callable from TUI crate (circular dep) — report, do not duplicate walker.
- A step's verification fails twice after a reasonable fix.

## Maintenance notes

- If keybinding persistence (G6) lands, `/reload` should also re-read `keybindings.json`.
- If live config reload (re-applying ruleset without restart) is needed, `/reload` is the hook — currently it only re-reads tiers, not the permission engine's layered ruleset.
- Reviewers: verify `/trust yes` writes only the hash, not the config file itself.
