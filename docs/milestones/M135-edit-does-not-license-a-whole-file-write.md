# M135 — an `edit` no longer hands the model the rest of the file

The F01 guard came back through a side door, one milestone after the door was built.

## What

M95 taught `read` pi's paged reads and made the read stamp a *coverage* record: how many lines,
contiguous from line 1, a read has shown, and whether any read reached the end of the file. A whole-file
`write` is refused against anything less, because the model would be replacing text it has never seen.
That is the defect F01 was, and the test that pins it is
`paging_to_the_end_grants_the_overwrite_but_a_skipped_page_does_not`.

M95 also recorded, in its own `Limits`, one hole it did not close:

> a successful **edit** stamps coverage `complete:true`, so read-page-1 → edit-one-line → whole-file
> write is permitted.

That hole was in the code because `edit` and `write` recorded their success through the *same*
function, `read_log::record_write`, and that function's contract — "the bytes it just wrote are known in
full" — is true of `write` and false of `edit`. `write` receives the entire new file as an argument from
the model, so the file really is the model's own text from line 1 to the end. An `edit` receives a
replacement for *lines the model named*, and the rest of the file is exactly the part it never saw. The
recorder could not tell the two apart, so it granted the stronger claim to both.

Reachable in three tool calls, by an ordinary agent, with no mistake in the reasoning:

```
read  book.txt              → page 1 of 5,500 lines, coverage 200/5,500
edit  book.txt              → replace "line 1 " with "line 1, edited by the model "
write book.txt              → "replaced all of it\n"     ← before: allowed. now: refused
```

M95's note said the hole was pre-existing. It shipped anyway, because a hole recorded in a `Limits`
section is a hole nobody is measuring, and nothing in the suite walked `read → edit → write`.

## Key decisions

- **A second recorder, not a flag on the first.** `record_edit` exists because the two tools have
  genuinely different knowledge states, and folding them back together with a boolean would put the
  next caller in the same position this defect came from — guessing which claim applies.
- **The stat is still refreshed.** The reason `record_write` exists at all is that a model's own
  change must not look like somebody else's: without it, a model's *second* edit is refused as
  `Stale::Modified`, i.e. the guard punishes correct behaviour. `record_edit` refreshes the stat and
  invalidates sibling scopes identically. Only the *knowledge* claim differs.
- **The line arithmetic is discarded, the flags are carried.** An edit can add or remove lines, so a
  high-water mark counted in the old numbering can only over-claim coverage of the new content —
  carrying `covered`/`total` would let a model that had read lines 1–200 of the old file read lines
  201–300 of the new one and be told the file is wholly seen. Discarding it is the conservative
  direction, and it is the direction the module already takes for an out-of-order page ("a model that
  reads the tail first must still page from the top"). `complete` and `capped` are carried, because both
  are statements about the model's knowledge rather than about line numbering.
- **`Snapshot` grew a `read` flag, so an edit-only file keeps the honest refusal.** A model can
  `edit` a file it never read (that is legal — the `old_string` is its evidence), and after that the
  path has an observation. Without the flag, a wholesale `write` would be refused as `PartialRead`
  ("only read in part") when the truer sentence is `NeverRead` ("has not been read in this session").
  Both refuse; only one tells the model what to do next. The flag makes the mutation-only case say the
  mutation-only thing.
- **No new `Stale` variant.** `Modified`/`PartialRead`/`NeverRead` still covers every refusal, and no
  diagnosis string changed — a model reading a refusal in a transcript sees the same three sentences it
  saw before, which is what makes old sessions comparable with new ones.

## Gotchas

- **The obvious fix is the bug.** "An `edit` writes the file, so record a write" is how the defect got
  here in the first place: it is true of the `fs::write` call and false of what the model knows. The
  recorder's unit is the model's knowledge, not the syscall.
- **The first draft of the fourth unit test asserted the wrong thing.** It re-read the whole file after
  the edit and expected a partial-read refusal — but a full re-read of the new content *should*
  complete it, and would have made the fix look like it blocked legitimate work. The test that
  discriminates is the one where a read *continues from the old high-water mark*: with the mark carried,
  `record_read(201..300)` closes the file at `300 == 300` while lines 1–200 of the new content have
  never been shown. Same refusal, opposite reason.
- **This machine's shell broke the first test run in a way that looked like a code failure.** Fifteen
  *pre-existing* `read_log` tests failed with `Access is denied (os error 5)` from
  `std::fs::File::create` in the test's own temp helper. The cause is the repo's documented Avira
  artifact block on a freshly linked test binary, not the change under test; the recorded remedy is
  `CARGO_PROFILE_TEST_DEBUG=1` so every test artifact links under a new name. Recorded in
  `agents/memory/next-hardening.md` with the disambiguation, because the failure text names a
  *permission* problem and reads like a real one.

## Tests

Five unit tests and one behavioural test on the state transition
(`crates/epsilon-tools/src/read_log.rs`, `tests/builtin_tools.rs`):

- `an_edit_is_not_a_licence_to_overwrite_the_rest_of_the_file` — the defect itself: 200/300 covered,
  one line edited, wholesale write still `PartialRead`; the targeted edit stays legal.
- `an_edit_of_a_fully_read_file_leaves_it_fully_read` — the guard is not a trap: a file read end to end
  stays wholly known across an edit, and a second edit is not refused as somebody else's change.
- `an_edit_of_a_file_nobody_read_leaves_it_a_file_nobody_read` — the `read` flag: `NeverRead`, with the
  diagnosis that says so.
- `coverage_starts_from_the_top_again_after_an_edit` — the discarded arithmetic: reading on from the old
  mark does not complete the file; paging from the top does.
- `an_edit_does_not_close_the_one_window_no_read_can_widen` — the `capped` carry-forward: a read stopped
  by an over-cap line, then an edit, and a whole-file write is still permitted. Without it, the
  conservative reset would have turned "you have not seen it all" into "you can never see it all", and
  the fix would have shipped a new trap while closing the old one.
- `the_capped_exemption_is_a_hole_the_read_tool_cannot_cloze` — the price of that exemption, written as a
  test because a comment gets forgotten: an over-cap read shows the model **no lines at all**, and the
  guard still permits replacing the whole file. Pre-existing (M65, narrowed in M95), and now stated
  rather than implied.

One behavioural test through the real tools
(`crates/epsilon-tools/tests/builtin_tools.rs::an_edit_does_not_hand_the_model_the_rest_of_a_partly_read_file`):
`ReadTool` → `EditTool` → `WriteTool`, asserting the refusal, that the file on disk is untouched, and the
wholly-read counter-case that still writes. This is the test whose absence let the defect ship, so its
existence is the point.

**It is a 200-line file read with `limit: 20`, not a capped 5,500-line one — and the first draft of this
test was wrong because of it.** The obvious reproduction is a file too large to read in one page, then
edit, then write. That reproduction is impossible, and the reason is a second guard: `edit` refuses a
source over its own byte cap (`edit_refuses_capped_source_without_losing_tail`), so the file the model
could only partly read is one the model cannot edit at all. The reachable narrow read is therefore the
caller's own `limit` — which is also the more honest shape, because it needs no large file to explain
why the defect is real. The test failed on its first run for exactly this reason, and the fix to the
test is recorded here because the wrong instinct (write a bigger file) is the one that feels thorough.

## Verification

Battery **`m135b`**, [`audit/evidence/gate-m135b.log`](../../audit/evidence/gate-m135b.log), on the final
bytes:

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean, zero warnings |
| `cargo test --workspace` | **1185 passed, 0 failed, 1 ignored** across 61 test binaries |
| `cargo run -q -p epsilon-cli -- --demo` | completes, 6 messages, 300/150 tokens |
| `cargo check -p epsilon-cli --features real` | compiles |

The suite delta is worth stating rather than leaving implicit: `epsilon-tools` gains **six** tests
(106 lib + 58 integration, against 100 + 57 before this slice), and nothing else in the workspace moved.

**Two things about how this battery was run, because both cost time and both are the kind of thing
that silently produces a false green:**

- It ran with `CARGO_TARGET_DIR` **outside the repo**. Avira had pinned the in-repo artifact path
  `target/debug/deps/epsilon_tools-<hash>.exe`, so every test that created a file failed with
  `Access is denied (os error 5)` while every test that did not passed — fifteen pre-existing tests
  failing in a crate the slice barely touched. A byte-identical copy of the same binary run from
  `%LOCALAPPDATA%\Temp` passed, which is what proved the cause was the path and not the code.
- It is labelled **`m135b`, not `m135a`**. `m135a` was started before the review's source fixes
  landed, was still running when they did, and was then killed mid-test-leg at 13 of 49 binaries
  without ever printing `GATE PASS`. Per `AGENTS.md` that log proves nothing, so it was not cited
  and a fresh battery ran in the foreground against the final bytes. The lesson is the one the
  rules already state: **stop an in-flight battery when a fix round changes the code** — and the
  practical addition, that a background battery on one job is long enough that a same-session edit
  is likely to land underneath it.

> **The independent review of this slice found a blocker in this section, and it was right**
> ([`audit/evidence/m135-review.md`](../../audit/evidence/m135-review.md)). Four places — this file,
> `docs/history.md`, `docs/README.md` and the F01 row in `audit/02-defect-ledger.md` — already described
> the negative control as run, while the Verification section said PENDING and **no `gate-m135*.log` was
> on disk**. The control *was* run (the two `record_edit` call sites were reverted to `record_write`, the
> behavioural test rebuilt and run, and it failed by writing the file: `Ok("overwrote book.txt (19
> bytes)")` where the fix makes it refuse) — but a claim about executed evidence that exists only in a
> terminal scrollback is not evidence, and `AGENTS.md` names the log as the record of record. So: the
> control output is captured to
> [`audit/evidence/m135-negative-control.txt`](../../audit/evidence/m135-negative-control.txt) as part of
> this round, and the wording in the other three places now points at that file rather than asserting
> the fact. This is the same class of defect as the milestone's own subject — a claim the reader cannot
> check, made about a guard the reader cannot see.

### The review's other findings, and what happened to them

- **S1, the test count.** Real: the slice adds five unit tests, not four, and the count was wrong in
  three places including a code comment sitting directly above the tests it miscounted. Corrected in all
  of them. A doc that undercounts its own tests hides the one it forgot to list, which is the `capped`
  carry-forward — the test that stops the fix trading a leak for a trap.
- **S2, the `capped` rationale.** Real, and the reviewer was right that the stated reason ("an edit
  changes only the lines the model named") is false for *this* tool: `edit_match::plan` runs fuzzy
  stages and `replace_all` rewrites every occurrence. The comment now says what is actually defended —
  that dropping the flag turns the M65 exemption into a trap — without claiming a property that does not
  hold. **The consequence the reviewer identified is now its own test**,
  `the_capped_exemption_is_a_hole_the_read_tool_cannot_cloze`: an over-cap read shows the model no lines
  at all, and the guard still permits replacing the whole file. Pre-existing, neither widened nor
  narrowed here, and now pinned rather than unmentioned.
- **S3, the module doc.** Real: "`write` and `edit` are the only tools that can destroy work" is false,
  because `bash` runs PowerShell and `shell.rs` consults nothing here. The module doc now states the
  boundary of its own guarantee — the F01 protection holds *through the file tools*, and a model with
  shell access is not covered. That sentence is the sort that protects a future reader from trusting a
  guarantee more than it is worth.
- **N1, the ADR's "diagnoses are unchanged".** True of the strings, not per-case: a file the model only
  ever edited is now refused `NeverRead` where it was refused `PartialRead`. Tightened.

## Limits — what this does not prove

- **The `capped` exemption is a real hole this slice deliberately did not close.** A read that stops
  on an over-cap line shows the model no lines at all, and the exemption still licenses a whole-file
  overwrite — the same destruction this milestone fixed, reached through the one window the read tool
  cannot widen. It is pre-existing (M65, narrowed in M95), M135 neither widened nor narrowed it, and it
  is now a named test rather than an unmentioned trade. The honest fix is upstream: make the read
  truncate a long line *visibly* (pi's own `sed` fallback shape), which would let `capped` be dropped
  entirely and the trap close on both sides.
- **The guard's boundary is the file tools, not the harness.** `bash` runs the platform shell and
  consults nothing in `read_log`, so a model with shell access can still overwrite a file it never read.
  The module doc now says so, because a reader who trusted the old wording would believe the
  protection was total.
- **The line arithmetic reset is a design choice, not a measurement.** "Discard the high-water mark after
  an edit" is the conservative direction, and the alternative (carrying it with a byte offset instead of a
  line number) was not attempted. A model that edits one line of a 5,000-line file it has read in full
  must now re-page from the top to regain a whole-file licence; that costs reads, not correctness, and
  nobody has measured the cost.
- **Coverage is still line-based, so it is still approximate for a file whose edits sit near the top.**
  The guard errs toward refusing, which is the safe direction, but "errs toward refusing" is a statement
  about this implementation and not a proof about every sequence of read/edit calls.
- **The negative control was run before the commit**, and it is the reason to believe any of this: see
  Verification. The pre-fix RED is available by construction (revert the two `record_edit` call sites
  to `record_write`).

## What nothing exercises

- The batch-edit path's coverage reset: `edit` with `edits: [...]` records through `record_edit` too, and
  is covered only by the existing batch tests (which assert the replacement happened) and by the shared
  unit tests above. A test that reads a page, batch-edits, and then attempts a wholesale write does not
  exist.
- **The `capped` hole is pinned as a fact, not fixed.** `the_capped_exemption_is_a_hole_the_read_tool_cannot_cloze`
  asserts the current (leaky) behaviour deliberately, so a future change that closes it will fail that
  test on purpose and the milestone will have to say so. That is the intent: the alternative is a
  comment, and comments are what let this class of defect through twice.
