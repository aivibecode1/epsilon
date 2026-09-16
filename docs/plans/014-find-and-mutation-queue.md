# Plan 014: Add find tool + file-mutation queue (G2/G3)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `docs/plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e200815..HEAD -- crates/epsilon-tools`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none (parallel with 013)
- **Category**: correctness/bug
- **Planned at**: commit `e200815`, 2026-08-27
- **Issue**: —

## Why this matters

Epsilon ships `glob` where pi ships `find` (allToolNames includes `find`, not `glob`), so the model cannot do pi's `find` pattern. Worse, `edit` and `write` race when the loop fans out parallel tool calls targeting the same path — pi wraps both with `withFileMutationQueue` serializing per canonical path. Without it, two parallel edits to the same file interleave reads/writes and silently lose one. Adding a `find` tool and a per-path queue closes correctness gap G2+G3 with a few hundred lines and no new deps.

## Current state

Relevant files, each with one line on its role:
- `crates/epsilon-tools/src/lib.rs` — `register_builtins` registers 8 tools (read/write/edit/bash/grep/glob/ls/todo)
- `crates/epsilon-tools/src/search.rs` — `GrepTool`, `GlobTool` (regex walk), `LsTool` + `glob_to_regex` helper
- `crates/epsilon-tools/src/files.rs` — `ReadTool`, `WriteTool`, `EditTool` (edit delegates to `edit_match::plan`)
- `crates/epsilon-tools/src/edit_match.rs` — fuzzy edit chain (exact → lineTrimmed → indentFlexible)
- `crates/epsilon-tools/src/util.rs` — `resolve_in_workspace`, `walk_files`, `read_text_capped`, `MAX_READ_BYTES`, `WALK_MAX_ENTRIES`
- `crates/epsilon-tools/Cargo.toml` — deps: epsilon-contracts/core/safety, serde, tokio, regex, schemars (no walkdir/ignore yet, `util::walk_files` does it)
- `references/pi-0.84.2/packages/coding-agent/src/core/tools/file-mutation-queue.ts` — `withFileMutationQueue(env, path, fn)` per-env WeakMap of per-canonical-path Promise chains
- `references/pi-0.84.2/packages/coding-agent/src/core/tools/find.ts` — `findSchema {pattern, path?, limit?}`, DEFAULT_LIMIT 1000, fd-backed with `relativizeFindResultPath`

Excerpts as of `e200815`:

```rust
// crates/epsilon-tools/src/lib.rs:58
pub fn register_builtins(registry: &mut ToolRegistry, confinement: &Confinement) {
    registry.register(Arc::new(ReadTool::new()));
    registry.register(Arc::new(WriteTool::new()));
    registry.register(Arc::new(EditTool::new()));
    let bash = match sandbox.as_ref() { Some(sb) => BashTool::new().with_sandbox(Arc::clone(sb)), None => BashTool::new() };
    registry.register(Arc::new(bash));
    registry.register(Arc::new(GrepTool::new()));
    registry.register(Arc::new(GlobTool::new()));
    registry.register(Arc::new(LsTool::new()));
    registry.register(Arc::new(TodoTool::new()));
}
```

```rust
// crates/epsilon-tools/src/search.rs — GlobTool already does glob→regex + walk_files
fn glob_to_regex(glob: &str) -> String { /* **/ → (?:.*/)?, * → [^/\\]*, ? → [^/\\] */ }
```

```rust
// crates/epsilon-tools/src/files.rs — Write/Edit do no per-path serialization
async fn execute(&self, call: ToolInvocation, ctx: ToolContext, ...) -> Result<ToolOutput, ToolFailure> {
    let path = ensure_inside(&ctx, &args.path)?;
    // immediate fs write — races if two parallel calls target same path
    std::fs::write(&path, &args.content)?;
}
```

Repo conventions to honor:
- Every tool is a typed struct deriving `JsonSchema` + `Deserialize`, schema via `schemars::schema_for!`, description via `include_str!("../descriptions/<name>.txt")` — see `files.rs:ReadTool` exemplar.
- Read-group tools use `ExecutionMode::Parallel`, mutation tools `Sequential` — `find` joins the read group.
- Workspace containment via `util::resolve_in_workspace(&ctx.cwd, &raw)` + `escaped_workspace` error — see `files.rs:152 ensure_inside`.
- Faux-provider-first: tool tests are offline unit tests in `--lib`, not live FS walks against real LLM.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Check | `cargo check -p epsilon-tools` | exit 0 |
| Lib tests | `cargo test -p epsilon-tools --lib` | all pass |
| Full gates | `cargo fmt --all -- --check && cargo clippy --workspace --all-targets && cargo test --workspace --lib` | clean / 0 warnings / all pass |

## Suggested executor toolkit

- Read `references/pi-0.84.2/packages/coding-agent/src/core/tools/file-mutation-queue.ts` (60 lines) and `find.ts` head for API shape — copy the *shape*, not the JS.
- Skills: `ponytail` — laziest queue is a global `HashMap<PathBuf, Arc<Mutex<()>>>` with `OnceLock`, not WeakMap per-env; reuse `util::walk_files` for find, don't add `ignore` crate yet.

## Scope

**In scope** (the only files you should modify):
- `crates/epsilon-tools/src/find.rs` (new — FindTool)
- `crates/epsilon-tools/src/mutation.rs` (new — per-path queue) or inline in `files.rs` if smaller diff wins
- `crates/epsilon-tools/src/lib.rs` (register find, wire queue into write/edit)
- `crates/epsilon-tools/src/search.rs` (only if extracting `glob_to_regex` for reuse)
- `crates/epsilon-tools/descriptions/find.txt` (new)
- Tests co-located

**Out of scope** (do NOT touch):
- `crates/epsilon-llm/*`, `crates/epsilon-safety/*`, `crates/epsilon-core/*` — unrelated
- Session file format, TUI, CLI
- Adding `walkdir`/`ignore`/`fd` deps — reuse `util::walk_files`
- HTML/markdown or other tooling

## Git workflow

- Branch: `advisor/014-find-and-mutation-queue`
- Commit style: `feat(m33): add find tool + file-mutation queue (G2/G3)`
- One commit for queue, one for find is fine; push only when operator instructs.

## Steps

### Step 1: Confirm drift and scaffold find description

Run drift check. Create `crates/epsilon-tools/descriptions/find.txt` — one paragraph mirroring pi's description: `Find files by glob pattern. Returns matching file paths relative to search dir. Respects .gitignore. Output truncated to 1000 results or 64KB (whichever first).` Keep it short.

Create `crates/epsilon-tools/src/mutation.rs` with a minimal per-path serializer:

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

fn queue_for(path: &Path) -> Arc<Mutex<()>> {
    static MAP: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    // normalize to lexically absolute (no symlink chase at v1) — canonicalize only if exists
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let map = MAP.get_or_init(|| Mutex::new(HashMap::new()));
    // sync fast-path via try_lock fallback — or just use tokio::sync::Mutex for map too
    // ponytail: keep it simple — global Mutex<HashMap> is fine; contention is per-file, not per-turn
    unimplemented!("see plan")
}
pub async fn with_file_mutation_queue<T>(path: &Path, f: impl Future<Output=T>) -> T { let g = queue_for(path).lock().await; let r = f.await; drop(g); r }
```

Prefer the smallest correct version: `OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>` where outer Mutex is `std::sync::Mutex` (blocking, held briefly) or `tokio::sync::Mutex`. Document ceiling with `// ponytail: global lock, per-file mutexes if throughput matters`.

**Verify**: `cargo check -p epsilon-tools` → exit 0.

### Step 2: Implement FindTool

Create `crates/epsilon-tools/src/find.rs`:

```rust
#[derive(Deserialize, JsonSchema)] pub struct FindArgs { pub pattern: String, #[serde(default)] pub path: Option<String>, #[serde(default)] pub limit: Option<usize> }
pub const FIND_DESC: &str = include_str!("../descriptions/find.txt");
pub struct FindTool { descriptor: ToolDescriptor }
```

- `name() -> "find"`, `label() -> "Find"`, `description() -> FIND_DESC`, `execution() -> Parallel`.
- In `execute`: validate `pattern` non-empty; resolve `root` via `util::resolve_in_workspace(&ctx.cwd, args.path.unwrap_or("."))` and `inside` check; reuse `search::glob_to_regex` (extract to `crate::search::glob_to_regex` or duplicate with `pub(crate)` — prefer reuse); apply same bare-pattern `**/` prefix rule as GlobTool; `limit = args.limit.unwrap_or(1000).min(1000)`; collect `util::walk_files(&root).iter().map(|p| p.to_string_lossy().replace('\\', "/")).filter(|rel| re.is_match(rel)).take(limit)`; if empty → `"no matches"`; else `matches.join("\n") + format!("\n[{} file(s)]", n)`. Respect `WALK_MAX_ENTRIES` cap.

Expose `glob_to_regex` as `pub(crate)` if needed.

**Verify**: `cargo check -p epsilon-tools` → exit 0.

### Step 3: Wire mutation queue into Write/Edit

In `crates/epsilon-tools/src/files.rs`, wrap the FS-mutating section of `WriteTool::execute` and `EditTool::execute` with `crate::mutation::with_file_mutation_queue(&path, async { ... }).await`. Keep the `ensure_inside` check outside the queue (it doesn't mutate). The queue key must be the resolved absolute `PathBuf` (same as Find's canonicalization choice).

Keep `ExecutionMode::Sequential` as-is — the queue is the correctness fix even if the scheduler already serializes sequential batch; it also protects against future parallel reclassification.

**Verify**: `cargo check -p epsilon-tools` → exit 0. `cargo test -p epsilon-tools --lib` → pass.

### Step 4: Register find

In `crates/epsilon-tools/src/lib.rs`, add `mod find; mod mutation;` (or `pub mod mutation` if tests need it), `pub use find::{FindArgs, FindTool};`, and in `register_builtins` add `registry.register(Arc::new(FindTool::new()));`. Keep registration order: read/write/edit/bash/grep/glob/find/ls/todo (find beside glob).

**Verify**: `grep -n "FindTool" crates/epsilon-tools/src/lib.rs` shows hit; `cargo check -p epsilon-tools` → exit 0.

### Step 5: Tests

In `find.rs` and `mutation.rs` (or `files.rs`) add `#[cfg(test)]`:

- `find_bare_pattern_matches_at_any_depth` — create temp dir with `a/b/c.rs` and `c.rs`, assert `pattern: "*.rs"` finds both via `**/*.rs` expansion.
- `find_respects_limit` — walk with many files, `limit: 2` returns 2.
- `mutation_queue_serializes_same_path` — spawn two concurrent `with_file_mutation_queue(&same_path, async { sleep(50ms); push })` and assert order is serialized (second starts after first), while different paths run concurrently (use `tokio::join!` + `Instant`).

Model after `crates/epsilon-tools/src/search.rs::tests::glob_translation_covers_dialect`.

**Verify**: `cargo test -p epsilon-tools --lib` → all pass including 3 new.

### Step 6: Full gates

**Verify**: `cargo fmt --all -- --check` → exit 0. `cargo clippy --workspace --all-targets` → 0 warnings. `cargo test --workspace --lib` → all pass.

## Test plan

- New tests listed in Step 5.
- Existing `register_builtins` count test (if any) updated to expect +1.
- Verification: `cargo test --workspace --lib 2>&1 | tail -n 20` → all pass.

## Done criteria

Machine-checkable. ALL must hold:
- [ ] `grep -rn "FindTool" crates/epsilon-tools/src/lib.rs` returns hit and `find` appears in `register_builtins`
- [ ] `grep -rn "with_file_mutation_queue" crates/epsilon-tools/src/files.rs` returns hits in both Write and Edit paths
- [ ] `cargo test -p epsilon-tools --lib` exits 0; new find+queue tests exist and pass
- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets` exit 0
- [ ] No files outside the in-scope list modified (`git status`)
- [ ] `docs/plans/README.md` status row for 014 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:
- The code at the locations in "Current state" doesn't match the excerpts (drift).
- `util::walk_files`/`resolve_in_workspace` signatures differ, requiring a walker rewrite.
- Adding the queue requires a new crate or changing `ToolContext` to carry an env handle.
- A step's verification fails twice after a reasonable fix.

## Maintenance notes

- If `ignore` crate is later adopted for `.gitignore` fidelity, `FindTool` and `GlobTool` must share the same walker config.
- If canonical-path (symlink-resolved) queue keys are needed, switch `queue_for` to `std::fs::canonicalize` with fallback — current lexical key is intentional v1 simplicity.
- Reviewers should check that `Sequential` + queue doesn't double-serialize unnecessarily — queue is the invariant, scheduler mode is the hint.
