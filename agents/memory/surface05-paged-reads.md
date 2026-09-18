# SURFACE-05 — paged reads beyond the byte prefix (next after M93)

## State (2026-09-18)
Brief read-only (deleg_00869eb1, verified at HEAD `fec032a`); **implementation NOT dispatched** —
waiting for M93's build to free the machine (one build owner). Planned milestone: M94.

## Current shape (verified)
- `ReadTool::execute` (`crates/epsilon-tools/src/files.rs:98-191`): `read_text_capped_ex` takes the
  first `MAX_READ_BYTES = 256 KiB` (`util.rs:204-217`), THEN the offset/limit apply to that
  prefix's lines (`:146-157`); total = the prefix's count (`:165`), so the continuation notice
  (`:170-175`) is false for capped files; empty and past-end are conflated (`:186-188`);
  `from_utf8_lossy` can inject U+FFFD at the byte boundary (`util.rs:217`) — pi never emits a
  partial line.
- Same reader's other consumers: `grep` (`search.rs:193`) truncates **silently** (no marker — a
  real honesty bug); the strict edit reader `read_text_for_edit` (`util.rs:176-192`) refuses
  over-cap/NUL/invalid-UTF-8 ✓ untouched by this slice; the `files.rs:620-635` F01 citation is the
  **batch-edit writeback**, not a batch read ✓ leave alone.
- pi's exact semantics (`pi/.../tools/read.ts`): past-end → `Err("Offset {n} is beyond end of file
  ({m} lines total)")`; empty file → Ok empty; over-cap single line → a bash/sed message +
  `firstLineExceedsLimit`; continuation strings `"[Showing lines a-b of t. Use offset=b+1 to
  continue.]"` / `"[n more lines in file. Use offset=a to continue.]"`; limits 2000 lines / 50 KiB.

## THE COUPLING THAT MUST SHIP TOGETHER (the reason this is one slice)
`read_log.rs`'s `capped` flag (`:26-40`) exempts a window from the stale-read guard because "no
further read can widen it" (`verify` :223-227, doc :36-39, M65's doc :18, test :301-313, and
`builtin_tools.rs:956-994`). Pagination falsifies that premise: if `capped` is dropped without a
coverage model, one page-1 read of a 5 MB file gains **wholesale overwrite rights** (F01-class
data loss). If it is kept as-is, the exemption silently outlives its premise. So: `RecordRead`
(`read_log.rs:149`, called only from `files.rs:189` + its own tests) takes **coverage** (contiguous
lines observed from line 1 vs the file total) and `capped` narrows to "a single over-cap line —
the only honest terminal reason left".

## Fix shape
- `util.rs`: new `ReadPage { text, first_line, last_line, total_lines: Option<usize>, next_offset,
  truncated_by: PageBound, long_line: Option<usize> }` + `read_page(path, offset, limit)` using
  `BufReader` + per-line `read_until` (resident memory = one line + the page; never the whole
  file), NUL refusal on the first chunk, per-line decode (boundary = line boundary), honest
  `total_lines` only when the scan reaches EOF. `read_text_capped(_ex)` stays for `search.rs`.
- `files.rs`: `ReadTool::execute` calls `read_page`, keeps numbering, emits one honest notice
  (page-truncated / caller-limit-stopped / long-line bash fallback), `Err` for past-end, empty for
  an empty file, and additive `details = {"read": {…}}`.
- `search.rs`: keep the cap but **mark it** (no silent lie) + a test.
- Docs: `descriptions/read.txt` (its pagination promise becomes true), M65's doc, milestone doc.
- Keep `MAX_READ_BYTES` as the per-page budget (edit tests reference it: `builtin_tools.rs:266,…`).

## Test plan (7 + 1, all with the sharpest discriminator named)
1 pages past the prefix (marker at line 3400 of >256 KiB); 2 past-end error vs empty Ok;
3 no U+FFFD at the boundary (byte 262144 inside a multibyte char); 4 small explicit limit beyond
the prefix; 5 over-cap single line → bash/sed route + a later wholesale write *allowed*;
6 **the sharpest**: full paging to EOF → write allowed, a skipped final page → write refused with
"only read in part"; 7 `read_log` unit: out-of-order pages don't extend coverage, contiguous pages
clear `partial`; 8 grep's cap is marked. Honest rewrites (not weakening): `read_log.rs:301-313`
split into the two cases; `builtin_tools.rs:261-291`/`:346-374` must keep passing unchanged.

## Owed by the round
Implementation → probes (revert `read_page`'s coverage handoff → test 6 must fail; revert the
boundary fix → test 3 must fail) → frozen candidate + battery `m94a` → independent review →
history row + ledger SURFACE-05 stamp → commit → cleanup. M93's queued items: also check M67's
`ProgressGuard` interaction (out-of-range reads become errors; keep the offset in the message).
