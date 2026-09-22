# M114 — the healer that read pi's messages and emptied them

## What

`heal_legacy_line` converts pi's pre-v2 session entries into this build's format. It read
`message.content` with `as_str()` — but pi writes an **array of parts** — so `as_str()`
failed and `unwrap_or("")` produced an empty message. The same branch ignored
`message.role`, so every role became a *user* turn, and read `timestamp` as a number when
pi writes an ISO string, so every entry was re-dated to the moment of the import.

Running it is what found it ([G19](file:///dev/null), evidence
[`../../audit/evidence/pi-import-empties-messages-2026-09-22.md`](../../audit/evidence/pi-import-empties-messages-2026-09-22.md)):
resuming pi's own fixture reported "resumed 369 entries", exit 0, and turned a
668-character prompt into nothing. The port's headline interop claim was, in practice,
worse than a refusal — it succeeded silently.

## Key decisions

- **Read the role, and map all four of pi's.** `user` → `Message::User`, `assistant` →
  `Message::Assistant`, `toolResult` → `Message::ToolResult`, and everything else —
  pi's `bashExecution` among them — to `Message::Custom` with the raw object inside, so the
  entry is preserved rather than guessed at. An assistant turn is healed *faithfully*:
  `provider`, `model` → `model_id`, `stopReason` → `stop_reason` (including pi's `toolUse`,
  `length`/`maxTokens`, `error`, `aborted`), `usage`, and `errorMessage`.
- **Convert the content array part by part, and keep what cannot be converted.** Text parts
  and `toolCall` parts become typed blocks; anything else comes back raw and rides in the
  entry's own free-form slot (`details` on a tool result). A `user` turn carrying a part
  this build cannot type becomes `Custom` rather than a user message *minus* that part,
  because a silently shortened message is the same defect one level down.
- **Both timestamp spellings, and the finer one wins.** pi stamps a message line twice: an
  ISO string on the line and epoch milliseconds inside the message, milliseconds apart. The
  millisecond field is preferred so the entry and its message carry one instant, and the ISO
  parse is a hand-written civil-date conversion — no new dependency for a format the repo
  writes in one place and reads in another.
- **No silent drop anywhere.** The function's contract is now: every pi line becomes a typed
  entry, and anything not understood is inside it, reachable, rather than gone.

## Gotchas

- **A stub that returns a plausible value is invisible.** `unwrap_or("")` did not look like
  data loss; it looked like a default. The test that would have caught it has to assert the
  *content* of a healing result, not that healing returned a message.
- **The bug was in the path that made the feature look finished.** The store reported 369
  entries and exit 0; the transcript was empty. Counting entries is not reading them.
- **pi stamps the same instant twice, in two formats.** Anything that reads one and ignores
  the other will disagree with pi by milliseconds at best and by decades (the import time)
  at worst.

## Verification

Battery `m114a` (`target/gate-m114a.log`, copied to
[`../../audit/evidence/gate-m114a.log`](../../audit/evidence/gate-m114a.log)): **all five
gates exit 0, 1,043 passed / 0 failed / 1 ignored across 59 test binaries** — five more than
M113's 1,038, which is exactly the new tests. `epsilon-session` reports 16 unit + **36**
integration tests.

**The end-to-end check, on pi's own fixture, before and after:**

```
before  resumed 369 entries · line 2 text: 668 chars -> 0 chars
after   resumed 372 entries · line 2 text: 668 chars, "alright, read @packages/coding-agent/src/main.ts …"
```

The 372 is 369 plus the three entries the (offline, scripted) turn appended. The prompt
comes back whole, which is the claim this milestone exists to make.

The new tests read pi's real line shapes: a `user` entry heals with its exact text, role and
millisecond stamp; an `assistant` entry keeps its `toolCall` id, name and arguments (an
assistant turn that asked for nothing while the tool result below it answers something is
worse than an empty user turn); a `toolResult` heals as a tool result; an unmappable part
and an unknown role are preserved with an assertion that proves the data is still reachable;
and a pi-shaped file round-trips through `persist_migration` with nothing lost.

## Limits

- **`model_change` and `thinking_level_change` are still preserved-but-not-applied.** They
  heal into `Custom` with their raw JSON, so a pi session's model switch does not take effect
  when resumed. Mapping them is small (`provider`/`modelId` and `level`), and it was
  deliberately left rather than half-done.
- **The header still drops what it does not model** and **conversion still happens in
  place** — the other two parts of G19, recorded in `audit/11` as separate slices. Fixing
  the messages makes the conversion correct, not safe.
- **No test drives pi's actual fixture file.** The tests use pi's line shapes verbatim, and
  the real-fixture run was done by hand; a checked-in fixture test would be stronger.
- **`bashExecution` has no typed home.** It is preserved faithfully and contributes nothing
  to the model view, which is honest but means such a session shows a gap where the command
  was.
