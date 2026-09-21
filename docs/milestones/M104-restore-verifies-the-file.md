# M104 — a checkpoint restore verifies the file before overwriting it

## What

M100 closed the first half of F15: a skipped capture is no longer restored by deleting the file.
This closes the half that was still open, and it is the one that would have destroyed work rather
than only risking it.

**A restore used to write the snapshot over whatever was there.** The store knew the file's
content at capture and never asked whether the file still held it. So the sequence a user
actually performs — the agent edits a file, the user edits it again, the user asks for an undo —
replaced the user's own work with the agent's older copy, and said nothing. No UI reaches this
yet, which is why it is a fix rather than an incident; it is also why the fix belongs in the
store, where the exposure will arrive.

Four changes, one commit:

- **The entry carries a `digest`, and `restore` compares before writing.** A capture already
  computes a content hash — the blob name *is* the content address — so `digest` is `Some(blob)`
  for a stored capture and `None` for the two entries that have no content to be a preimage of.
- **A missing file is not a conflict.** Deletion is the case a snapshot exists to undo, so that
  path still restores. Only a file that is *present and different* is refused, with the byte
  counts of both states in the message.
- **An entry that cannot be verified is refused, not assumed safe.** Entries written before
  `digest` existed have no hash; the old code would have written them over anything. "We cannot
  tell whether this is still the file" is a different answer from "yes", and only one of them is
  safe to act on.
- **`append` reports failure.** It returned `()` and swallowed four distinct failures — directory
  creation, serialization, open, write — while `record` reported the capture as `Stored`. `record`
  now maps each case to `Recorded::Skipped` with the reason (and, for the oversized/unreadable
  path, appends "and the ledger could not record it"), so a caller can no longer mistake "the
  checkpoint is never retrievable" for "the checkpoint is durable".

## Key decisions

- **Content, not metadata.** The tempting cheap check is length or mtime. Both are wrong in a way
  this repository had already been bitten by: two writes inside one filesystem clock tick with the
  same length are indistinguishable by metadata, which is exactly the flake `read_log`'s own guard
  test carried until it started writing different lengths (fixed in M99, one slice earlier). A
  content hash cannot be fooled that way, it was already being computed, and the cost is one read
  of a file the caller is about to overwrite.
- **The unverifiable entry is refused rather than trusted.** `#[serde(default)]` on `digest` is
  what makes old ledgers readable at all, and the default is `None` — so the decision had to be
  made explicitly, and the safe answer is the refusal. The alternative ("no digest, write it
  anyway") is precisely the behaviour being removed.
- **The refusal names both states.** `cannot restore {path}: the file changed since it was
  captured ({now} bytes now, {then} then). Reverting would discard that work — read the file
  first, and delete it if you really want the older content back.` A user who cannot see that the
  file is larger than the snapshot has no way to tell a stale snapshot from a wrong path.
- **A test was codifying the bug, and that is worth saying out loud.**
  `a_recorded_file_can_be_restored_byte_for_byte` captured a file, wrote `"clobbered\n"` over it,
  and then asserted the restore succeeded — the exact sequence the fix now refuses. It restores
  from an untouched file now, and the clobbering case became its own test asserting the refusal.
  A green test that asserts the wrong behaviour is worse than a missing one, because it makes the
  behaviour look intentional.
- **`append`'s return type change is the F07 half of the same defect family.** `record` returning
  `Stored` while the ledger write failed is a durability claim the store could not honour; the
  caller still proceeds with the edit either way, so the loss is bounded to the undo — but it is
  now *reported* rather than discovered later.
- **`writeln` then `flush`, deliberately chained.** A short write that is never flushed is exactly
  the failure this function exists to stop being silent, and `writeln!` on a buffered handle does
  not promise the bytes left the process.

## Gotchas

- **`check_unchanged` has one dead line, and it is visible in the diff.** In the "entry from
  before digests existed" branch it reads a path derived from `target.parent()` into a local,
  discards it with `let _ = recorded;`, and then returns the refusal. The refusal is the
  behaviour; the read is a leftover from the shape this branch was first written in, and it
  computes a path that is not the blob path anyway. It is noted here rather than quietly removed:
  a reader looking for the comparison will find a read that looks like one and is not.
- **The digest is a stable-within-one-build content address, not a cryptographic hash.** The
  module already documented that about `blob_name`, and `digest` inherits the property. It answers
  "is this the same file?" against the store's own past, which is the question being asked; it is
  not an authenticity check against an adversary who chooses the content.
- **A pre-digest entry can never be restored while the file exists.** The only documented escape
  is to remove the file first, so the restore then takes the missing-file path. That is a coarse
  affordance and it is what the message tells the user to do.
- **Skipped captures still have `digest: None`, and they are refused by M100's rule first.** The
  order matters: an entry with no blob and `existed: true` is refused for "nothing was captured",
  not for "cannot be verified". Two refusals that would both be true; the more specific one wins.
- **`restore` now reads the target before writing it.** That is a second read of a file the caller
  is about to replace, in the same call. Cheap, but it means a restore can fail on the read for
  reasons unrelated to the restore — and it does, with `read before restore: {e}`.
- **The blob-missing path is still separate.** `blob: Some(name)` with the blob file unreadable
  yields `checkpoint blob {name} is missing: {e}`, which is a damaged store rather than a
  conflict, and the verification happens *after* the blob is read. A restore therefore fails on a
  damaged blob before it ever asks whether the target moved.

## Verification

No battery log for this slice is preserved, and no gate log exists for M104 (`audit/evidence/`
holds `gate-m93b`, `gate-m94c`, `gate-m95b` and `m101-gates.log`; none of them is these bytes).
The evidence is the commit message's own measurements, reproduced here as the commit states them:

- `cargo test -p epsilon-tools` — **97 + 45 passed, 0 failed** (library plus the integration
  target).
- `cargo clippy -p epsilon-tools --all-targets -- -D warnings` — exit 0.
- `cargo fmt --all -- --check` — exit 0.
- **Three tests added, and the commit says each fails without its fix:**
  `a_restore_refuses_to_clobber_a_file_that_changed_since_capture`,
  `a_restore_recreates_a_file_that_was_deleted_after_capture`,
  `a_restore_refuses_an_entry_it_cannot_verify`.

The `cargo test -p epsilon-tools` figure is anchored by the two commits on either side of it in
this crate's own history: M99's message reports **92 passed, 0 failed** and M100's reports **94
passed, 0 failed** for the same crate and command, so the 94 → 97 delta matches the three tests
added here. (M101 and M102 landed between them and touched `epsilon-core`, not this crate.) That
reconciliation is arithmetic on commit messages, not a battery: no gate battery ran against these
bytes.

## Limits

- **No caller reaches this either.** Like M100, this is a latent-defect fix: nothing outside
  `checkpoint.rs`'s tests calls `restore`, and no UI exists that would present the refusal. The
  exposure — and the decision about what a user should see when an undo is refused — arrives with
  the rewind command.
- **`audit/11` still lists G3 as open.** Its row still reads "Checkpoint restore conflates
  'skipped' with 'deleted' (F15); a durable-commit failure is discarded (F07)", and neither M100
  nor M104 updated it. The two fixes together close the two mechanisms named there, but a reader
  arriving through the assessment will see an open row and should be told why: the row was not
  revised, so the documentation lags the code by two milestones.
- **Two items remain on F15's own acceptance list, named in the commit as not claimed:**
  serializing ledger appends across different-file writers (the session lease already excludes
  concurrent writers and the store documents the assumption) and validating every blob path
  against the store root. The second is the path-traversal half of the same store-hardening
  family and is untouched here.
- **The restore is not atomic.** The file is read, compared, and then written; a write that lands
  between the comparison and the `std::fs::write` is not detected. The session lease and the
  per-path mutation queue are what make this unreachable in practice, and neither is asserted by
  a test here.
- **`digest` gives no protection against a *restored* file being restored again.** Re-applying the
  same snapshot twice is idempotent by construction (the digest matches after the first), which is
  the right behaviour and needs no guard — recorded because it is the question a reader asks next.
- **Nothing verifies that a capture's digest matches the blob it wrote.** Both come from the same
  `blob_name` call, so they cannot disagree today; a corrupted blob on disk would be written over
  the target if the target's content still hashes to the entry's digest — which it would not,
  because the digest and the blob are the same value.
- **The error strings are asserted by substring only.** Three tests match
  `"changed since it was captured"`, `"predates content verification"` and `"nothing was
  captured"`. A reworded message that keeps those phrases passes; the byte counts in the
  conflict message are not asserted at all.
- **Windows-only verification.** The path handling uses `MAIN_SEPARATOR_STR` and the tests ran
  here; the Linux CI job has not run on these bytes.
