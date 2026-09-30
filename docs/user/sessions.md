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

A cross-process **lease** (heartbeat-enforced) admits a single writer. What a second `epsilon`
sees depends on whether the first is alive, and the difference is worth knowing because the
second case looks like a hang for about four seconds:

| The holder is | What happens |
|---|---|
| **alive** | the second `epsilon` **waits**, drawing `waiting for this session to be released — Ns elapsed, ~Ms left` on the console, and is refused at the end of the window **naming the holder**. It is not refused immediately: waiting is cheaper than a false refusal when the first window is about to exit. |
| **gone** (crashed, or the terminal was closed) | its lock stops being re-touched, and after about **four seconds** of that the second `epsilon` takes it and starts. Four seconds is deliberate — a heartbeat is two — because the margin is what stops a *live* process that stalls for a moment from being mistaken for a dead one, and the steal is the only path by which two writers could share one session file. |

Locks left by a process that is gone are also **reaped at launch** (a lock untouched for thirty
seconds is deleted), so a workspace does not accumulate stale `.lock` files beside its sessions.

## Compaction, and what it guarantees

Compaction appends summary entries rather than deleting old messages, so the transcript keeps
every line while the model sees the compacted projection. That projection **replaces** the
summary it supersedes instead of stacking on top of it, and a summary is committed only when it
is usable — a non-failure stop reason, non-empty text, and an append that reached the disk (F04
and F06, fixed in M86). A cancelled run that salvaged a partial answer commits it like a
completed turn (F17, the same milestone). A durable append that fails is also **not** ignored:
the turn ends before the provider is called, the failure is reported as a notice, and later
requests refuse to build a request from a session missing a turn the loop recorded (F07, fixed
in M122; the `serve` path that still swallowed it, in M133). The append-only design alone does
not guarantee correct recovery.

## Recovery from damage

Repair runs when a session opens:

- A transcript ending mid-tool-call gets the missing results synthesized so the file stays
  valid for the next run.
- A truncated final line is repaired. A **complete** line this build cannot read is not: it is
  refused with a typed diagnosis and the file is left byte-identical, because position no longer
  decides whether data survives (F08, fixed in M108).
- An entry kind from a newer Epsilon is kept rather than used to close the session: it is carried
  on the next write, left out of the model view, and reported when the session opens (M113, M116).
- The header is the floor of any truncation, so a header followed by a torn first entry keeps its
  header — and with it the session's ids, its working directory and, for a delegated child, its
  parent link (M105).

Practical guidance:

- Back up `.epsilon/sessions/` privately before reopening damaged files: a genuinely torn tail
  is still repaired in place, and only your own copy proves what was there.
- `/tree` currently shows the active branch; `/fork` copies the file without switching
  to it. These are not complete pi session workflows.

## Checkpoints (revert)

Pre-edit file checkpoints are recorded, and the store refuses to restore one it cannot verify: a
restore compares **content**, not timestamps, and a file whose bytes differ from the capture is
left alone rather than overwritten (F15, fixed in M100 and M104 — the two halves were deleting a
file it could not restore, and clobbering edits made after the snapshot). What does not exist yet
is a *user-facing* revert: nothing outside the tests calls the restore path, so treat checkpoints
as an internal record rather than an undo key. Keep sessions under private backup for recovery;
do not commit sensitive session transcripts to a public repository.
