# Plan 008: CLI composition cleanup — RunParams struct, banner sink, dead-code retirement

> **Drift check**: `git diff --stat 64e927f..HEAD -- crates/epsilon-cli/ crates/epsilon-session/src/lib.rs`

## Status

- **Priority**: P3 · **Effort**: M · **Risk**: LOW
- **Depends on**: none (do after P1/P2 plans to avoid conflicts) · **Category**: tech-debt / dx
- **Planned at**: commit `64e927f`, 2026-08-24

## Why this matters

DEBT-01/04/06/07 + DEP-02 + DX-01 cluster: `build_runtime` takes 11 params (workspace
passed twice under two names; adding `--plan` cost 9 edits); startup diagnostics use
three ad-hoc Vec<String> patterns plus stray eprintln!s; several "kept for parity"
dead items and a crate-wide `#![allow(dead_code)]` mask real rot (session repair is
documented as consumed but never was); AGENTS.md claims "29+ tests" vs actual 213+;
two unused manifest deps misstate the vocabulary crate's footprint.

## Current state

- `main.rs:601-614` signature; twin workspace args at all 4 call sites.
- `main.rs:349-356`, `:304-307`, `:638-642` three banner/notice collections;
  `:486-491` unreachable TuiBench arm (short-circuited at `:318`).
- `crates/epsilon-session/src/lib.rs:1` crate allow; repair never wired.
- `AGENTS.md:19` "(29+ tests)"; `crates/epsilon-core/Cargo.toml:13` unused serde;
  `crates/epsilon-contracts/Cargo.toml:13` unused tokio.

## Scope

**In scope**: main.rs/runtime.rs, session lib.rs allows, the two manifest lines,
AGENTS.md line, README count conventions. **Out of scope**: splitting app.rs or
main.rs into modules (record as future), wiring repair (separate decision).

## Steps

1. Introduce `struct RunParams` in runtime.rs consuming build_runtime's params
   (single `workspace` field — delete the duplicate). All four call sites construct
   it; delete the clippy too_many_arguments allow. The plan-layer construction
   simplifies to `PermissionEngine::layered(vec![plan_ruleset()])`.
2. One `Diagnostics` collector (push + flush_stderr) replacing banners/project_notices/
   ext_notices; drain once in dispatch. Remove unreachable TuiBench arm.
3. epsilon-session: replace crate-level allow with targeted ones carrying reasons;
   fix repair.rs's false "consumed from M5" comment (state honestly: test-only until
   a milestone wires it).
4. Delete unused serde (core) and tokio (contracts) manifest lines — verify with
   `cargo check --workspace --all-targets`.
5. AGENTS.md: drop the stale number ("full suite must stay green"); README keeps one
   exact-at-milestone convention.

## Done criteria

- [ ] No duplicate-workspace param; no too_many_arguments allow in cli
- [ ] Exactly one diagnostics drain path; grep shows no bare eprintln! in build_runtime
- [ ] Manifests carry no unused deps (compiler-verified)

## STOP conditions

- RunParams refactor reveals call-site behavior differences between modes beyond
  parameters (report before unifying anything).
