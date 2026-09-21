# M113 — an entry kind this build does not know is kept, not used to close the session

## What

A session file can carry an entry kind this build has never heard of — written by a newer
Epsilon, or by another tool sharing the same envelope. Until now the store **refused the
whole session** at that line: `StoreError::corrupt`, "entry type `x` is not known to this
build — the session was probably written by a newer Epsilon". The diagnosis was right and
the consequence was wrong. One foreign entry cost the entire conversation, and the user's
only route back was to upgrade or to edit the file by hand.

The refusal came from M108, which had a good reason: a complete line written on purpose
must not be deleted, and refusing deletes nothing. But refusing is not the only way to
delete nothing, and it is the most destructive of the safe options. An entry can be
**kept** — parsed as an opaque object, replayed verbatim on the next write, and left out of
the model view — which honours "do not delete" *and* leaves the session usable.

So the store now keeps it, and says so.

## Key decisions

- **Lenient about the tag, strict about the body.** The `kind` codec parses the object,
  reads its `type`, and consults `KNOWN_ENTRY_KINDS`. An unknown tag becomes
  `EntryKind::Unknown { type_name, body }` carrying the whole object. A *known* tag whose
  body will not parse is still an error — because preserving that would be worse than
  refusing it: the session would open with a **silent gap** where a turn should be, which is
  precisely the failure M108 was written to end. The two cases get two diagnoses,
  `UnknownEntryType` ("foreign") and the new `MalformedEntry` ("damaged"), because they call
  for different actions from whoever reads the message.
- **The whole `kind` object is kept, not the two fields we name.** Re-serializing from named
  fields would silently drop everything this build has no name for; keeping the object means
  a rewrite reproduces every field and value. Key *order* is not preserved — `serde_json`'s
  map is sorted — and the type says so rather than claiming byte-identity the
  implementation does not deliver. (The first version of the test asserted byte-identity and
  failed, which is how the claim got corrected instead of shipped.)
- **`KNOWN_ENTRY_KINDS` is a single exported list, with a drift guard.** The codec needs it
  to choose between preserving and insisting; the store's diagnoser needs it to choose
  between "foreign" and "damaged". `the_known_kind_list_covers_every_variant` walks one of
  every variant and fails if a tag is missing — so adding a variant without a tag is a test
  failure, not a mis-diagnosis three layers away.
- **The store reports what it kept.** `preserved_unknown()` returns `(seq, type_name)` for
  every entry preserved in the last scan. An entry that never reaches the model view and
  never appears in the transcript is exactly the kind of absence a frontend has to be able
  to announce — the same rule the repair report follows. Nothing calls it yet; the CLI
  wiring is recorded as owed rather than implied.
- **This is the landing place for the importer, not the importer.** A real pi 0.85.1 file
  does not reach this code path at all: its *envelope* differs (`id`/`parentId`/ISO
  `timestamp`/top-level `type`, versus this build's `seq`/`timestamp_ms`/nested `kind`), so
  it fails on its first entry line. Adding `Label`/`SessionInfo` variants — which this slice
  also does, since they are pi's meaning-bearing kinds and the harness can *apply* them —
  makes no pi file readable on its own. The importer is its own milestone with its own
  design ([plan](../../audit/evidence/session-format-gap-plan.md)).

## Gotchas

- **The obvious serde shortcut silently destroys the payload.** `#[serde(other)]` looks like
  exactly what an "unknown variant" wants, but it is only allowed on a *unit* variant, so
  the tag and the body are discarded at parse time and the next rewrite writes a stub. The
  custom field codec exists because the attribute cannot express this.
- **Changing a refusal into a preservation breaks exactly one test, and that test was
  right.** `an_unknown_entry_type_is_refused_by_name_not_called_corruption` failed the
  moment the rule changed. It was rewritten rather than deleted, and its subject — "a file
  from a newer Epsilon is not damaged, and must not be reported as corruption" — is still
  the point; what changed is the remedy.
- **A fixture can pass for the wrong reason.** The rewritten test first asserted the
  foreign entry's place in the chain against a hardcoded id, then asserted it survived "a
  rewrite" by *appending* — which only adds bytes and tests nothing about rewriting. Both
  are fixed: the parent comes from the parsed first entry, and the rewrite is a `fork`,
  the operation that actually re-serializes entries into a new file.

## Verification

Battery `m113a` (`target/gate-m113a.log`, copied to
[`../../audit/evidence/gate-m113a.log`](../../audit/evidence/gate-m113a.log)): **all five
gates exit 0, 1,038 passed / 0 failed / 1 ignored across 59 test binaries** — six more than
M112's 1,032, which is five new contract tests and one new store test net of the rewritten
one. `epsilon-contracts` 30 tests, `epsilon-session` 16 unit + 31 integration.

Battery covers: an unknown kind opens the session, is reported by `preserved_unknown()`,
leaves the file byte-identical at open, keeps its place in the chain, stays out of the
model view, and survives a fork's re-serialization with its nested payload intact; a known
kind with an unreadable body is refused as `MalformedEntry` with the file untouched; and
the tag list matches the enum.

## Limits

- **Nothing surfaces the preserved entries yet.** `preserved_unknown()` exists and is
  tested; the CLI does not print it, so a user opening such a session still learns nothing.
  That is the next small slice, and it is what makes the preservation *loud* rather than
  merely safe.
- **`preserved_unknown()` covers the last scan only.** A store that has appended since
  reports the prefix it parsed; entries appended this session are written by this build and
  cannot be unknown, so the answer is right for the question asked — but a caller iterating
  after many appends should read it once, at open.
- **A preserved entry is not model-visible, by construction.** If a future kind *should*
  reach the model, preserving it silently would be wrong; the fix then is to name the kind
  in `KNOWN_ENTRY_KINDS` and give it a variant, which is exactly the pressure the design
  wants.
- **The importer is not built.** Foreign *envelopes* (pi's own session files) still refuse,
  and that is a larger piece of work with its own plan.
