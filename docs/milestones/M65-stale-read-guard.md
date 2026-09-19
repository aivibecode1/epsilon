# M65 — the stale-read guard

**Scope:** port-plan Tier 2 item 6 / defect **D-15**. `write` and `edit` are the only tools that
can destroy work, and until now nothing checked that the model's picture of a file still matched
the disk. A `task` child, a background script, the user's editor — any of them could rewrite a
file between the model's read and its write, and the write would land on top without a word.

The mutation queue (`epsilon-tools/src/mutation.rs`) already serialized *harness* writers against
each other. It could not know that the bytes changed.

## What

| Rule | Refusal | Why |
|---|---|---|
| The file exists, nothing in this session has read it, and the call overwrites it wholesale | `Stale::NeverRead` | Replacing a file nobody has looked at is how work the model never knew about disappears. Hermes refuses write-without-read the same way; DSH calls it `FS_NOT_OBSERVED` |
| The file changed since the last read (mtime **or** size differs) | `Stale::Modified` | The model's `old_string` or full-file body was written against content that no longer exists |
| The file was only read in a window (offset/limit, line cap) and is now overwritten wholesale | `Stale::PartialRead` | The overwrite would discard lines that were never in context |
| The window was the harness's own 256 KB byte cap | *allowed* | No amount of re-reading reveals more through this tool; refusing would trap the model permanently on large files |

> **Amendment (M95).** The last row's premise died when `read` learned to page
> ([`M95-paged-reads.md`](M95-paged-reads.md)): a file larger than a page *is* readable
> further, by asking for the next offset, so "no further read can widen this window" is false
> for anything but a single line longer than the byte budget. The exemption now covers exactly
> that one case (pi's `firstLineExceedsLimit`); everything else is coverage — contiguous lines
> from line 1 through the file's end, accumulated across pages. One page of a 5 MB file no
> longer licenses a wholesale overwrite.

Everything else proceeds: creating a file, writing a file that was read whole and has not moved,
and any targeted `edit` (whose `old_string` must match text the model had to have seen). A
successful mutation records the state it produced, so the harness's own change never looks like
someone else's.

Three supporting changes came out of building it:

- **The read cap is no longer silent.** `read_text_capped` truncated at 256 KB without saying so,
  which made a 1 MB file look fully read. The read result now ends with
  `[file is N bytes; this read is limited to the first 262144 bytes — use grep or bash to inspect
  the rest]`, and the same flag decides whether a later overwrite is legitimate.
- **The tool descriptions state the rule.** `write` and `edit` prompt text now says that
  overwriting requires a read of the current content. A guard the model cannot predict is a trap;
  one it can predict is a workflow.
- **`record_write` marks our own mutations.** Without it, the second edit in a chain would be
  refused by the first.

## Key decisions

- **One registry per process, shared by parent and children.** A `task` child runs in this
  process and writes to the same filesystem, so a parent read has to be visible to a child write
  and vice versa — that is the D-15 scenario. It follows the existing precedent in
  `mutation.rs` (a global map behind a `OnceLock`) rather than inventing a per-agent context.
- **Diagnoses never claim to know who read the file.** Because the log is shared, "you" may be a
  sibling agent. The wording is *"changed on disk since it was last read"*, not *"since you read
  it"*, and a test asserts the message does not say otherwise. Naming the wrong author would send
  the model looking in the wrong place.
- **A refusal must say what to do.** Each diagnosis names the path, the state that changed, and
  the action that lifts it ("read it again, then apply your write"; "read it through first (or
  edit the specific text instead)"). A refusal without a remedy is just an obstacle.
- **The check runs inside the mutation queue.** Check-then-write is a race in the general case;
  running both under the same per-path gate removes it for every harness writer, which is the
  population the harness controls.
- **Honest about what it is not.** An external writer can still win the race between the check and
  the write, and the module says so in its own docs. This detects *known* change; it is a guard,
  not a lock, and it cannot attribute a change to a sibling versus an external process.

## Test-caught bugs

1. **The guard could have been a trap.** Rule 3 as first written refused an overwrite whenever the
   read was windowed — including the window the *harness* imposed at 256 KB. A file over the cap
   can never be read whole, so it could never be rewritten: a permanent refusal with no way out.
   The snapshot now records *why* the view was partial (`partial` vs `capped`) and only the
   caller-chosen kind blocks. Found by writing the rules out and asking what lifts each one.
2. **A shared registry plus parallel tests is a flaky test suite.** The first read-log tests called
   a global `reset_for_tests()` before each case; running in parallel, one test's reset cleared
   another's snapshot and two tests failed intermittently. Uniqueness has to come from the path,
   not from clearing shared state — the reset helper is gone.
3. **My own byte arithmetic was wrong in an assertion.** `assert_eq!(now_len, 19)` for a 20-byte
   string — a test that hard-codes a length it could read. It now derives the number from the
   string it wrote.
4. **Wording asserted the wrong subject.** Reviewing my own message against the shared-log design
   caught "since you read it" and turned it into a test: the diagnosis must not claim the reader
   was the caller.

## Verification

```
$ bash scripts/gate.sh m65
===== fmt --check =====      clean
===== clippy =====           0 warnings
===== test =====             560 tests, 0 failed, 1 ignored (the live smoke)
===== demo =====             completes
===== real-check =====       compiles
```

Twelve new tests: eight unit tests for the rules (`read_log::tests`) and four that drive the real
tools end to end (`builtin_tools.rs`) — read → external write → refused and the file untouched;
re-read → the same write succeeds; partial read → overwrite refused, full read → allowed; edit
refused after an external change but unaffected by our own; create allowed, blind overwrite of an
existing file refused.

Checked against a mutated implementation, with the guard short-circuited to `Ok(())`:

```
$ cargo test -p epsilon-tools
read_log::tests::a_partial_read_cannot_justify_a_full_overwrite ... FAILED
read_log::tests::a_file_that_changed_underneath_is_refused_with_both_sizes ... FAILED
read_log::tests::an_existing_file_nobody_read_is_not_overwritten_blind ... FAILED
read_log::tests::edits_are_guarded_too ... FAILED
builtin_tools: a_write_after_something_else_changed_the_file_is_refused ... FAILED
builtin_tools: a_partial_read_cannot_justify_overwriting_the_whole_file ... FAILED
builtin_tools: creating_a_file_needs_no_history_but_blind_overwrites_do_not_happen ... FAILED
builtin_tools: an_edit_after_a_change_is_refused_but_an_unchanged_edit_is_not ... FAILED
```

Eight failures, and the tests asserting *allowed* behaviour still passed — which is the shape a
guard's tests should have. Reverted before the gate run above.

## Follow-ups

Attribution (was it a sibling or an external writer?) needs a writer map like Hermes'
`path → (task_id, ts)`; the guard's protection is the same either way, so it is a diagnosis
improvement rather than a gap. The rest of Phase 3 — verification enforcement, no-progress and
empty-response guards, untrusted-result wrapping — is listed in [`audit/04-roadmap.md`](../../audit/04-roadmap.md),
along with the standing items: committing M33–M65 (D-12), live-provider evidence for every wire
(D-13), release artifacts (D-21).
