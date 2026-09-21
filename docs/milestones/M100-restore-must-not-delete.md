# M100 — a revert that cannot restore a file must not delete it

## What

`CheckpointStore::restore` treated `blob: None` as "this file did not exist, so reverting means
removing it". That is true for one of the two situations the field covers.

The other is a **skipped capture**, and the store records it deliberately: a file over the 2 MB
limit (`MAX_FILE_BYTES`), or unreadable, or outside the workspace, is written to the ledger with
`blob: None`, `existed: true` and a `skipped` reason — so the ledger says "there is no checkpoint
for this file" rather than leaving an absence nobody notices. `record`'s own doc comment already
said the failure reaches the caller through `Recorded::Skipped` and the edit proceeds.

`restore` did not ask. Both shapes had `blob: None`, so it fell into the remove branch:
**the user asks to undo one edit to a large file and loses the file.** The ledger they could have
read to find out says only that a capture was skipped, and nothing in the result distinguishes
"reverted a creation" from "deleted a file we never saved". That is the worst outcome a revert
path can produce, and `existed` was already on the entry to prevent it.

A missing blob on a file that existed is now an `Err` the caller must see:

```
cannot restore {path}: nothing was captured for it ({reason}).
The file is untouched rather than removed — revert it by hand.
```

The reason is the recorded one, or `the reason was not recorded` when the ledger is itself old
enough to have neither. A file that genuinely did not exist still reverts by being removed.

## Key decisions

- **The fix is `existed`, not `skipped`.** `skipped` is the *explanation* and may be absent on an
  entry written by something other than `record`; `existed` is the field that answers the
  question the branch is asking. Reading the reason for the message and the boolean for the
  decision keeps the decision total.
- **Refusing is the safe direction, but refusing *too much* would be a different bug.** Both
  halves are pinned by their own test: `reverting_a_skipped_capture_refuses_instead_of_deleting_
  the_file` reverts an oversized capture and asserts the bytes are byte-identical afterwards,
  and `reverting_a_file_that_never_existed_still_removes_it` reverts the creation of a file and
  asserts it is gone. A fix that refused every `blob: None` would have broken the one thing the
  `Created` path exists for.
- **The refusal carries the reason into the caller's text.** The message is the only place a user
  or a frontend can learn *why* the undo cannot be performed; a bare "cannot restore" would send
  them looking for a corrupt store. When the ledger is old enough to have no reason recorded, the
  fallback sentence says so rather than inventing one.
- **The doc comment is the argument, not the code.** `restore`'s rustdoc now names both
  situations, the temptation to conflate them, and the consequence; the code is four lines. A
  future reader who "simplifies" the branch back will have to delete the paragraph that says why
  it is wrong.
- **The status is stated plainly in the commit rather than dressed up.** Nothing calls `restore`
  outside tests, so this closes a latent defect, not a live one — `audit/11` listed G3 as silent
  data loss on a path a user reaches only after something else already went wrong, and the
  accurate status is "would be, once a rewind command exists".

## Gotchas

- **A skipped capture is not an error at capture time.** The edit proceeds and the ledger records
  why there is no checkpoint. That is the correct behaviour, and it is exactly what makes the
  restore path's mistake easy to miss: every other signal in the system says "handled".
- **The remove branch was silent by construction.** It returned `Ok(0)` — the same value a
  legitimate create-revert returns — so a caller could not have detected the deletion even in
  principle. The failure was not "the wrong bytes", it was "no way to know".
- **"Outside the workspace" produces the same shape.** A path that `record` refuses as outside
  the workspace is also `blob: None, existed: true`; the fix covers it without a special case,
  which is the point of keying on `existed` rather than on the reason text.
- **A blob that is *missing from disk* is still a different failure.** When `blob: Some(name)`
  and the blob file cannot be read, the message is `checkpoint blob {name} is missing: {e}`.
  That path is unchanged, and it is genuinely a damaged store rather than a skipped capture —
  the two must not be conflated in the other direction either.

## Verification

No battery log for this slice is preserved, and no gate log exists for M100 (`audit/evidence/`
holds `gate-m93b`, `gate-m94c`, `gate-m95b` and `m101-gates.log`). The evidence is the commit
message's own measurements, reproduced here as the commit states them:

- `cargo test -p epsilon-tools` — **94 passed, 0 failed**.
- `cargo clippy` — clean.
- `cargo fmt --all -- --check` — exit 0.

Two tests were added, one per branch (the two named in Key decisions). Neither is claimed as a
pre-fix RED: the skipping test would fail against the pre-fix code by losing the file, and that
is decidable by reading the four changed lines, but no red run was preserved. For scale, the
preceding slice's own commit message reports `cargo test -p epsilon-tools` at **92 passed, 0
failed** (M99, immediately before this one), so the 92 → 94 delta matches the two tests added
here and nothing else moved.

## Limits

- **No caller reaches this.** `restore` has no consumer outside `checkpoint.rs`'s own tests, so
  this is a latent-defect fix: the exposure arrives with a rewind/undo command that does not
  exist yet. The `Err` will surface wherever that command chooses to put it, and nothing in this
  milestone decides what a user should see.
- **F15 is not closed.** The commit names two items that remain on its acceptance list: serialize
  ledger appends across different-file writers (the session lease already excludes concurrent
  writers, and the store documents the assumption), and validate every blob path against the
  store root. The first is the durable-commit half of the same defect family and the second is
  path traversal; both are still open here, and the second half of F15's own restore semantics
  was closed later, in M104.
- **The refusal is user-hostile by design and there is no recovery affordance.** The message says
  "revert it by hand"; nothing offers to show the file's current content, to diff it against what
  was captured, or to name the edit that could not be undone. That is a UI question this slice
  deliberately did not answer.
- **An entry with `existed: false` but a blob is not considered.** The branch reads `blob` first,
  so a `Some(blob)` + `existed: false` entry — which `record` cannot produce — would be written
  rather than refused. Unreachable through the store's own writers; noted because the invariant
  it relies on is a convention rather than a type.
- **The 2 MB limit is the reason most skips happen, and it is not configurable.** A workspace
  full of large-but-text files will produce a ledger with many skipped captures and, after this
  fix, many refusals to revert them.
- **Windows-only verification.** The path joining uses `MAIN_SEPARATOR_STR` and the tests ran
  here; nothing about the fix is platform-specific, but the Linux CI job has not run on these
  bytes.
