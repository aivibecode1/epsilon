# Sessions and recovery

For users inspecting or resuming transcripts. Keep private backups before repair
or restore; transcripts may contain sensitive source and tool output.

## Where sessions live

One JSONL file per session under `<workspace>/.epsilon/sessions/`. Entries form a tree
(each carries ids and parents), so a session can be forked, browsed by timeline and resumed
rather than merely replayed.

TUI: `/new` `/resume` `/sessions` `/session` `/tree` `/timeline` `/fork` `/clone` for
lifecycle; `/export` (html/json) writes to `.epsilon/exports/`; `/import` `/share` `/copy`
move transcripts around; `/search` finds text across history; `/name` renames; `/compact`
compresses context.

CLI:

```sh
epsilon -c                    # continue the newest session in this workspace
epsilon --resume <file>       # resume a specific session file
```

## One writer per session

A cross-process **lease** (heartbeat-enforced) is intended to admit a single writer. A second
`epsilon` attaching to the same session is refused and names the current holder; a genuinely
stale lock (crashed process) is stealable.

## Compaction and current limitations

Compaction appends summary entries rather than deleting old messages. However,
successive summaries accumulate in the current durable model projection (F04) and
failed summaries can replace context (F06). A durable append that fails is **no longer
ignored**: the turn ends before the provider is called, the failure is reported as a
notice, and later requests refuse to build a request from a session missing a turn the
loop recorded (F07, fixed in M122). The append-only design alone does not guarantee
correct recovery.

## Recovery from damage

Repair runs when a session opens:

- A transcript ending mid-tool-call gets the missing results synthesized so the file stays
  valid for the next run.
- Many malformed or incompatible entries receive typed refusal, but unusable final
  entries and a final second header can be mistaken for torn writes and truncated
  (F08). A header followed by a torn first entry can lose its header during repair.
  See F08 in [the defect ledger](../../audit/02-defect-ledger.md) for the
  source evidence and required preservation guarantees.

Practical guidance:

- Back up `.epsilon/sessions/` privately before reopening damaged files; opening can
  mutate a tail. Never assume unknown final content will be preserved.
- Local cancellation can leave returned partial text absent from history (F17).
  Resume is not proof that all displayed text was persisted.
- `/tree` currently shows the active branch; `/fork` copies the file without switching
  to it. These are not complete pi session workflows.

## Checkpoints (revert)

File-level checkpoints exist for undoing tool edits, but restore-safety issues are **open
audit findings (F15)** — before relying on checkpoint restore, read
[`../../audit/02-defect-ledger.md`](../../audit/02-defect-ledger.md). Keep sessions under
private backup for recovery; do not commit sensitive session transcripts to a public repository.
