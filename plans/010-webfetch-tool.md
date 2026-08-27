# Plan 010: Ship webfetch as a read-group tool (G-M5 closure)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat abd045a..HEAD -- crates/epsilon-tools crates/epsilon-safety crates/epsilon-llm design/epsilon-architecture.md`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none (can run parallel to 009)
- **Category**: direction (stated-but-undelivered)
- **Planned at**: commit `abd045a`, 2026-08-27
- **Issue**: —

## Why this matters

`design/epsilon-architecture.md` §4 lists `webfetch` as a built-in and
`crates/epsilon-safety/src/engine.rs:124` already has
`Rule::new("webfetch", "*", Action::Ask)` — but `crates/epsilon-tools/src/lib.rs:58`
`register_builtins` registers only 8 tools (read/write/edit/bash/grep/glob/ls/todo)
and `design/epsilon-architecture.md` was patched in m31 to call webfetch
"deferred". Every reference ships a fetch (opencode `webfetch.ts` + `webfetch.txt`,
pi extensions, DSH `guard/timeout-policy`). Doc-code drift is a finding per
the improve audit playbook (stated-but-undelivered). Closing it makes the
architecture doc honest again and gives the agent a safe way to pull remote
context without shelling out to curl inside bash.

## Current state

Relevant files, each with one line on its role:
- `crates/epsilon-tools/src/lib.rs` — `register_builtins` registers the 8 built-ins; `Confinement` enum routes bash through sandbox
- `crates/epsilon-tools/src/util.rs` — spill helpers, `MAX_READ_BYTES`, head/tail preview logic used by read tool
- `crates/epsilon-tools/Cargo.toml` — deps: epsilon-contracts/core/safety, serde, tokio, reqwest already via workspace (`=0.13.4` default-features false json/stream), regex, schemars
- `crates/epsilon-safety/src/engine.rs` — `Rule::new("webfetch", "*", Action::Ask)` at line 124; `evaluate("webfetch", "*") == Ask` tested at line 192
- `crates/epsilon-safety/src/wildcards.rs` — opencode-parity glob (m31 regex impl; `*`→`.*`, `?`→`.`, backslash→`/`)
- `crates/epsilon-llm/src/http.rs` — existing `reqwest` usage, `HttpEventStream`, timeout/retry patterns; `Cargo.toml` already has `reqwest = { workspace = true }` via llm
- `crates/epsilon-contracts/src/tool.rs` — `Tool` trait (`name/label/description/schema/execution/execute`), `ToolContext { session_id, cwd, spill_dir, token, on_update }`, `ExecutionMode::Parallel` vs `Sequential`
- `design/epsilon-architecture.md` §4 — currently reads "webfetch — planned, see G-M5 — would join the read group" and "webfetch deferred (rule exists in epsilon-safety, ships next milestone)"
- `references/opencode-1.18.21/packages/core/src/tool/webfetch.ts` (reference, 240 lines head) + `packages/opencode/src/tool/webfetch.txt` (prompt) — opencode prompt + fetch logic to mirror

Excerpts as of `abd045a`:

```rust
// crates/epsilon-tools/src/lib.rs:58
pub fn register_builtins(registry: &mut ToolRegistry, confinement: &Confinement) {
    // ReadTool, WriteTool, EditTool, BashTool, GrepTool, GlobTool, LsTool, TodoTool
}
```

```rust
// crates/epsilon-safety/src/engine.rs:124
Rule::new("webfetch", "*", Action::Ask),
```

```rust
// design/epsilon-architecture.md §4 (deferred wording)
| epsilon-tools | Built-in tools: read, write, edit, bash, grep, glob, ls, todo, task(subagent) — webfetch deferred (rule exists in epsilon-safety, ships next milestone) |
(read/grep/glob/ls yes; webfetch — planned, see G-M5 — would join the read group; write/edit/bash/todo/task no)
```

Repo conventions to honor:
- Every tool is a typed struct deriving `JsonSchema` + `Deserialize` for params, schema shared via schemars — see `files.rs` ReadTool as exemplar; descriptions live in `descriptions/<name>.txt` via `include_str!` (opencode pattern, ADR-004).
- Concurrency classification: read-group tools declare `ExecutionMode::Parallel` (read/grep/glob/ls), mutation tools `Sequential` — webfetch joins the read group.
- Permission gates: tool name + subject string evaluated via `engine.evaluate("webfetch", url_or_host)`. Keep default Ask as-is; no new rules.
- Spill rule: outputs above `spill_threshold_bytes` (default 256 KiB, see `util.rs`) are written to `.epsilon/spill/<hash>` with head/tail preview + locator (ADR-010, DSH spill). Webfetch must reuse the same spill path.
- `CancellationToken` in `ToolContext` must be respected (select against it on the reqwest future).
- Workspace deps pinned exact (`=x.y.z`) — do not add a new http client; reuse `reqwest` already in workspace.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Check | `cargo check -p epsilon-tools -p epsilon-safety` | exit 0 |
| Lib tests | `cargo test -p epsilon-tools --lib -p epsilon-safety --lib` | all pass |
| Full gates | `cargo fmt --all -- --check && cargo clippy --workspace --all-targets && cargo test --workspace --lib` | clean / 0 warnings / all pass |
| Manual | `cargo run -p epsilon-cli -- -p "fetch https://example.com and summarize"` (with faux or real) | webfetch call appears in transcript, result truncated + spill if large |

## Suggested executor toolkit

- Read `references/opencode-1.18.21/packages/core/src/tool/webfetch.ts` lines 1–240 and `packages/opencode/src/tool/webfetch.txt` for prompt wording; copy the *shape*, not the code.
- Skills: `improve` security categories (data crossing into interpreters, SSRF surface) when reviewing URL validation.
- Keep the ponytail heuristic: laziest solution that actually works — a single `WebFetchTool` with one reqwest GET, no HTML→markdown heavy pipeline at v1 (raw text + content-type aware truncation is enough).

## Scope

**In scope** (the only files you should modify):
- `crates/epsilon-tools/src/webfetch.rs` (new file: `WebFetchTool` impl)
- `crates/epsilon-tools/src/lib.rs` (register it, export)
- `crates/epsilon-tools/descriptions/webfetch.txt` (new: one-paragraph tool description for the model)
- `crates/epsilon-tools/Cargo.toml` (only if a feature flag on reqwest needs enabling; prefer reusing workspace dep as-is)
- `design/epsilon-architecture.md` §4 table + §4 concurrency note (flip deferred → shipped, keep one line)
- Tests co-located in `webfetch.rs` + a fixture test in `lib.rs` if needed

**Out of scope** (do NOT touch, even though they look related):
- `crates/epsilon-safety/src/engine.rs` — rule already exists; do not change defaults.
- `crates/epsilon-llm/src/http.rs` — provider HTTP stays separate; webfetch owns its own reqwest client (small, isolated).
- Any new crate or workspace dep beyond what is already pinned.
- HTML→markdown converters, headless browsers, JS rendering — explicitly deferred; raw body + truncation is v1.

## Git workflow

- Branch: `advisor/010-webfetch-tool`
- Commit style: `feat(m32): ship webfetch tool (read-group, Ask-gated, spill-aware)`
- One commit for tool + registration, one for doc flip is acceptable; push only when operator instructs.

## Steps

### Step 1: Confirm drift and scaffold the tool file

Run drift check. Create `crates/epsilon-tools/src/webfetch.rs` with the typed param struct:

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WebFetchParams {
    /// URL to fetch (http/https only).
    pub url: String,
    /// Optional max characters to return inline before spill (default: spill threshold).
    #[serde(default)]
    pub max_chars: Option<usize>,
}
```

and `descriptions/webfetch.txt` with one paragraph (mirror opencode's webfetch.txt: "Fetch a URL and return its content as text. Use for docs, APIs, ... Respect ...").

**Verify**: `cargo check -p epsilon-tools` → exit 0 (struct compiles, no logic yet).

### Step 2: Implement WebFetchTool::execute

Implement `Tool` for `WebFetchTool`:

- `name() -> "webfetch"`, `label() -> "Fetch URL"`, `description() -> include_str!("../descriptions/webfetch.txt")`, `schema() via schemars`, `execution() -> Parallel`.
- In `execute`:
  1. Validate URL: must parse, scheme is `http` or `https` only; reject `file:`, `javascript:`, private-range hosts are allowed at v1 (no SSRF blocklist yet — document as known limitation; permission Ask gate is the control).
  2. Build a `reqwest::Client` with 15s timeout, `user_agent("epsilon-webfetch/0.1")`, redirect limit 5 (reqwest default is 10 — clamp to 5), `connect_timeout(5s)`.
  3. `tokio::select!` the GET future against `ctx.token.cancelled()` — on cancel return `ToolFailure` with `isError: true` and message "webfetch cancelled".
  4. Check `status.is_success()` — on non-2xx return `ToolOutput { content: format!("HTTP {status}: {snippet}"), isError: true }` where snippet is first 2k chars of body (do not spill errors).
  5. Read body as bytes with a 5 MiB cap (abort if `content-length` > cap or streaming exceeds cap) — protects the spill dir from unbounded fetches.
  6. Detect `content-type`: if `text/*` or `application/json`/`application/xml` treat as UTF-8 lossy; if `text/html` at v1 return raw text (no HTML stripping — follow-up can add a lightweight stripper). Binary types return a short note with headers + size, not the bytes.
  7. Apply spill: if `body_text.len() > spill_threshold` (reuse `util::spill_threshold_bytes()` or the constant in `util.rs`), write full body to `ctx.spill_dir/<sha256(url)>.txt` and return `ToolOutput { content: format!("{head}\n...\n[spilled to {path} — {total} bytes, tail: {tail}]"), details: json!({ "spilled": true, "path": path, "bytes": total }) }`. Otherwise return full body inline.
  8. Call `ctx.on_update` once with a `ToolPartial` progress note if body > 50 KiB (optional, throttled — keep it simple, one call).

Reuse `util::truncate_with_ellipsis` / spill helpers already in `util.rs` — do not duplicate.

**Verify**: `cargo check -p epsilon-tools` → exit 0. `cargo test -p epsilon-tools --lib webfetch` (new tests, next step) will exercise it.

### Step 3: Register the tool and flip the doc

In `crates/epsilon-tools/src/lib.rs` add `mod webfetch;` and in `register_builtins` push `WebFetchTool` (same pattern as `ReadTool` etc.). Update `builtin_registry` test helper if it asserts count. In `design/epsilon-architecture.md` flip the two deferred lines:

- Table row → `read, write, edit, bash, grep, glob, ls, todo, task(subagent), webfetch`
- Concurrency note → `(read/grep/glob/ls/webfetch yes; write/edit/bash/todo/task no)`

**Verify**: `grep -n "webfetch" crates/epsilon-tools/src/lib.rs` shows registration; `grep -n "webfetch" design/epsilon-architecture.md` shows shipped wording, no "deferred".

### Step 4: Tests

Add `#[cfg(test)] mod tests` in `webfetch.rs` (pattern: `files.rs` tests):

- `rejects_non_http_scheme` — `file:///etc/passwd` → isError with "only http/https".
- `spills_large_body` — stub a 300 KiB body via an injected fetcher or by calling the truncation helper directly; assert `details["spilled"] == true` and content contains `[spilled to`.
- `respects_cancellation` — create a `CancellationToken`, cancel before execute, assert cancelled error.
- `non_success_status_is_error` — simulate 404 via a test helper that bypasses network (extract status→error mapping into a pure fn `map_status` and test that).

For network-free CI, keep tests offline: extract pure helpers (`validate_url`, `map_status`, `maybe_spill`) and test those; add one `#[ignore] live_fetch_example_com` gated test that hits `https://example.com` when `EPSILON_LIVE_WEBFETCH=1`.

**Verify**: `cargo test -p epsilon-tools --lib` → all pass including new tests. `cargo test -p epsilon-tools --lib -- --ignored` with `EPSILON_LIVE_WEBFETCH=1` passes when network available (optional).

### Step 5: Full gates

**Verify**: `cargo fmt --all -- --check` → exit 0. `cargo clippy --workspace --all-targets` → 0 warnings. `cargo test --workspace --lib` → all pass.

## Test plan

Model after `crates/epsilon-tools/src/files.rs` tests and `crates/epsilon-safety/src/engine.rs` rule tests.
- New `webfetch.rs` tests listed in Step 4.
- Existing `register_builtins` count test (if any) updated to expect +1.
- Verification: `cargo test --workspace --lib` → all pass.

## Done criteria

Machine-checkable. ALL must hold:
- [ ] `grep -rn "WebFetchTool" crates/epsilon-tools/src/lib.rs` returns a hit
- [ ] `grep -rn "webfetch" design/epsilon-architecture.md` shows shipped wording, no "deferred" or "planned, see G-M5"
- [ ] `cargo test -p epsilon-tools --lib` exits 0; new webfetch tests exist and pass
- [ ] `cargo fmt --all -- --check` exits 0 and `cargo clippy --workspace --all-targets` exits 0
- [ ] No files outside the in-scope list are modified (`git status`)
- [ ] `plans/README.md` status row for 010 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:
- The code at the locations in "Current state" doesn't match the excerpts (drift).
- `reqwest` workspace dep lacks the needed features for a simple GET (requires adding a new dep or enabling `rustls` vs `native-tls` switch — report, do not silently change TLS backend).
- Spill helpers in `util.rs` cannot be reused without touching session persistence — report the mismatch.
- Adding webfetch requires a new permission capability beyond the existing `webfetch` rule (it shouldn't).

## Maintenance notes

- Follow-up: add lightweight HTML→text stripping (e.g., `html2text` or pulldown-cmark on fetched markdown) only after measuring real usage; raw body is correct for v1.
- If private-network SSRF blocking is later required, add a `deny_private_ranges` check in `validate_url` behind a config flag — not in this plan.
- Reviewers should scrutinize timeout/cap constants (15s, 5 MiB) — they are the product.
