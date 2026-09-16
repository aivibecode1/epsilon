# Plan 002: Loop integrity — overflow latch reset, drained-queue restore, UTF-8 safe clipping/snippets

> **Executor instructions**: Step-by-step; verify each gate. STOP conditions at bottom.
>
> **Drift check**: `git diff --stat 64e927f..HEAD -- crates/epsilon-core/src/lib.rs crates/epsilon-core/src/compact.rs crates/epsilon-session/src/search.rs`

## Status

- **Priority**: P1 · **Effort**: M · **Risk**: LOW-MED
- **Depends on**: none · **Category**: bug
- **Planned at**: commit `64e927f`, 2026-08-24

## Why this matters

Four independent HIGH-confidence bugs: (1) the overflow-retry latch
(`overflow_retried`) is set once and never cleared, so ADR-007 trigger #2 works only
on the FIRST run in a process — every later long-session run dies on overflow without
compacting; (2) steering/follow-up messages already drained from the queues are lost
without trace when a run aborts or errors seconds later; (3) compaction clips tool
output with byte slicing (`&txt[..CAP]`) which panics on multi-byte chars (CJK,
emoji) whose byte 2000 falls mid-character; (4) `search.rs::snippet_for` finds
`byte_pos` in a lowercased copy but slices the ORIGINAL string — `to_lowercase()` can
change byte length (Turkish İ), so offsets diverge and slicing panics.

## Current state

- `crates/epsilon-core/src/lib.rs:216` field; `:257` init false; `:538` swap(true);
  contrast `:442-444` where `run()` clears `doom_streaks` but NOT this flag.
- `crates/epsilon-core/src/lib.rs:492,600` drain into local `pending`;
  abnormal exits at `:501-504` (abort), `:506-509` (budget) `break 'outer` leaving
  `pending` unflushed; `:527` `Err` return likewise.
- `crates/epsilon-core/src/compact.rs:100-104` — `&txt[..CAP]` raw byte slice.
- `crates/epsilon-session/src/search.rs:103-127` — `snippet_for` offset mismatch.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Crate tests | `cargo test -p epsilon-core -p epsilon-session` | all pass |
| Gates | fmt --check + clippy + workspace tests | clean |

## Scope

**In scope**: `lib.rs`, `compact.rs`, `search.rs`, their test modules.
**Out of scope**: queue semantics redesign, compaction algorithm changes beyond the
clip fix, anything touching the provider layer.

## Steps

1. In `run()` next to the `doom_streaks.clear()` block (~line 442), also store
   `false` into `overflow_retried`. Add test: two consecutive runs each hitting an
   overflow script both recover (second must not end with raw provider error).
2. Add `push_front_batch(&mut self, msgs: Vec<Message>)` to the pending-queue type
   (`lib.rs:81-115` area). On each abnormal exit path inside `run_inner`
   (abort break, budget break, error return), re-queue remaining `pending` messages
   to the FRONT of the queue they came from (steering vs follow-up is knowable from
   which drain produced them — keep two locals if clearer). Test: steer then abort →
   message still queued afterwards (assert via a follow-up run receiving it).
3. compact.rs clip: replace byte slice with a char-boundary floor walk:
   `let cut = txt.char_indices().rev().find(|(i,_)| *i <= CAP).map(|(i,_)| i).unwrap_or(CAP);`
   (or forward walk) then `&txt[..cut]`. Test: CJK string longer than 2000 bytes
   does not panic and yields ≤ CAP+suffix bytes.
4. search.rs `snippet_for`: compute the snippet window entirely on ONE string. Simplest
   correct form: work on `lower`, find match there, take ±40 CHARS around it in
   `lower` via char_indices, slice `lower` itself for the returned text (case of
   snippet becomes lowercase — acceptable; note it in a comment), guaranteeing
   boundary safety. Alternative (preserve original case): map offsets through
   `char_indices().zip(char_indices())` pairs — only if trivially correct.
   Test: haystack containing "İ" before the needle; query matching after it;
   assert no panic and snippet contains needle.

## Done criteria

- [ ] New tests above exist and pass; full workspace suite green
- [ ] grep confirms no other raw `&str[..usize]` slices added in these files

## STOP conditions

- Queue internals differ materially from excerpts (drift).
- Restoring pending on abort causes a double-run of an already-executed tool call
  (would indicate deeper state machine coupling — report).

## Maintenance notes

The latch reset interacts with future "retry budget" features; keep the reset next to
doom_streaks so they stay paired.
