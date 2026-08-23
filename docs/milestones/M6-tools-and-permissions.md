# M6 — Real tools + permission ruleset engine

Date: 2026-08-23 · Status: complete

## What was built

**epsilon-tools** (new code in previously-empty crate):

| Tool | Highlights |
|---|---|
| `read` | numbered lines, offset/limit paging, 2000-line/256KB caps, binary refusal |
| `write` | create-or-overwrite, parent-dir creation, created-vs-overwrote reported |
| `edit` | exact-match replacement; ambiguous match fails WITH count; not-found fails with closest-line hint; replace_all escape hatch |
| `bash` | PowerShell (Windows) / bash -c (POSIX), timeout kill ladder, combined output, tail-truncation + spill locator, exit codes surfaced as information not run-stoppers |
| `grep` | Rust regex, workspace walker (skips .git/target/node_modules, keeps hidden files — `.env` stays visible so its read-gate matters), 200-match cap, forward-slash paths for stable rules |
| `glob` | dialect: `**/` = zero-or-more dirs, `*` within segment, bare names match at any depth |
| `ls` | shallow listing with sizes |

Shared `util`: lexical path resolution + containment check (no fs canonicalize → no
symlink surprises), capped walker, spill/truncate helper.

Schemas derive from typed arg structs via schemars (`schema_for!`); runtime validation is the
SAME struct via serde deserialization — one source of truth, friendly errors (ADR-004).

**epsilon-safety** (filled):

- `wildcards.rs` — iterative glob matcher (no exponential backtracking).
- `engine.rs` — `Rule {capability, pattern, action}` layers, **last-match-wins PLUS monotonic
  deny**: a deny anywhere wins immediately regardless of layer order (DSH guard principle
  applied to opencode's stacking). Defaults: reads allow, `.env*` ask (`.env.example` re-allow),
  writes allow (workspace enforcement is M10's confinement job — documented gap), **bash asks**
  until Layer 2 exists, unknown capabilities ask.
- `RulesetGate implements core::PermissionGate` — session memory (always-allow promotions),
  engine evaluation, `ApprovalUi` trait for frontend-owned dialogs, once/always/deny-with-
  guidance flow.

**Core seam upgrade:** `PermissionGate::decide` became `authorize() -> Authorization{decision,
asked}`. The loop now gates EVERY registered call (not just doom-loops): capability = tool name,
pattern = path arg / command head / `*`; paths resolving outside `Agent.workspace_root`
downgrade to an `external_dir` capability pre-execution. Events fire only when `asked == true`.

## Test-caught bugs (again — the methodology paying rent)

1. **Tools ran against the process cwd, not the declared workspace.** The env-read scenario
   failed because `run_one` built `ToolContext.cwd` from `std::env::current_dir()` while gating
   used `Agent.workspace_root`. Two sources of truth; now one. Exactly the class of bug the
   cross-brick scenario existed to catch.
2. **Defense-in-depth proven by test**: even when the UI *approves* an external write, the tool
   itself still refuses — gate and executor enforce independently.
3. MSRV lint caught `floor_char_boundary` (stable 1.91 > our 1.85) before it could break older
   toolchains; replaced with a manual boundary walk.
4. Glob semantics fixed by test: `**/*.rs` must match top-level files (zero directories).

## Deliberate v1 simplifications (documented, revisit later)

- Command analysis = first-token head, not tree-sitter grammars (opencode-grade parsing lands
  with M10 confinement work where it can be tested against real policies).
- Lexical path containment only; symlink escapes are a Layer-2 concern.
- Denial guidance rides inside blocked tool-result text; richer structured feedback waits for
  the prompt-tier work in M7.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 56 passed (15c /12l /11s+4s-it /7core /4t-unit /8t-it... see suite)
cargo run -p epsilon-cli                # write→read→grep demo completes in tempdir workspace
```

Suite totals: contracts 15 · llm 12 · safety 11+4 · core 7 · session 1+6 · tools 4+8.

## Next

M7 — compaction engine + three-tier byte-stable prompt builder (the agent-contract asset gets
include_str!'d here).
