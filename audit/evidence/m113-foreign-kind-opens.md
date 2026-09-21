# M113 end-to-end — a session with a foreign entry kind opens

**The experiment.** A real session file with one added entry whose `kind.type` this build
has never known:

- line 1: the header, verbatim from `.epsilon/sessions/cli.jsonl`
- lines 2 and 3: two real message entries, verbatim (`seq` 0 and 1)
- line 4: `{"id":"ent_zz","seq":2,"parent_id":"<line 3's id>","timestamp_ms":2,
  "kind":{"type":"telemetry_v9","payload":{"n":1}}}`

**Before M113**, the same experiment with a different unknown type refused the whole
session — that run is preserved in
[`../../docs/milestones/M108-unknown-final-line.md`](../../docs/milestones/M108-unknown-final-line.md):

```
open …: session line #5: entry type `future_thing` is not known to this build — the session
was probably written by a newer Epsilon. Refusing rather than dropping the line
exit=1 · file unchanged · 5 lines still on disk
```

**After M113:**

```
$ target/debug/epsilon.exe --resume %TEMP%\eps-m113\foreign-real.jsonl --print hi
…
resumed 2 entries from C:\Users\Hp\AppData\Local\Temp\eps-m113\foreign-real.jsonl
exit=0 · file unchanged (sha256 verified before and after)
```

Two entries resumed, not three: the foreign entry is on the chain and in the file, and it
contributes nothing to the model view — which is the whole design. It is *kept*, not
*applied*.

## Two fixture traps, recorded because both cost a run

1. **`seq` must be gapless.** The first fixture gave the foreign entry `seq: 99`. The store
   indexes by `seq` — it *is* the file position — so the branch walk resolved the leaf to an
   index past the end, stopped with a chain break, and the CLI reported `resumed 0 entries`
   while still exiting 0. Preservation keeps an entry's payload; it does not make the chain
   tolerant of an incoherent index.
2. **Slurping a prefix of a long file keeps the original `seq` values.** Taking "the first
   three lines" of a real session and appending `seq: 3` skipped `seq: 2`, which broke the
   same way. The entry that follows a prefix must carry the next index, not the next number
   the author had in mind.

Both failures were *visible* — `resumed 0 entries` rather than a silent empty transcript —
which is the M91 rule doing its job: a `seq` that disagrees with line order is a chain
break, not a panic and not a wrong branch.

Re-run: `target/debug/epsilon.exe --resume <file> --print hi`.

## The same two runs against the installed binary

After the round's reinstall, `%USERPROFILE%\.cargo\bin\epsilon.exe` was checked on both
sides of the new rule, because a rule that only preserves would be as wrong as one that only
refuses:

```
$ epsilon --resume foreign-real.jsonl --print hi          # one foreign kind appended
resumed 2 entries … · exit=0 · file unchanged

$ epsilon --resume bad-body.jsonl --print hi              # `message` with no body
open …: session line #4: entry type `message` is one this build knows, but its body cannot
be read — the entry is damaged rather than foreign. Refusing rather than preserving it,
because an entry nothing can interpret would leave a silent gap in the transcript
(file left intact)
exit=1
```

The second message is the one M108's Limits asked for: before this round, a known kind with
a broken body was reported as *"not known to this build — the session was probably written
by a newer Epsilon"*, which pointed the reader at the wrong cause.
