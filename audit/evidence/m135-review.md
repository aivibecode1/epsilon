# Independent review — M135, "an `edit` does not license a whole-file write"

**Reviewer:** independent read-only pass. **Base:** HEAD `4fbca81` + working tree.
**Scope:** the M135 slice as diffed. **Not run:** no cargo command, no test, no build — a gate
battery owns the build owner slot. Every finding below was read in the cited code before it was
written down.

**Verdict: the implementation is sound. No code blocker.** The `record_edit` state transition is
correct on every branch I could walk, the fix covers *every* call site that should be covered, and
nothing in the conservative direction is falsely refused. What is not sound is the slice's
**evidence**: three documents present an executed negative control as fact, the milestone's own
`Verification` section says the battery has not run, and no `gate-m135` log exists in the tree.
That is the one thing standing between this slice and landing.

---

## Blockers

### B1 — The negative control is asserted as executed in three documents; the milestone says the battery has not run, and there is no log

**Where:**
- `audit/02-defect-ledger.md:89` — "plus a negative control that reproduces the overwrite
  (`Ok("overwrote book.txt (19 bytes)")`) with both `edit` call sites reverted."
- `docs/history.md:168` — "the negative control reproduces the defect exactly: with both `edit`
  sites reverted, the behavioural test fails with `Ok("overwrote book.txt (19 bytes)")`."
- `docs/README.md:79` — "the record here is the fix, the five tests, and the negative control that
  reproduces the overwrite with the fix reverted."
- `docs/milestones/M135-edit-does-not-license-a-whole-file-write.md:130-132` — "**The negative
  control was run before the commit**, and it is the reason to believe any of this: see
  Verification."
- `docs/milestones/M135-…:111` — "**PENDING** — battery `m135a` has not run. This section is filled
  from the log, not from the tests above."

**What I read.** I listed every `gate-*.log` in `audit/evidence/` (53 files). The newest are
`gate-m133a.log`, `gate-m132a.log`, `gate-m130b.log`, `gate-m129c.log`. **There is no `gate-m135*`
log** — the battery the milestone names does not exist in the tree. I also searched
`audit/evidence/` for `m135`, `record_edit` and `book.txt`: the only hit is
`audit/evidence/m95-red.log:2136-2139`, which is the *M95* control for a *different* test
(`paging_to_the_end_grants_the_overwrite_but_a_skipped_page_does_not`, failing with
`Ok("overwrote book.txt (12 bytes)")`).

**Why it matters.** Three separate documents now tell a reader that a specific test was reverted
and observed to fail with a specific string. The milestone that owns the claim points the reader at
its `Verification` section for the evidence — and that section says the battery has not run. So the
slice contradicts itself in writing, and the corroborating artifact is absent. `AGENTS.md:27-31`
(as amended by this very diff) states the rule the slice is breaking: "the executed record of
record is `audit/evidence/gate-<label>.log`, cited by the milestone that ran it and by
`docs/history.md`."

**In fairness.** The quoted string is exactly what the code would print: the test writes
`"replaced all of it\n"` (19 bytes, which I confirmed independently) and `WriteTool` formats
success as `"{verb} {path} ({len} bytes)"` (`files.rs:369-374`). So the claim is internally
coherent and may well be true. I am not saying it is fabricated — I am saying it is **unbacked by
anything in the tree**, which is the standard this repo set for itself one milestone ago (M134 was
entirely about a document that kept being cited after it stopped being true).

**To clear:** either commit `audit/evidence/gate-m135a.log` and fill the `Verification` section
from it, or downgrade the three assertions to "the control is available by construction" — which is
what the milestone's own `Limits` section is already careful to say at line 131-132.

---

## Should-fix

### S1 — The test count is wrong in three places, including a code comment sitting directly above the tests it miscounts

**Where:** `read_log.rs:674-678` (code comment), `docs/milestones/M135-…:83`, and transitively
`docs/README.md:79` / `docs/history.md:168` ("the five tests" / "Five tests").

**What I read.** The diff adds **five** unit tests to `read_log.rs`, not four:
`an_edit_is_not_a_licence_to_overwrite_the_rest_of_the_file` (`:680`),
`an_edit_of_a_fully_read_file_leaves_it_fully_read` (`:701`),
`an_edit_does_not_close_the_one_window_no_read_can_widen` (`:720`),
`an_edit_of_a_file_nobody_read_leaves_it_a_file_nobody_read` (`:740`), and
`coverage_starts_from_the_top_again_after_an_edit` (`:757`). Plus one behavioural test
(`builtin_tools.rs:845`) = **six** new tests.

The comment at `read_log.rs:674-678` reads "**These four tests** are that sentence turned into
properties" and then enumerates four properties — directly above five tests. The milestone's
`Tests` section says "**Four** unit tests on the state transition" and lists four, omitting
`an_edit_does_not_close_the_one_window_no_read_can_widen` entirely (it appears only later, in
"What nothing exercises"). README and history say "five tests", which matches the milestone's
undercount but not the real total of six.

**Why it matters.** This is the exact defect class M134 was filed for. The omitted test is not a
trivial one — it is the `capped` carry-forward guard, the single test that stops the fix from
turning the M65 exemption into a trap. A reader counting tests to decide how much of the state
machine is pinned gets the wrong answer, and the wrong answer is the one that hides the missing
test.

### S2 — The `capped` carry-forward test's stated justification is not what the code guarantees

**Where:** `read_log.rs:720-737`, rationale at `:722-725`.

**What I read.** The test comment claims: "The over-cap line is a property of the *content*, and an
edit changes only the lines the model named." `record_edit` (`:350`) carries `capped` forward
unconditionally from the previous snapshot, and `verify` (`:402`) treats `capped` as a blanket
exemption from the partial-read refusal for a **full overwrite**.

The claim "an edit changes only the lines the model named" does not hold for this tool.
`edit_match::plan` runs fuzzy stages — whitespace-trimmed, indentation-flexible,
escape-normalized, blank-line-trimmed, and anchor-similarity matching (`files.rs:545`) — and
`replace_all` (`files.rs:415`, `:499-503`) replaces *every* occurrence of a fragment. The
`M71` `Suspicious` guard (`files.rs:579-595`) only refuses when the matched region is *much*
larger than the `old_string`; a modest fuzzy divergence still rewrites bytes the model never typed.
The model's `new_string` can also introduce a brand-new over-cap line, or shorten the old one so
the exemption is no longer needed at all — `capped` is a fact about one read of one content, and
the edit invalidates it in both directions.

**Why it matters.** The consequence is that a model which has seen **zero lines** of a file (a
`long_line` read shows nothing — `read_page` sets `truncated_by = LongLine` and returns
`first_line = last_line = 0`, `util.rs:529-539`, `:593-603`) can `edit` it and then overwrite the
whole file, destroying 100% of content it has never seen. The exemption is *not* a regression
introduced by M135 — `read_log.rs:508-527` already asserts `verify(.., true) == Ok(())` for a capped
read with no edit at all, and `verify:402` grants it today. M135 neither widens nor narrows it. But
M135 **enshrines it in a new test with a rationale that overstates what holds**, and the slice's own
framing (an edit "hands the model the rest of the file") is exactly the property this case
exempts. The honest label is the pre-existing M65/M95 trade — "a guard that can never be satisfied
is a trap" — plus the acknowledgement that the trap is wide: it exempts the whole file, not the one
unshowable line. That acknowledgement appears nowhere.

### S3 — The module doc asserts a completeness the code does not have, and M135 edits that exact doc block

**Where:** `read_log.rs:3` — "`write` and `edit` are the only tools that can destroy work".

**What I read.** `BashTool` runs the platform shell (`shell.rs:45-47`: "Windows uses PowerShell
… because it is always present"), with `SandboxMode::WorkspaceWrite` as the default ceiling
(`shell.rs:68-69`). A redirect or `Set-Content` from `bash` overwrites a file the model never read,
and `shell.rs` contains no reference to `read_log` at all — I grepped the whole `crates/` tree for
`read_log` and the only hits are `lib.rs:15` (the module declaration), the six call sites in
`files.rs`, and a passing mention in a `checkpoint.rs:410` comment. `checkpoint::restore`
(`checkpoint.rs:350`) also writes file bytes back and records nothing.

**Why it matters.** This is pre-existing text, but M135 is the slice that (a) edits the very doc
block it sits in, and (b) exists to restore the F01 guarantee that "a model may not overwrite text
it has never seen." The module's opening sentence is the broadest claim the file makes, and it is
false. A reader who trusts it will believe the guard is total. It is also worth a line in the
milestone's `Limits`: the fix closes the route through `edit`, and the route through `bash` is
untouched and unguarded.

---

## Nits

### N1 — "the guard's diagnoses are unchanged" is true of the strings and false per-case

**Where:** `docs/design/epsilon-decisions.md:1073` ("the guard's diagnoses are unchanged") and
`docs/milestones/M135-…:58-60` ("no diagnosis string changed — a model reading a refusal in a
transcript sees the same three sentences it saw before").

**What I read.** Both are correct that the `diagnosis` function (`read_log.rs:91-112`) is untouched
and that `Stale` still has exactly three variants (`:67-87`). But the new branch at
`read_log.rs:397-401` changes *which* of the three a given transcript shows: a file the model only
ever edited is now refused `NeverRead` where it was refused `PartialRead`. The milestone's own
`read`-flag bullet (`:52-57`) and the CHANGELOG entry both say this plainly, so the docs are not
actually in conflict — the ADR sentence is just loose enough to be misread as per-case stability.

### N2 — The behavioural test's counter-case asserts that the write succeeds but never what it wrote

**Where:** `builtin_tools.rs:918-925`. Minor: the read-full → edit → write half checks the call
returns `Ok` but not the resulting bytes. The other half of the same test does check the file
(`:900-905`), so the asymmetry is probably deliberate. Noted, not worth changing on its own.

---

## What I checked and found clean

- **The third-path question (the one a fix like this usually fails).** `mod read_log;` is **private**
  (`lib.rs:15`), so no other crate can reach these functions. The complete set of call sites in the
  entire workspace is six, all in `crates/epsilon-tools/src/files.rs`: `record_read` (`:229`),
  `verify(.., true)` (`:356`), `record_write` (`:368`), `verify(.., false)` (`:531`), and
  `record_edit` at **both** edit sites — the single-edit path (`:555`) and the batch path (`:689`).
  Both were switched. The write tool's own site correctly keeps `record_write`. There is no third
  path with the false contract: no patch tool, no append tool, no notebook tool exists in the tool
  set, and the milestone's disclosure that the *batch* coverage reset is untested
  (`:118-121`) is honest. This is the check most likely to find a blocker here, and it is clean.
- **The coverage reset's soundness.** I walked every `verify` branch (`:376-408`) against every
  state `record_edit` can produce (`:342-352`). `record_edit` sets `covered: 0, total: None` and
  carries `read`/`complete`/`capped`; the arithmetic is genuinely unrecoverable except from the top,
  because `record_read`'s contiguity rule (`:240`) admits a page only when
  `from <= prev.covered + 1`, which after a reset means `from <= 1`. I confirmed the
  carried-arithmetic counterexample the milestone describes at `:70-72` is real: carrying
  `covered: 200, total: Some(300)` would let `record_read(page(201, 300, Some(300)))` close the
  file at `300 == 300` while lines 1-200 of the new content were never shown. The reset is the
  conservative direction and is correct.
- **`complete` carried across an edit is sound.** A model that read all N lines and supplied the
  replacement for one of them knows every byte of the result. Verified against the sticky-`complete`
  logic at `record_read:253`.
- **Regression sweep in the conservative direction.** Traced, none falsely refused:
  `write`→`write` on a created file (snapshot has `read: true, complete: true`, `verify` returns
  `Ok`); `write`→`edit`→`write` (both flags carried, still `Ok`); `write`→partial-`read`→`write`
  (pinned by the pre-existing `read_log.rs:570-580`); full-`read`→`edit`→`write` (the milestone's
  counter-case). The one workflow that **is** newly refused — a file the model only ever *edited*,
  never read, is now `NeverRead` on a wholesale write (`:397-401`) — is correct under F01 and is
  disclosed in the CHANGELOG.
- **Test discrimination.** Three of the five unit tests genuinely fail if the fix is reverted:
  the partial-read one (`:680`), the `read`-flag one (`:740`), and the coverage-reset one (`:757`) —
  I derived the last one's failure mode by hand rather than taking the milestone's word for it. The
  other two (`:701`, `:720`) pass under both old and new code by design; they are non-trap guards
  against over-refusal, and the milestone says so at `:87-88` rather than claiming them as
  discriminating. I found no trivially-true assertion.
- **The behavioural test's arithmetic.** `paged_file(200, 0)` read with `limit: 20` →
  `read_page` breaks on `PageBound::CallerLimit` (`util.rs:525-528`), `total_lines = Some(200)`
  (`:577-581`), so coverage is `page(1, 20, Some(200))` → `covered = 20 ≠ 200` → partial. The
  notice is the `_` arm at `files.rs:204-208` → "180 more lines in file", matching the assertion at
  `builtin_tools.rs:870`. The `"line 1 "` anchor is genuinely unique in that fixture (`"line 10 "`
  does not contain it), so the edit applies. The test fails under the old code, since the write
  would return `Ok` and `.unwrap_err()` would panic.
- **House rules.** ADR-005's M135 amendment (`epsilon-decisions.md:1073`) describes the code
  accurately — refresh the stat, carry `read`/`complete`/`capped`, discard the line arithmetic, no
  new `Stale` variant. The milestone has the required `Verification`, `Limits` and "what nothing
  exercises" sections, and its `Limits` are honest rather than defensive: it names the batch-edit
  coverage gap, calls the reset "a design choice, not a measurement", and concedes nobody has
  measured the re-read cost. That candour is why B1 stands out — the evidence claim is the one
  place the slice overstates.
