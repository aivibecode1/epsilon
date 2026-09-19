# M95 — Paged reads (SURFACE-05) and the coverage handoff

## What

`read` promised pagination in its own description ("pass offset (1-based) and limit to page
through larger files") and did not deliver it. `read_text_capped_ex` took the **first 256 KiB**
of the file and only then applied `offset`/`limit` to that prefix's lines, so:

- nothing past byte 262144 was reachable: `offset` paged *inside the prefix*, and a match at
  line 3400 of a 400 KB file could not be read at all;
- the continuation notice counted the prefix (`[file has 2372 lines; showing 1-2000; pass offset
  to continue]` for a file with 3400 lines) — a false total, i.e. the honest number the model
  needs to decide whether to keep reading was wrong;
- an empty file and an offset past the end were the same result, `"(empty or past end of file)"`,
  so the model could not tell "nothing is there" from "you aimed past the file";
- `from_utf8_lossy` over the byte-truncated buffer injected U+FFFD whenever byte 262144 fell
  inside a UTF-8 character, and emitted a partial last line pi never emits.

**The fix.** `util::read_page` ports pi's semantics
(`references/pi-0.84.2/packages/agent/src/harness/tools/read.ts:97-141` and the same code in
`packages/coding-agent/src/core/tools/read.ts:273-323`), streaming instead of slurping:

- `offset` is 1-based over the whole file; past the end is the error pi raises —
  `Offset {n} is beyond end of file ({m} lines total)` — while an **empty file reads Ok and
  empty**, exactly pi's `"".split("\n")` path (there the offset check cannot fire and the
  selection is empty);
- `limit` slices first, then the page is capped at 2000 lines / 256 KiB (epsilon's byte budget
  for pi's 50 KiB — the edit tests and the tool contract already name `MAX_READ_BYTES`);
- a page whose **first line alone exceeds the byte budget** gets pi's `firstLineExceedsLimit`
  answer — the actual message from the test fixture:
  `[Line 1 is 260.0KB, exceeds 256.0KB limit. Use bash: sed -n '1p' huge.txt | head -c 262144]`
  — no lossy prefix;
- every other page that stops early ends with pi's continuation strings
  (`[Showing lines a-b of t. Use offset=b+1 to continue.]`, the `(256.0KB limit)` variant for a
  byte stop, `[n more lines in file. Use offset=a to continue.]` for a caller-limit stop), and
  the numbers are the file's real ones: the scan counts the remainder of the file (without
  retaining it) so `t` and `n` are never the prefix's count again;
- the page text holds whole lines only, decoded per line, so a window boundary is always a line
  boundary and a multibyte character is never cut;
- the tool result carries additive `details.read` (`first_line`, `last_line`, `total_lines`,
  `next_offset`, `truncated_by`) for frontends; details never reach the model.

**The coupling that shipped with it.** `read_log`'s `capped` exemption existed because "no
further read can widen this window" — the 256 KiB cap. Pagination makes that false for every
file bigger than a page, so keeping the flag as-is would have granted a single page-1 read of a
5 MB file **wholesale overwrite rights** (F01-class data loss). `RecordRead` now takes a
`Coverage { from, to, total, capped }`; coverage accumulates only across reads **contiguous from
line 1** of the same bytes, a write may replace a file only when the reads reached its end (or
the harness wrote it), and the exemption narrowed to pi's over-cap single line — the one window
no read can ever widen:

| Read | `write` (full overwrite) |
|---|---|
| page 1 of a 3-page file | **refused** — `only read in part` (was: *allowed*, via `capped`) |
| pages 1..n-1, last page skipped | refused |
| pages 1..n to EOF | allowed |
| a read that covers a whole file in one page | allowed (`total == covered`) |
| a line longer than 256 KiB, `sed`-fallback page | allowed (the M65 exemption, narrowed) |
| read → file changed → page re-read | refused until the new content is read through |

Coverage of a snapshot is discarded when the file's stat moved or a sibling wrote it: the old
lines described other bytes, so they cannot cover the new ones.

`grep` shares the same helper and its 256 KiB cap was **silent** (a match past the cap was
simply not found). The cap stays — the walker visits up to 5000 files and an unbounded read per
file trades a silent miss for an unbounded scan — but the result now names the files that were
only searched in part, in the no-match case too, and `details.files_capped` carries the list.

## Key decisions

- **One streamed pass per read, not a seek.** Lines before `offset` are read and dropped, the
  page is retained, and the rest is counted without retention: resident memory is one line plus
  the page (never the whole file), and the cost is `O(file bytes)` per read — the same order as
  pi, which reads the whole file into memory and slices it. The alternative (stop at the page
  and report no total) would have made pi's own continuation strings unemittable, and the
  past-end error needs the full count anyway.
- **Whole lines are the unit of everything** — the page, the byte budget, the cutoff. This is
  what removes the U+FFFD injection mechanically: there is no byte boundary to split a
  character, only line boundaries. `from_utf8_lossy` remains per line for bytes that are
  genuinely invalid in the file, which is what pi's `buffer.toString("utf-8")` does.
- **Coverage, not a flag, is what a mutation is verified against.** A pre-existing
  contradiction sat in the old record: the read stamp called a 256 KiB prefix "capped" (no
  further read can widen it) at exactly the moment the tool description promised
  `offset`-paging. Coverage states what was seen as lines; `complete` is a line-coverage test
  (`total == covered`) and nothing else, including for an empty file.
- **pi's strings are emitted verbatim where the brief pinned them, including the counts.**
  The one deliberate divergence is the line count itself: epsilon counts the lines it can
  display (`str::lines`), pi counts `text.split("\n").length`, which is one more for a
  newline-terminated file — a phantom empty line pi can return as an empty page (`offset ==
  total_pi` is legal there). A phantom line would make the numbers in pi's own notices
  disagree with the numbered lines this tool shows, and this tool's numbers are what the model
  pages by.
- **The exemption survived, narrowed, with its reason re-checked.** A guard that cannot be
  satisfied is a trap (M65), and the over-cap line is the one case where no read through this
  tool can ever show the window. It is also *sticky*: encountering it makes completeness
  impossible (the line is never covered), so clearing it on a later page would trap the model
  on exactly the file the exemption exists for.
- **`grep`'s cap is marked, not widened.** ripgrep-backed pi finds matches past 256 KiB; our
  in-process walker does not. Widening it means streaming every walked file in full — a
  per-file unbounded read in the search path — so the honest miss is reported instead, with a
  test that pins today's behaviour and the note.
- **No new dependency, no TUI change.** The pager is `BufReader` + `fill_buf`/`consume`; the
  only new public surface is `ReadPage`/`PageBound`/`read_page`/`format_size` and
  `read_log::Coverage`.

## Gotchas

- **The coverage model has one trap and it found one bug in drafting.** Treating "the model
  saw the whole file once" as sticky *unconditionally* would have re-opened the F01 window on
  a different path: read a file whole → someone else replaces it → the model re-reads a single
  page of the new content → the old whole-file credit would still be attached and the overwrite
  allowed. Coverage is therefore keyed to the stat it was taken against (mtime + length, as the
  rest of the guard already was): a changed file starts coverage from scratch. Pinned by
  `coverage_of_one_content_does_not_cover_another`.
- **A page is a window only if it is one.** The old `windowed` flag was derived from the
  arguments, not from the lines shown: any read of a file with 2000+ lines counted as windowed
  even when it returned the whole file (`total >= MAX_READ_LINES`), and `start > 1` alone marked
  a read windowed regardless of what it covered. Coverage is the lines actually shown, so a read
  that returns a 2000-line file whole now allows the overwrite it should.
- **`read_log`'s old exemption test changed meaning, not strength.** `a_window_the_harness_
  imposed_does_not_block_a_rewrite_forever` asserted the old premise; it is now two tests — a
  page of a larger file is *refused* (`a_page_of_a_larger_file_cannot_justify_a_full_overwrite`)
  and the over-cap line is the one exemption left
  (`a_line_longer_than_the_byte_budget_is_the_only_window_left_open`) — plus a trap test that a
  later page does not withdraw it.
- **The NUL refusal had to move, not change.** With a streamed scan, checking "the buffer" no
  longer meant anything: the rule (first 4 KiB of the file) is now probed before the scan, so a
  short page cannot hide a binary. Same window, same message.
- **The M93 note about `ProgressGuard` is satisfied by the error itself.** An out-of-range read
  is now an `Err` naming the offset and the file's line count instead of a successful
  `(empty or past end of file)`, so a run that pages past the end reads a distinct result per
  attempt rather than the same string — which is what the stuck-run guard counts.
- **`grep`'s secret-class skip note used to vanish when there were no matches** (it was pushed
  into the match rows, which the no-match early return discards). Both notes now travel the same
  path, so the skip report and the new cap report survive the no-match case.
- **The page text keeps the blank line a `lines()` split would drop.** The page's text is whole
  lines joined by `\n`, so a page that ends on an empty line ends with a newline; numbering it
  with `str::lines()` silently dropped that line (and its number), making the page claim more
  lines than it showed. Found while writing the pager's tests; `a_page_that_ends_on_a_blank_line_
  keeps_that_line` pins the split on `\n` with the empty-page guard.
- **The image probe used to read every file whole.** `ReadTool::execute` called `std::fs::read`
  on every path just to look for image magic bytes (M43), so the pager's memory bound was real
  only after an `O(file)` allocation had already happened — and a huge binary would have been
  loaded whole before the text path ran. The probe now reads the first 4 KiB; when those bytes
  look like an image, the whole file is read and detection re-runs on the full buffer, which is
  byte-for-byte what the old code did on that branch (every signature lives in the head, so
  "no signature in the head" implies "no signature at all").

## Verification

Raw logs in `audit/evidence/`. The focused runs describe the working tree (base HEAD `6c946f5`);
the **frozen-candidate battery `m95b` passed** — see the block at the end of this section.

- `m95-clippy.log` — `cargo clippy -p epsilon-tools --all-targets -- -D warnings` → exit 0
  (the first run earned its keep: it failed on `single_char_add_str` and on two dead
  `Coverage` constructors the lib did not call — both fixed, then clean).
- `m95-focused-tools.log` — `cargo test -p epsilon-tools` → **82 passed / 0 failed** (lib, the
  `read_log` unit suite included) + **44 passed / 0 failed** (`builtin_tools`, the 8 new
  integration tests included), exit 0. (This log contains no `fmt` step; the fmt claim belongs to
  `m95-focused-rerun.log`, which re-ran all three gates on the final bytes — see below.)
- `m95-red.log` — the pre-fix RED the task requires. The slice's four source files
  (`util.rs`, `read_log.rs`, `files.rs`, `search.rs`) were reverted to HEAD
  (`git checkout HEAD -- …`; HEAD hashes `fc1ed929…`, `328164f6…`, `93270170…`, `87800abf…`),
  the new tests kept, and the seven paging/coverage tests run against those pre-fix sources:
  **7/7 failed**, each for the reason it names —
  - `paging_to_the_end_grants_the_overwrite_but_a_skipped_page_does_not`:
    `called Result::unwrap_err() on an Ok value: "overwrote book.txt (12 bytes)"` — the F01
    rule: pre-fix, **one page-1 read of a 5,500-line file was enough to overwrite it wholesale**;
  - `a_multibyte_character_is_never_cut_by_the_page_boundary`: the pre-fix page contains
    ` 2623| �` and the old cap notice, i.e. the cut character rendered as U+FFFD;
  - `read_pages_past_the_byte_prefix_to_the_marker`: the continuation string is
    `[file has 2372 lines; showing 1-2000; pass offset to continue]` — the prefix's count, not
    the file's 3400, and no `of 3400` anywhere (line 3400 is unreachable);
  - `read_past_the_end_errors_and_an_empty_file_reads_ok_empty`:
    `called Result::unwrap_err() on an Ok value: "(empty or past end of file)"`;
  - `a_small_explicit_limit_reports_how_many_lines_remain`: no continuation notice at all for a
    12-line file read `offset=3&limit=4`;
  - `an_over_cap_single_line_points_at_bash_and_leaves_the_write_open`: the page returns a
    256 KiB prefix of the line with the old cap notice instead of pi's `sed` fallback;
  - `grep_marks_the_files_it_could_only_search_in_part`: `no matches`, with no note naming the
    truncated search.

  The slice was then restored from byte-exact backups — `RESTORED-OK` for all four with sha256
  `util.rs a45f9c9b…`, `read_log.rs 6f60e140…`, `files.rs 32641607…`, `search.rs 867d6ada…`
  (equal to the pre-revert values) — and the same seven tests re-ran green on the restored bytes
  (`GREEN_COUNT=7 of 7`), with `git checkout`-restored files never used again.

  The eighth new integration test (`a_page_that_ends_on_a_blank_line_keeps_that_line`) was **not**
  RED-screened: the RED run held the seven paging/coverage tests, and no pre-fix failure of the
  eighth is claimed — it pins a split detail found while writing the tests. The ninth, added by the
  fix round below, is RED-screened — its pre-fix failure is in `m95-focused-rerun.log`.

- `m95-focused-rerun.log` — the fix round's one complete focused log on the final bytes. The
  round's code change: `read_page`'s over-cap branch is now gated on `lines.is_empty()`, so only a
  page whose *first* line exceeds the budget takes pi's `sed` path — the round-1 tree fired it for
  any over-cap line, which the review caught as blocker 1 (a mid-page over-cap line replaced the
  whole page, dropped `of T`/the continuation, and recorded `capped`, licensing a wholesale
  overwrite of a file the read had shown nothing of). The code now matches this document's
  "first line alone" claim, and a later over-cap line is an ordinary byte stop. It also carries:
  the executed check of pi's `limit: 0` behaviour (pi emits its notice — with the blank separator
  it always writes — rather than silence, so the port now matches pi, the reviewer's premise
  having been checked and falsified; raw output in the log); the RED for the ninth test
  `a_later_over_cap_line_still_shows_the_page_and_still_refuses_the_overwrite` — `util.rs` was
  reverted to the reviewed pre-fix bytes (sha256 `4cfe7bcc87c1550b…`, the hash the round-1 review
  records for `util.rs` — note `m95-red.log`'s own pre-fix hash is `a45f9c9b…`; the delta
  re-review flagged the earlier wording as misattributing it), the single test failed
  `a read stopped by a later over-cap line must remain a partial view, but the whole-file write
  returned Ok("overwrote mixed.txt (9 bytes)") while the read had shown "[Line 4 is 260.0KB,
  exceeds 256.0KB limit. Use bash: sed -n '4p' mixed.txt | head -c 262144]"`, and after a
  byte-identical restore (`util.rs bc8ac7875ead95ef…` before and after) the same test re-ran
  green; and all three gates on the final bytes — `cargo fmt --all -- --check` exit 0,
  `cargo clippy -p epsilon-tools --all-targets -- -D warnings` exit 0, `cargo test -p
  epsilon-tools` → **82 passed / 0 failed** + **45 passed / 0 failed**, exit 0.

**Final working-tree bytes** (after the fix round: the `lines.is_empty()` gate in `read_page`,
the `limit: 0` fold in `files.rs`, comment updates and the ninth test; `m95-focused-rerun.log`
describes these — the earlier logs describe the round-1 bytes):
`util.rs bc8ac787…`, `read_log.rs 6f60e140…`, `files.rs e2cdcae7…`, `search.rs 867d6ada…`,
`tests/builtin_tools.rs e5339904…`, `descriptions/read.txt fff98dab…`.

**Frozen-candidate battery — label `m95b`, PASS.** `audit/evidence/gate-m95b.log`: all five gates
exit 0 and the log ends `===== GATE PASS =====`. Aggregate **876 passed / 0 failed / 0 ignored
across 55 summaries** (all 55 `ok`). Per-binary reconciliation against the M94 battery
(`gate-m94c.log`, 863/0/0/55): the two logs are identical index-for-index except **`builtin_tools`
36 → 45 (+9)** and **`epsilon_tools` 78 → 82 (+4)** — the +13 total, and the whole of it. Candidate
composition: `git archive HEAD` plus exactly the slice's files; the other workstreams' files
(`registry.rs`, `stream.rs`, `agent-contract.md`, `contracts/lib.rs`, the M18 documentation) were
proven equal to HEAD (EOL-normalized) before the build, and the shipped code bytes were re-verified
hash-identical to the candidate's after the battery (`util.rs bc8ac787…`, `read_log.rs 6f60e140…`,
`files.rs e2cdcae7…`, `search.rs 867d6ada…`, `tests/builtin_tools.rs e5339904…`). The first
attempt (`m95a`) was stopped when the review's blocker changed the bytes — its log held only the
fmt header, so no misleading evidence exists for it.

**Delta re-review** (read-only, `deleg_82b61a28`; the reviewer's findings are appended to
`$LOCALAPPDATA/Temp/m95-review-delta.md` and folded into Limits): **no blockers**; its whole-tree
sweep confirmed the fix closes the class.

**Rebuild**: the global `epsilon.exe` is rebuilt from this frozen candidate (the slice is
CLI-visible — the model's `read` results change, so the owner's binary must come from verified
bytes).

## Limits

- **Battery and review status.** Everything above is a focused run on the working tree; the battery
  on a frozen candidate is `m95b` (`$LOCALAPPDATA/Temp/m95b-round.sh`, fresh label, gate-log
  verdict). The read-only review found one blocker (fixed above) and the delta re-review of that
  fix returned **no blockers**: its whole-tree sweep confirmed `capped` can only be stamped by
  `Coverage::long_line()`, whose only non-test caller is `files.rs`'s page replacement — body
  replacement and capped recording cannot disagree — so the only remaining nothing-displayed +
  capped path is the intended first-line exemption (pi's `firstLineExceedsLimit`, an anti-trap).
  The installed-binary rebuild is still owed (this slice is CLI-visible: the model's `read` results
  change, so the owner's `epsilon.exe` must come from the verified bytes).
- **`limit:0` and friends — queued as one follow-up (delta re-review findings F2-F5).** With
  `offset > 1` the "N more lines" count uses the total where pi uses `total − startLine` (a 5-line
  file at `offset=5` says "5 more" where pi says 1); an empty file with `limit:0` gets silence
  where pi emits its phantom-line notice; nothing pins `limit:0`; the ninth test's refusal matches
  `Err(_)` rather than the `only read in part` diagnosis; the over-cap branch compares raw bytes
  where pi compares the lossy-decoded length; and `details.read.offset` reports 1 on an empty page
  while the notice says otherwise. None has a guard effect (coverage stays correct); batched so
  they cost one battery cycle rather than several.
- **Every read scans to EOF** to report pi's exact line counts: memory is `O(line + page)`, but
  time is `O(file bytes)` per read. pi pays the same order (it reads the file whole, plus
  `O(file)` memory). A seek-based version would need a line index this tool has never had.
- **The line count is this tool's, not pi's.** pi counts `text.split("\n").length`, one more for
  a newline-terminated file (a phantom empty line it can return as an empty page). Epsilon counts
  the lines it can display, so the numbers in the notices match the numbers on the left of the
  output. Recorded as a deliberate divergence, not an oversight.
- **The over-cap-line exemption is a real, narrow window, not zero.** The exemption triggers only
  when the page's *first* line alone exceeds the byte budget (pi checks `lines[0]` of the
  selection, `truncate.ts:103-105`): that read shows nothing, leaves `capped` set, and a wholesale
  overwrite is then allowed (M65's anti-trap rule). An over-cap line *later* in the page is an
  ordinary byte stop instead — the lines already shown are kept, the total and the continuation
  survive, coverage stays partial, and a wholesale overwrite is refused (pinned by
  `a_later_over_cap_line_still_shows_the_page_and_still_refuses_the_overwrite`). The window is
  strictly narrower than M65's — that one exempted *any* file larger than 256 KiB; this one needs a
  single unreadable first line — but a model that lands on such a line has seen nothing of the file
  and may still replace it. Pinned by
  `an_over_cap_single_line_points_at_bash_and_leaves_the_write_open` and the unit tests beside it.
- **A successful `edit` still stamps the file as fully covered** (pre-existing, unchanged vs HEAD).
  `record_write` marks a snapshot `complete: true` (`read_log.rs:278`) and the edit path calls it
  after every applied edit (`files.rs:555`, `:689`), so read-page-1 → edit-one-line → whole-file
  `write` is permitted though the model never saw the rest of the file. HEAD's `record_write`
  cleared its `partial` flag the same way, so this is not a regression introduced here and it is
  not fixed here — it wants its own guard change (an edit should extend coverage over the text it
  replaced, not the whole file), so it is queued rather than smuggled into this slice.
- **`offset`/`limit` are `usize` where pi clamps its numbers.** pi takes JS numbers and clamps
  them (`Math.max(0, offset - 1)`, `Math.min(startLine + limit, …)`), so a negative offset reads
  from the top and a fractional limit slices where JS truncation lands. epsilon's schema takes
  unsigned integers: a negative value fails argument deserialization and a float is a schema error,
  rather than being clamped. Unreachable through a well-behaved model; recorded as a fidelity gap,
  not guarded against.
- **The `sed` fallback sizes the over-cap line in raw bytes.** pi reports
  `Buffer.byteLength(allLines[startLine], "utf-8")` (`read.ts:299`) — the byte length of the
  *lossy-decoded* string — where epsilon reports the byte count it read from disk. The two differ
  only for an over-cap line with invalid UTF-8 (a replacement character is three bytes where the
  original byte was one); the message, path and budget are pi's, the size can be larger.
- **Coverage is a high-water mark, not a segment set.** Pages that are not contiguous from the
  mark extend nothing: a model that reads the tail first must re-read from the top (or from its
  current mark) before the file counts as covered. Conservative by design; a segment set would
  let a scattered read pattern look complete.
- **`complete` inherits the guard's old stat granularity.** mtime + length equal and not
  invalidated ⇒ "the same bytes"; a same-length external write that restores the mtime is still
  invisible (pre-existing, documented in `read_log.rs`).
- **Invalid UTF-8 inside a line still renders as U+FFFD** — that is pi's own
  `buffer.toString("utf-8")` (`read.ts:274`). What is gone is U+FFFD produced by the *window*: no
  page emits a partial line or a cut character. A file with invalid bytes anywhere in a shown
  line looks the same as it did before this milestone.
- **CRLF still loses its `\r`** on the read path (`str::lines`' rule, epsilon's pre-existing
  behaviour); pi keeps it. Unchanged here; the page split is on `\n` with one trailing `\r`
  stripped, exactly as before.
- **`grep`'s 256 KiB cap remains a fidelity gap.** ripgrep-backed pi finds matches past that
  byte; the in-process walker does not. The cap is now announced (`[grep] searched only the first
  262144 bytes of N file(s) …`, `details.files_capped`) and the note survives the no-match case,
  but making it boundless means streaming every walked file in full — queued, not done.
- **The `ProgressGuard` interaction is satisfied by construction, not by a test.** An
  out-of-range read is now an `Err` naming the offset and the file's line count (M93's queued
  item), so the stuck-run guard sees a distinct result per attempt; the guard itself lives in
  `epsilon-core` and no test here drives it with a past-end read.
- **Windows-only verification.** The pager is std-only (no platform branches), and the tool tests
  ran on this Windows box; the Linux CI job (`ubuntu-latest`, bwrap suite) has not run on these
  bytes.
- **The evidence logs are untracked and the two docs they touch are untracked in this tree**
  (`docs/milestones/M65-stale-read-guard.md` is one of the historical milestone docs that have
  never been committed; `M95-paged-reads.md` is new). A future `git clean` would destroy the M65
  amendment; whoever commits this slice must stage those files explicitly.

