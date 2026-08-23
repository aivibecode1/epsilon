# M1 — Workspace Scaffold

Date: 2026-08-23 · Status: complete

## What was built

- Root `Cargo.toml`: workspace with resolver 3 (required by edition 2024), shared
  `[workspace.package]` keys, and a single source of truth for lints.
- 11 member crates under `crates/`, each an empty lib that compiles clean:
  `epsilon-{contracts,llm,session,core,tools,safety,ext,mcp,tui,server,cli}`.
- Release profile: thin LTO + symbol strip (binary size matters for distribution, ADR-001).

## Key decisions made while coding

1. **Lints live in the workspace table**, not per crate. One place to tighten; crates can
   override locally later if a genuine need appears (e.g., the future Windows sandbox backend
   may need to relax `unsafe_code`).
2. **`unsafe_code = "forbid"` workspace-wide from day one.** Cheapest possible safety posture;
   exceptions must be deliberate per-crate overrides, never accidents.
3. **`missing_docs = "warn"`** — this project is written to be read (see `docs/README.md`);
   public APIs without docs surface as warnings at every build.
4. No dependencies added yet. Exact-pinned deps (`=x.y.z`) enter via
   `[workspace.dependencies]` in the milestone that needs them (ADR-016 policy).

## Gotchas hit

- PowerShell here-string generation produced files without trailing newlines; `cargo fmt --check`
  caught it immediately — first proof the quality gate works.

## Verification (all green)

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets   # no warnings
cargo test --workspace                   # compiles all targets
```

## Next

M2 fills `epsilon-contracts` with the ported pi vocabulary.
