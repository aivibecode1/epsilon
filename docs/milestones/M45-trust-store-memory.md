# M45 — pi-parity: trust-store persistence (`trust-manager.ts` memory half)

Thirteenth port batch against the 2026-09-03 parity audit (session/auth/config
findings S18/S19 — the second half; M39 did the execution half). M39's
resource scan + curtailed roots answered "untrusted code never loads", but
nothing REMEMBERED the answer: every run re-asked, and a remembered `false`
couldn't deny an unchanged config. pi's `ProjectTrustStore`
(`<agentDir>/trust.json`: path → true/false/null, nearest-ancestor
inheritance, lock-guarded) is the memory.

## What landed

1. **Store** (`epsilon-tui/src/trust_store.rs`, new): `ProjectTrustStore`
   at `<global_dir>/trust.json` with `get`/`get_entry` (nearest ancestor
   wins), `set`/`set_many` (atomic, sorted keys), `clear` (pi's `null`),
   cross-process `trust.json.lock` with stale-steal, tmp+rename writes,
   corrupt-file reads as no-memory + self-heals on next write.
2. **Composition** (`cli/main.rs` dispatch): remembered `false` denies even
   an unchanged config; `true` trusts even a changed one; `None`/unreadable
   falls back to the hash. "No memory is never a decision."
3. **Acceptance writes both** (TUI `trust`/`trust yes` at both gates):
   path → `true` in the store + config hash, each reported separately.
4. **Tests**: 4 store tests (roundtrip, ancestor inheritance + child
   override + clear-fallback, atomic sorted set_many, corrupt-heals).

## Key decisions

- The store lives in **epsilon-tui, not epsilon-cli**: app.rs needed it and
  tui <- cli would cycle the dependency law (cli already depends on tui).
  The config-hash half stays in cli/runtime (per-file integrity); the
  per-path memory lives with its writer. Documented on the module.
- `trust.json` at `<global_dir>` (not per-workspace `.epsilon/`): pi puts it
  in the agent dir for exactly this reason — per-workspace trust files would
  let a cloned repo pre-approve itself. The M39 hash file stays per-project
  (integrity of THAT file); the memory of the ANSWER is global.
- Deliberately NOT ported: `getProjectTrustOptions` (the Trust / Trust-parent
  / session-only / Do-not-trust picker) — the TUI accept path covers
  Trust-now; parent-folder and session-only scoping are enhancement-phase UX.
  The store API (`set_many`, `clear`) already supports them.
- Corrupt store = no-memory (fail OPEN to asking), not fail-closed: a torn
  global file must not brick every project. Same posture as the M35
  auth-store decision, opposite direction — auth guards secrets (fail
  closed), trust guards convenience (fail to asking).

## Gotchas

- First draft put the store in `epsilon-cli/runtime.rs` — compiles, but
  app.rs (epsilon-tui) can't import it without a tui→cli dep. Moved to
  `epsilon-tui/trust_store.rs` before wiring.
- `BTreeMap as TrustMap` alias broke (E0107: alias without generics can't be
  used as a type) — plain `type TrustMap = BTreeMap<String, bool>;` instead.
- `BTreeMap::shift_remove` doesn't exist (that's IndexMap) — `remove`.
- `self.lock()?` in a `Result`-returning fn (E0277: Option vs Result) —
  `.ok_or_else(...)`.
- The `unicode-segmentation` dep went missing from `epsilon-tui/Cargo.toml`
  mid-session (my own earlier Cargo.toml edit reverted it — `git checkout`
  restored the file WITHOUT it since it's uncommitted work). Re-added;
  `cargo check` confirmed. Lesson: `git checkout <file>` in a dirty tree
  destroys uncommitted deps — check `git status` scope first.
- Patch-tool lint noise (`async fn not permitted in Rust 2015`) continues —
  pre-existing host/edition mismatch, ignores throughout.

## Verification

- New: 4 trust_store tests green.
- cli 10 lib, tui 130 lib (126 old + 4 new), fmt clean, workspace clippy zero.
