# M115 — converting an older session keeps its header and its previous bytes

The last two parts of G19. M114 fixed the *messages* a pi session loses on conversion; the
file still lost two other things: everything in the header this build does not model, and the
original itself.

## What

**The header kept only what this build names.** pi writes
`{"type":"session","id":…,"timestamp":…,"cwd":…,"provider":"anthropic","modelId":"claude-opus-4-5","thinkingLevel":"off","branchedFrom":…}`,
and a rewrite produced `{"type":"session","version":3,"id":…,"timestamp":…,"cwd":…}` — the
model the session ran on, its thinking level and the session it branched from, gone. That is
the same failure M113 fixed for entry kinds, one layer up: the harness dropped what it did not
understand instead of keeping it.

**The conversion rewrote the file in place with no way back.** `persist_migration` writes the
converted file over the original (tmp + rename). For a session pi also reads, that is the only
copy in pi's format, so the conversion was correct and irreversible.

## Key decisions

- **Preserve unknown header fields the way unknown entry kinds are preserved.**
  `SessionHeader` gains a flattened map, documented with the same reasoning *and the same
  caveat*: `serde_json`'s map is sorted, so a rewrite is equal as JSON and not equal as bytes.
  The alternative — dropping unknown fields — is what made a pi session stop round-tripping
  through pi's own tooling.
- **The serde detail is proven, not assumed.** A `flatten` field switches the derive to a
  flat-map serializer, so `skip_serializing_if` on the *other* fields could plausibly stop
  being honoured; a test asserts the exact bytes of a header this build writes, which is the
  assertion that would catch a stray `"preserved":{}` or a `"version":null`.
- **A fork does not inherit the source's unmodelled fields.** A fork is a new session with a
  fresh id; copying a `branchedFrom` from another tool's header would make it claim an
  ancestry it does not have.
- **Back up once, beside the file, and never overwrite.** The copy is `<file>.pre-epsilon`,
  taken after the converted content is complete in the temp file and before the rename, so a
  failure before that point leaves nothing behind, and a failed copy fails the migration
  rather than converting without a way back. "Never overwrite" is what makes a second pass
  safe: the file already there is the true original, and replacing it with a converted copy
  would destroy the only remaining way back.
- **No migration, no backup.** The copy is taken only when the file actually needed
  migrating, so a current session never grows a sibling it has no use for.
- **The message names the backup.** Both CLI call sites now print where the previous bytes
  went. A conversion that changes the user's file and says only "migrated" is the same class
  of problem as the defect: something happened to their data and nobody said so. The path
  rule lives in the session crate (`original_backup_path`) rather than being re-spelled in
  each frontend, because a duplicated suffix is how two names drift apart.

## Gotchas

- **A `#[serde(flatten)]` field and `skip_serializing_if` interact.** The empty map must not
  serialize, and the nullable fields beside it must keep skipping — an exact-bytes test is the
  only cheap way to know.
- **The backup is the file as it exists at persist time.** A store that appended first (which
  lazily stamps a header onto a headerless file) would copy that stamped version; the resume
  path calls `persist_migration` first, so a pi file is copied exactly. Recorded because the
  difference is invisible from the outside.
- **Two representations of one name is a drift bug waiting.** The CLI could have hardcoded
  `.pre-epsilon`; exposing `original_backup_path` means the printed name is the written name.

## Verification

Battery `m115b` (`target/gate-m115b.log`, copied to
[`../../audit/evidence/gate-m115b.log`](../../audit/evidence/gate-m115b.log)): **all five
gates exit 0, 1,049 passed / 0 failed / 1 ignored across 59 test binaries** — six more than
M114's 1,043 (two contract tests, four store tests). `epsilon-contracts` 32 tests,
`epsilon-session` 16 unit + 40 integration.

`m115a` is the first attempt and failed `clippy` on a collapsible `if` in the backup call
site; the battery was stopped, the lint fixed, and the whole thing re-run under a new label
so the passing log covers the bytes that were committed.

New tests: a pi-shaped header keeps the fields this build does not model; a header this build
writes is unchanged in shape (exact bytes); converting a pi file keeps its header fields on
disk (asserted individually, through the real `persist_migration` path); a converted file
keeps the original bytes beside it (byte-for-byte against what was written, with an assertion
that the file really was converted); an existing backup is never replaced by a second
conversion; and a file that needs no migration leaves no backup.

## Limits

- **The original is kept, not the extension.** `.pre-epsilon` is one file beside the session.
  A user who wants a pi session to stay pi-readable has to know to look there; the notice
  tells them once, at conversion time, and nothing lists it later.
- **Conversion still happens in place.** This slice makes it *recoverable*, not *avoidable*.
  The importer plan's import-as-copy rule would make `--resume` produce a new file and leave
  the original untouched, which is a behaviour change with a UX question attached (which file
  am I in now?). Recorded, not taken.
- **`model_change` and `thinking_level_change` are still preserved but not applied** — a
  session's model switch heals into `Custom`, so resuming does not restore it.
- **No test drives pi's fixture file.** The shapes are used verbatim and the real-file runs
  were done by hand; a checked-in fixture test would be stronger.
