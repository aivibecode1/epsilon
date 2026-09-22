# M115 end-to-end — the conversion is now lossless and reversible

One run of the **installed** binary over pi's own fixture, checking all four things this
round and the last one set out to fix:

```sh
cp references/pi-0.85.1/packages/coding-agent/test/fixtures/before-compaction.jsonl /tmp/pi.jsonl
epsilon --resume /tmp/pi.jsonl --print hi
```

```
migrated C:\…\pi.jsonl to current version (previous bytes kept at C:\…\pi.jsonl.pre-epsilon)
resumed 372 entries from C:\…\pi.jsonl

header now : {"type":"session","version":3,"id":"ffae836b-…","timestamp":"2025-12-09T00:53:29.825Z",
              "cwd":"/Users/badlogic/workspaces/pi-mono","branchedFrom":"/Users/badlogic/.p…
backup exists: True   backup is the original bytes: True   (sha256 against the pristine fixture)
line 2 text: 668 chars
```

| What | Before this work | Now |
|---|---|---|
| message text | 668 chars → **0** | 668 chars |
| message role | every role became *user* | role preserved |
| timestamp | re-dated to the import time | pi's own instant |
| header | `provider`/`modelId`/`thinkingLevel`/`branchedFrom` dropped | kept |
| the original | overwritten, gone | byte-identical at `<file>.pre-epsilon` |
| the user | told "migrated" | told **where the previous bytes went** |

## What this does not claim

Conversion still happens **in place**: the session the user resumes is the file they named,
rewritten in Epsilon's format, and they must go back to the `.pre-epsilon` sibling to use it
with pi again. The importer plan's import-as-copy rule would instead produce a new file and
leave the original untouched, which is a behaviour change carrying a UX question — which file
am I in now? — and is recorded rather than taken.

`model_change` and `thinking_level_change` entries still heal into `Custom`: preserved, not
applied, so a pi session's model switch does not take effect on resume.
