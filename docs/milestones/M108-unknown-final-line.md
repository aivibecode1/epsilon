# M108 — a complete line this build cannot read is refused, not deleted

## What

`SessionStore::open` repairs a crash by dropping a final line it cannot parse. That is
right for what it was written for and wrong for what it was doing, because "cannot parse"
covers two situations that need opposite answers:

- **A fragment.** The process died inside a value — no closing brace, no newline, nothing
  that parses. It is debris; the user never had that turn; removing it costs nothing.
- **A complete line in a shape this build does not know.** Somebody wrote it on purpose.
  It is the user's newest turn, or an entry from a newer Epsilon, and deleting it silently
  is the one thing a recovery path must never do.

The store treated both as the first case. `reindex` asked only "is this the last line and
did it fail to parse?", so a session file whose final entry used an unknown `kind.type`, or
held bare JSON of any shape, had that line truncated at open — while the same bytes in a
mid-file position were refused with a typed diagnosis. Position decided whether the data
survived, which is not a rule anybody would choose deliberately.

This is F08 in [`audit/03-defects.md`](../../audit/03-defects.md), whose
acceptance criterion was already written down and unmet: *"unknown complete final entry …
refused with the entire file unchanged."* The repository's own test asserted the opposite
behavior and named it as intended (`a_torn_tail_is_still_tolerated_whatever_its_shape`,
which looped over a cut line **and** a complete `{"kind":{"type":"future_thing"}}` line).

## Key decisions

- **The discriminator is syntax, and only syntax.** `is_whole_json(fragment)` asks whether
  the line parses as a JSON value. If it does, the write finished, so the line is not
  debris and the store refuses it. If it does not, the write was interrupted, so the store
  removes it. One question, asked at every place that used to ask "is it last?".
- **The refusal is typed, not generic.** `LineDiagnosis` gains `UnknownLineShape` for
  "valid JSON, none of the shapes I know" — distinct from `NotJson` ("damaged") and from
  `UnknownEntryType` ("an entry from a newer build"). The three call for different actions
  from whoever reads the message, which is what the type exists for (M75's rule).
- **Refusing beats repairing, and the file must not be touched.** A refusal that truncated
  first and reported the problem afterwards would be the same data loss with a message
  attached. The test asserts byte-identity of the whole file, not just that an error
  came back.
- **The old test was rewritten rather than deleted.** Its subject — "a torn tail is
  tolerated" — is still true and still important; what changed is that its cases now
  contain only genuine fragments. The complete-line case moved to its own test, which is
  where the new rule is pinned.

## Gotchas

- **`if i + 1 == last` was doing two jobs.** It meant both "this is the end of the file"
  and, implicitly, "so damage here is probably a crash". Splitting the second meaning out
  is the whole fix; the loop now reads as three conditions instead of one.
- **A trailing newline is not what makes a line complete.** A completed append keeps one,
  a kill mid-write usually loses it, but a hand-edited or foreign file can end either way.
  The test drives both spellings for every shape, because the first version of the fix
  would have passed with `trim_end()` missing.
- **Restoring a file by moving a backup back keeps its old mtime, so cargo re-runs the
  stale test binary.** The negative control below first appeared to fail *after* the fix
  was restored, which is exactly backwards; `cargo test` had reused the artifact built
  from the reverted source because the restored file's timestamp was older than the
  binary's. Touching the file fixed it. Worth knowing before trusting a green run after
  any restore-and-verify step — the repo's own evidence discipline is built on that step.
- **Empty and blank lines needed no special case.** They are filtered before the loop, so
  the `""` case in the torn-tail test is "nothing after the newline", not an empty line.

## Verification

Battery `m106a` (shared with M106 and M107; same three files, one round, one battery —
the precedent is the M86–M88 round). All five gates exit 0, **1,021 passed / 0 failed /
1 ignored across 59 test binaries**; `epsilon-session` reports **30 integration tests and
16 unit tests, 0 failed**. Log:
[`../../audit/evidence/gate-m106a.log`](../../audit/evidence/gate-m106a.log).

**Negative control** (a pre-fix RED is available here, so it was run rather than
reasoned about): the three `&& !is_whole_json(fragment)` gates were reverted in place in
`crates/epsilon-session/src/store.rs` and the new test re-run. It failed with

```
a complete unknown final line was accepted: "{\"id\":\"ent_x\",\"seq\":2,\"kind\":{\"type\":\"future_thing\"}}"
```

— the property it is named for, failing for the reason it names. The file was then
restored and verified byte-identical by sha256 (`108B5BC17AB12355…`), the mtime trap above
was cleared, and the suite went green again.

**End-to-end, through the installed binary** (the strongest form of this evidence, and
the reason the reinstall was owed). A copy of a real session file with one appended
unknown-shaped entry, resumed with the reinstalled `epsilon.exe`:

```
$ epsilon --resume real-unknown.jsonl --print hi
open …\real-unknown.jsonl: session line #5: entry type `future_thing` is not known to
this build — the session was probably written by a newer Epsilon. Refusing rather than
dropping the line, because skipping it would change the transcript silently
(file left intact)
exit=1 · file unchanged (sha256) · 5 lines still on disk
```

The refusal names the right line, the right cause, and leaves every byte in place.

Global `epsilon.exe`: **reinstall required, and done.** Session open is the CLI's resume
path; a file that used to be silently truncated now refuses with a message, which is
user-visible behaviour. The installed binary is rebuilt from this round's tree.

## Limits

- **Refusing means a newer session file cannot be opened by an older build at all.** That
  is the intended trade — no data is destroyed and the message says which shape was
  unrecognized — but it is a real cost: the user must upgrade rather than read a session
  with its last line missing. Nothing in the UI offers "show me the rest anyway", and
  offering it is a product decision not taken here.
- **The repair still deletes a fragment that might have been meaningful.** A kill between
  `write` and the newline loses that turn; that is unchanged, and it is the behaviour the
  torn-tail test pins. What distinguishes the two cases is whether the bytes form a
  complete value, not whether the user cared about them.
- **Only the store's own reader is covered.** `crates/epsilon-session/src/search.rs` and
  the TUI's session browser parse session files directly in places; whether they classify
  an unknown final line the same way was not checked in this slice.
- **A known `kind.type` with an unreadable body is reported as an unknown type.** Probing
  the new path with a hand-written fixture — `kind.type` `message`, but missing fields the
  current `SessionEntry` requires — produced *"entry type `message` is not known to this
  build — the session was probably written by a newer Epsilon"*. The refusal was correct
  and the file was untouched, but the diagnosis names the wrong cause: `message` is a type
  this build knows; what it could not read was the body. `diagnose_line` keys on the
  presence of `kind.type` alone, so it cannot currently tell "an entry from a newer build"
  from "an entry of a known type whose shape changed". Worth a follow-up:
  `UnknownEntryType` only when the type is genuinely unrecognized, and a separate
  `MalformedEntry { entry_type }` otherwise. Recorded rather than fixed here because it
  needs the list of known `kind.type` values to live somewhere the diagnoser can reach.
