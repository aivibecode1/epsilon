# M35 — pi-parity P0 batch 3: auth store hardening (file lock, atomic writes, no-clobber)

Third execution batch against the 2026-09-03 parity audit. Single target:
audit S13 (auth file locking + fail-open wipe). No new dependencies (std-only
lock via `create_new`); the grapheme-correctness P0 (U1) is scoped as M36 —
it needs an editor coordinate migration, not a side fix (see below).

## What landed (`crates/epsilon-llm/src/auth.rs`)

1. **Cross-process `auth.lock`.** Every read-modify-write (`save`, `remove`)
   holds an `auth.lock` created with atomic `create_new`: CLI + TUI saving at
   once can no longer interleave and drop each other's providers. Stale locks
   (mtime ≥ 30s, pi's async threshold) are stolen so a crashed holder never
   wedges logins; otherwise 10 backed-off retries then a loud timeout error.
   The guard removes the file on drop, panic included.
2. **Atomic publish.** Writes go to same-dir `auth.tmp` + `rename` (atomic on
   both platforms): a crash mid-write leaves the previous file intact, never
   a torn `auth.json`. 0600 lands on the tmp *before* the rename, so the
   secret is never visible at default perms even briefly; on unix a chmod
   failure is now an error (pi enforces), not best-effort.
3. **Corrupt-file no-clobber.** Reads stay fail-open (advisory contract,
   existing test pinned), but writes REFUSE a present-but-unparsable file
   with "back it up or delete it first" instead of loading `{}` and wiping
   every provider on save. Same rule in `remove` (a corrupt file is evidence,
   not an empty map). `remove` on a missing file still returns `Ok(false)`
   without touching the disk or the lock.

## Key decisions

- No `fs2`/`proper-lockfile` crate: `create_new` + mtime + drop-guard is the
  whole mechanism in ~50 lines, zero supply-chain surface for a secrets path.
- Lock-then-validate order: `create_dir_all` → lock → corrupt check → write,
  so the corrupt check itself races nothing.
- U1 deferred to M36 deliberately: the editor addresses everything in `char`
  indices across `app.rs` (~5k lines), undo snapshots, kill ring, and paste
  markers. Grapheme safety needs boundary-snapped movement + cluster-consuming
  deletes on that whole surface (`unicode-segmentation` 1.13.3 is already in
  the lockfile, so no network needed) — a milestone, not a drive-by.

## Gotchas

- Windows Defender path-blocked the default-feature llm lib-test exe
  (`target/debug/deps/epsilon_llm-59b4dd66….exe`): `touch` on that exact path
  gives permission denied while sibling names work, with no process holding
  it. Same-hash rebuilds (`cargo clean -p`) would reuse the blocked name, so
  verification ran under `--features real` (different artifact hash; due
  anyway per AGENTS.md). Default-feature re-run pending Defender releasing
  the path — retry before calling the suite fully green.
- An early `cargo test` failure in this batch was the link block, not the
  code: always read the error body before assuming a test regressed.

## Verification

- 3 new tests green: `corrupt_file_blocks_writes_without_clobber` (save AND
  remove refuse, bytes byte-identical), `stale_lock_is_stolen_fresh_lock_blocks`
  (60s-old lock stolen + released; live lock fails loudly, persists nothing),
  `concurrent_saves_keep_every_provider` (8 threads, all 8 entries survive).
- Full `cargo test -p epsilon-llm --features real --lib`: 42/42.
- `cargo fmt --check`: clean. `cargo clippy -p epsilon-llm --all-targets`:
  zero warnings.
- Public signatures unchanged (`save/remove/load`), so CLI/TUI login flows
  need no edits; the new corrupt error surfaces through their existing
  `Result<String>` paths.
