# Plan 004: Secrets don't leak — grep/.env bypass, gate subject normalization, session-memory vs deny

> **Drift check**: `git diff --stat 64e927f..HEAD -- crates/epsilon-tools/ crates/epsilon-core/src/exec.rs crates/epsilon-safety/src/gate.rs`

## Status

- **Priority**: P1 · **Effort**: M · **Risk**: MED
- **Depends on**: none · **Category**: security
- **Planned at**: commit `64e927f`, 2026-08-24

## Why this matters

SECURITY-03: the headline secrets rule (`read *.env -> ask`) is bypassed three ways:
grep prints matching `.env` LINES into the transcript ungated; glob/ls list `.env`
ungated; and the gate matches raw subject strings so `read .env/`, `./.env`,
`src/../.env` each behave inconsistently. SECURITY-07: a session "always allow"
promotion stored early outlives later deny rules because memory is checked BEFORE
engine evaluation — policy tightening mid-session doesn't revoke it.

## Current state

- `crates/epsilon-tools/src/search.rs:95-114` grep scans walker output incl.
  hidden files; `util.rs:201-221` asserts `.env` stays searchable (test will need
  updating deliberately).
- `crates/epsilon-core/src/exec.rs:91-116` subjects are raw arg strings;
  `:118-124` bash subject = first-token head.
- `crates/epsilon-safety/src/gate.rs:63-74` memory checked before engine; deny in
  engine never sees memory-promoted pairs.

## Scope

**In scope**: exec.rs subject derivation, gate.rs evaluation order, grep tool
filtering, tests. **Out of scope**: glob/ls filtering (record as follow-up),
canonicalization via filesystem resolve (lexical only, per M6 documented posture).

## Steps

1. Gate order fix (smallest, highest value): in `gate.rs::authorize`, evaluate the
   engine FIRST for Deny; memory consulted only to convert Ask→Allow. Test: promote
   ("bash","git") once → add project-layer deny for same pair → next call must be
   denied (memory must not win).
2. Subject normalization: before rule matching, normalize path-ish subjects
   lexically (strip leading `./`, collapse `a/../`, strip trailing `/`) via a small
   pure fn in exec.rs (reuse patterns from tools/util.rs). Session-memory keys use
   the SAME normalized string (fixes double-prompt inconsistency).
3. Grep guard: after collecting matches but BEFORE emitting lines, skip lines whose
   source file's file-name matches the engine's sensitive classes by consulting the
   same defaults used by the read-gate (simplest honest version: skip files named
   `.env`, `.env.*` except `.env.example`; emit a notice line
   "[grep] skipped <name> (secrets class)"). Update util.rs test that asserts .env
   searchability — its new assertion: skipped-with-notice.
4. Tests for each step; keep them table-driven where natural.

## Done criteria

- [ ] `grep API_KEY` over a workspace containing `.env` emits no secret content and
      one skip-notice line
- [ ] Memory cannot override a fresh deny (new gate test)
- [ ] `read ./x/.env` and `read x/.env` share one approval decision

## STOP conditions

- Normalizing subjects breaks existing rule tests beyond mechanical updates.
- The grep emit pipeline has no single post-collection point to filter (report the
  actual structure).

## Maintenance notes

glob/ls filtering deferred deliberately — record in plans/README rejected-notes if
you assess cost > benefit at this scale.
