# M214 — the notice M210 obsoleted, asked the question that survived

**Status:** landed. GATE PASS `m214-r2` — see [Verification](#verification). Reinstall: **required
and outstanding** (the sentence is user-visible; one release build carries it).

## What

[A20](../../audit/03-defects.md) fixed. M205 added a launch notice for the state a **fixed file per
frontend** produced — *"this session already holds N entries — this launch starts a fresh
conversation in the same file"*. M210 gave each launch a file of its own, so the count it read was
always zero and the notice never fired; in the one case it could still fire, its wording was false.

The question that survived the change is **"does this workspace know about earlier conversations?"**,
and the answer lives in the directory rather than in this file:

```text
• this workspace holds 3 earlier sessions — `epsilon -c` opens the most recent, `/resume <file>`
  picks another
```

Three details, each of which is a decision rather than typing:

- **It counts the *other* files**, excluding this launch's own, or every launch would report one
  earlier session — itself.
- **It uses `epsilon_session::list_sessions`**, which already skips files that are not sessions
  (the directory is shared with other tooling) and knows how to read a header. A hand-rolled
  `read_dir` here would be a second answer to "what counts as a session", which is the defect A12
  documented in the config directory.
- **It reads the directory, not the store.** That is the whole change: the store is this launch's
  file and can only ever answer "nothing", while the directory is what the user opens with `-c`.

## The test fixture changed with it, and that needed care

The old tests wrote entries into the app's own store, which is exactly the thing that no longer
matters. The new ones **plant a second session file beside it**, and they do it by opening a real
`SessionStore` and appending an entry rather than by writing header JSON by hand — a hand-rolled
header would rot the moment the format moves, and would let the test pass while the product could not
read the file.

`a_launch_in_a_workspace_with_earlier_sessions_says_so`, `the_notice_is_plural_and_absent_when_there_is_nothing_earlier`,
and `a_launch_that_did_restore_the_history_says_nothing`.

## Verification

GATE PASS, 2026-09-30 — battery **`m214`**, documentation final before it started, disk knob as
M208 established. Seven gates, exit 0, per-gate seconds in order: 0s, 0s, 3s, 4s, 160s, 4s, 1.

**1,633 tests across 65 binaries, 0 failed** — the same total as M213, which is right: three tests
were *rewritten* rather than added, because the old three asserted the question that no longer has an
answer.

**Two mistakes on the way, both recorded because both are cheap to repeat:** the first battery failed
`fmt --check` only (my own formatting, no behaviour), and the changelog entry silently did not
happen because a `Set-Content` call was missing `-Value` so PowerShell rejected the whole
statement — the edit was assumed rather than verified, and only a grep for the new sentence caught
it. The retry used a fresh label (`m214-r2`) per the rule that a reused label destroys the failed
attempt's evidence.

Evidence: [`audit/evidence/gate-m214.log`](../../audit/evidence/gate-m214.log).

**Reinstall: required** — the sentence is user-visible — and **outstanding** (2026-09-30): the
installed binary carries M212, and this notice reaches it with the next release build.

## Limits — what this does not prove

- **No console check.** This is a startup notice, and the evidence is the notice channel plus the
  frame tests that already cover notices; nothing launched the TUI in a workspace with earlier
  sessions and read the line off a real screen. M205's own slice said the same, and the honest
  position is unchanged: the notice path is exercised, the sentence is not photographed.
- **The count is a count of files, not of conversations.** A directory holding three empty sessions
  and one real one reports four; `list_sessions` filters foreign files, not empty ones. The sentence
  says "sessions" and that is what they are, but a reader might expect "conversations".
- **Nothing prunes, so the number only grows.** A workspace used daily reports a larger number every
  day, which is honest and also a reminder that retention is still an undesigned product question
  (M210's Limits, M211's Limits).
