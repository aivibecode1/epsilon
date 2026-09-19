# SURFACE-05 — paged reads beyond the byte prefix (M95)

## State (2026-09-19)
**Implemented in the working tree (M95, uncommitted)** at base HEAD `6c946f5`: `read` pages with
pi's offset/limit semantics and the `read_log` coverage handoff ships in the same change.
Focused verification done — fmt clean, clippy exit 0, `cargo test -p epsilon-tools` 82+44 passed
/ 0 failed, and a pre-fix RED (7/7 new tests fail against HEAD's four source files, then green
after a byte-identical restore). **The frozen-candidate battery has NOT run**;
`docs/milestones/M95-paged-reads.md` carries the full record (Verification + Limits).

## Evidence (audit/evidence/, all untracked)
- `m95-clippy.log`, `m95-focused-tools.log`, `m95-red.log` (RED + restore + green re-run);
  `m95-focused-rerun.log` (fix round: pi `limit:0` check, ninth-test RED/restore/green, fmt +
  clippy + tests on the post-fix bytes).
- Slice file sha256 (round-1 bytes): `util.rs 4cfe7bcc…`, `read_log.rs 6f60e140…`,
  `files.rs 32641607…`, `search.rs 867d6ada…`, `tests/builtin_tools.rs 4bc728b0…`,
  `descriptions/read.txt fff98dab…` (post-fix-round bytes in the checkpoint section below). (The
  RED run's restore hashes — `util.rs a45f9c9b…` — predate a comment-only `ponytail:` edit; the
  whole focused battery was re-run on the final bytes.)
- Build target used: `$LOCALAPPDATA/Temp/epsilon-m95-target` (~300 MB, left warm; `rm -rf` it
  when the machine needs the space — a fresh dir is required for the battery anyway).

## What shipped
- `util.rs`: `ReadPage` / `PageBound` / `read_page` — one streamed pass (BufReader, one line +
  page resident), the rest counted without retention, NUL refusal on the first 4 KiB, per-line
  decode (window boundary = line boundary), `format_size` (pi `truncate.ts:61`),
  `read_text_capped(_ex)` kept for `grep`.
- `files.rs`: `ReadTool::execute` pages, emits pi's continuation strings / the `sed` fallback,
  additive `details.read`, records `Coverage`; the image magic-byte probe now reads 4 KiB
  instead of the whole file.
- `read_log.rs`: `Coverage { from, to, total, capped }` accumulates lines contiguous from line 1,
  keyed to the stat it was read against; `complete` is the overwrite predicate; `capped` narrowed
  to pi's `firstLineExceedsLimit`.
- `search.rs`: grep's 256 KiB cap announced (`files_capped` + a note that also survives
  "no matches"; the secret-skip note no longer vanishes there either).
- Docs: `descriptions/read.txt`, M65's rule table (amendment), ADR-005 amendment (M95),
  `docs/history.md`, `docs/README.md` (M95 row), `docs/milestones/M95-paged-reads.md`.

## Next steps (in order)
1. Frozen-candidate battery on a NEW label (e.g. `m95a`) + evidence out of `target/`; fill the
   milestone's Verification from the log and reconcile the per-binary test delta.
2. Independent read-only review → `audit/evidence/m95-review.md`; every residual into Limits.
3. Rebuild the global `epsilon.exe` from the verified bytes (CLI-visible slice).
4. Commit as one milestone commit after 1-3 (stage the untracked M95 doc AND the M65 amendment —
   `git clean` would destroy both; do not sweep the owner's other untracked files).

## M95 state checkpoint (2026-09-19 ~12:45, written after two backend restarts)
The implementation is **complete in the working tree** (uncommitted; HEAD `6c946f5`): `util.rs`
+302, `read_log.rs` +255, `files.rs` +142, `search.rs` +49, `tests/builtin_tools.rs` +401,
descriptions + docs + ADR + history + docs-map. Evidence on disk: `audit/evidence/m95-{clippy,
focused-tools,focused-external,red,focused-rerun}.log`; the RED log is the load-bearing one —
pre-fix, one page-1 read of a 5,500-line file granted wholesale overwrite (`overwrote book.txt
(12 bytes)`) and a page cut a multibyte character into U+FFFD.
- The child's own focused runs: clippy exit 0, `epsilon-tools` 82 + 44 passed / 0 failed, fmt
  clean (base HEAD `6c946f5`). My independent re-run was killed by a backend restart mid-tests
  (log truncated) — **not yet re-established**.
- NEXT: battery `m95a` (script `$LOCALAPPDATA/Temp/m95a-round.sh`, fresh target, gate-log
  verdict, flake retry) -> independent review (re-dispatched; the first died one minute in) ->
  fix blockers -> commit -> reinstall (the read/write tool surface is user-visible).
- ENVIRONMENT: two delegations lost their terminal results and one background chain was killed
  within an hour (2026-09-19 11:32-12:40). Assume background work can die: keep rounds scripted,
  keep evidence on disk, and treat `process_manage` "not_found" as the signal to re-run.

## M95 review outcome + fix round (2026-09-19 ~13:00)
The read-only review (full text in `$LOCALAPPDATA/Temp/m95-review.md`) confirmed the central
claim — no CLI/TUI/server/other-crate consumer of the read-before-write state exists, and page 1
of a 5,500-line file now records PartialRead — but found ONE blocker in the same F01 class:
`util.rs:428`'s over-cap branch lacked a `lines.is_empty()` guard, so an over-cap line *mid-page*
made the port replace the whole page with the sed message, drop `of T`/the continuation offset,
and record `capped=true` — a read displaying NOTHING licensed a wholesale overwrite. pi checks
only `lines[0]` (`truncate.ts:103-137`); the code contradicted its own docs.
Fix round done (this session): `lines.is_empty()` gate in `read_page`'s over-cap branch; the new
RED-screened test `a_later_over_cap_line_still_shows_the_page_and_still_refuses_the_overwrite`
(against the reviewed `util.rs 4cfe7bcc…` it failed `Ok("overwrote mixed.txt (9 bytes)")`; restored
byte-identically to `bc8ac787…`, green after); the `limit:0` fold went *against* the review's
premise — an executed run of pi's own read/truncate block shows pi emits the notice with its blank
separator, so the port now matches pi (log section "pi check"); docs corrected (ADR phrase, fmt
citation, new Limits: edit-stamps-complete, usize offsets, sed byteLength); ONE complete focused
log `m95-focused-rerun.log` (fmt exit 0 / clippy exit 0 / tests 82+45 passed, exit 0). Post-fix
bytes: `util.rs bc8ac787…`, `files.rs e2cdcae7…`, `tests/builtin_tools.rs e5339904…` (read_log.rs
and search.rs unchanged). NOT committed.
- Battery `m95a` was stopped for the third time (bytes changed) — its log held only the fmt
  header, so nothing was lost. Re-run as `m95b` after the fix reports, then a delta re-review.
- Carried, not fixed (pre-existing, unchanged vs HEAD): a successful **edit** stamps coverage
  `complete:true`, so read-page-1 -> edit-one-line -> whole-file write is permitted. Queue it.
  (Recorded in M95's Limits this round.)

## M95 round 3 (2026-09-19 ~13:20): blocker fixed, battery + delta review running
- `util.rs:429` now gates the over-cap branch on `lines.is_empty()`; a mid-page over-cap line takes
  the normal byte stop (lines kept, true total + continuation kept, coverage partial -> whole-file
  write refused). Ninth test + pre-fix RED (panic named both consequences: the write returned
  Ok while the read showed only the sed message); restore byte-identical (`bc8ac7875ead95ef…`).
- `limit:0`: the reviewer's premise was FALSIFIED by executing pi 0.84.2's real `truncate.ts` in
  Node (`$LOCALAPPDATA/Temp/m95-pi-limit0/`): pi emits a notice with a blank separator; `files.rs`
  now matches. This is the strongest evidence class in the repo: executed, not re-read.
- Focused log (complete this time): `audit/evidence/m95-focused-rerun.log` — fmt exit 0, clippy
  exit 0, `epsilon-tools` 82 + 45 passed / 0 failed.
- In flight: battery `m95b` (`$LOCALAPPDATA/Temp/m95b-round.sh`, same candidate/target paths so the
  warm target always belongs to one tree identity) + delta re-review (`deleg_82b61a28`).
- NEXT after they land: fold findings -> history row (M95) -> commit `feat(m95)` (slice files only;
  the exclusion checks pass) -> reinstall `epsilon.exe` (tool surface is user-visible) -> cleanup.
