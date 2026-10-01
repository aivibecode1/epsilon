# M243 — a sessions view that can actually switch sessions, and a data-loss guard it found

M242 deliberately did not add a Sessions view, and said why: *"a session list that does not
let you switch is a list of files with extra steps."* This adds it, with the switching.

## The switch is the same handover the terminal uses

`Agent::live_resume` — release the lease, open the other file, repair dangling tool calls,
re-derive the shadowed model view, restore the history, re-attach. It is what `epsilon`'s
`/resume` does, so a conversation moved between the two frontends is the *same* conversation,
not a re-import.

## The rebuild after a switch is not optional

**Nothing emits an event for a restored session.** `live_resume` puts the history onto the
agent; the event stream only carries what happens from now on. A window that reduced events
alone would switch files and show an empty pane — "nothing happened", reached by *opening*
something, which is the worst way to arrive at it.

So `Transcript::from_messages` builds rows from `Agent::history()` after every switch: prompts,
answers, reasoning, and tool calls with their results marked as passed or failed. The test
asserts the frame, not the struct.

## The bug this found: a mistyped path silently created an empty session

`SessionStore::open` **creates** what is missing. So `/resume does-not-exist.jsonl` — a typo, in
either frontend — produced a fresh empty session and the *next* write went into it, while the
reader believed they were continuing the conversation they named.

That is data loss wearing success's clothes: no error, no trace, and the old file untouched
so nothing looks wrong. `live_resume` now refuses a path that is not a file, and names the
alternative. The test asserts both halves: the refusal *and* that no file was left behind.

This is the Ponytail "protection against data loss" rule catching a case that no test was
looking for, found by asking what happens when a person types a name that does not exist.

## Clicking the session you are already in does nothing, on purpose

It is not an error and it is not a spinner. It is the same conversation, and the pane says so
with a marker rather than pretending an action is in flight.

## Verification

**Battery `m243` — GATE PASS on attempt 1, 1,783 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m243.log`). Up from 1,781: two window tests.

## What nothing exercises

- **A session held by another process.** The lease conflict path — two harnesses on one file —
  is the refusal a user will actually hit, and nothing drives it here.
- **Switching back and forth twice.** One switch is tested; the second switch would exercise
  the `already_holding` branch in `live_resume`, which is the fiddly one.
- **The list at scale.** `list_sessions_for_cwd` is the shared reader and has its own tests;
  nothing here checks a hundred rows in a 264-point pane.
- **Dates.** Each row shows a name and a marker; the modified time is *not* drawn, though the
  API has it. A reader comparing "which of these three is newest" gets the ordering and no
  evidence for it.