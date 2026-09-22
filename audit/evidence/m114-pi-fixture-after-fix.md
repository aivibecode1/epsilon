# M114 end-to-end — pi's own session now resumes with its prompts intact

The same experiment as
[`pi-import-empties-messages-2026-09-22.md`](pi-import-empties-messages-2026-09-22.md),
re-run against the binary built from this round's tree.

```sh
cp references/pi-0.85.1/packages/coding-agent/test/fixtures/before-compaction.jsonl /tmp/pi-after-fix.jsonl
target/debug/epsilon.exe --resume /tmp/pi-after-fix.jsonl --print hi
resumed 372 entries          # exit 0
```

Line 2 of the rewritten file, parsed:

```
text length: 668
head:        "alright, read @packages/coding-agent/src/main.ts @packages/c…"
```

Before the fix the same run reported `resumed 369 entries`, exit 0, and that text was
**0 characters**. 372 is 369 plus the three entries the offline scripted turn appended.

## What the fix does not cover, so this file is not read as more than it says

The restart rewrote the file again, so it is still a **conversion in place** — the header
still lost `provider`, `modelId`, `thinkingLevel` and `branchedFrom`, because
`SessionHeader` models only the fields this build writes. Both are recorded as the remaining
two parts of G19 in `audit/11`, and both are design decisions rather than patches: the
header needs the same "preserve what you do not understand" rule M113 applied to entry
kinds, and the write needs the importer plan's own import-as-copy rule that the resume path
does not follow.

Until those land, `--resume` on a pi session is **correct but not safe**: the transcript is
faithful, the original pi file is gone.
