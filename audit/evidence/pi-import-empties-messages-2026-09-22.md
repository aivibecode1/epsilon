# Resuming a pi session silently empties it (found 2026-09-22)

**Severity: data loss on a supported path.** Epsilon can *open* a real pi session file, and
what it produces from it is a transcript with every message emptied, every message turned
into a user message, and every timestamp replaced by "now" — and then it **rewrites the file
in place**, so the original is gone.

This was found by running the importer for the first time rather than reading it. The
empirical step was the point: the port re-check said "pi entries are refused at line 2", the
plan repeated it, and neither is true of pi's own fixtures — which makes the real defect
worse, not better, because the path *succeeds*.

## The reproduction

pi ships two real session files, and both are read:

```sh
cp references/pi-0.85.1/packages/coding-agent/test/fixtures/before-compaction.jsonl /tmp/pi-v1.jsonl
epsilon --resume /tmp/pi-v1.jsonl --print hi

resumed 369 entries from /tmp/pi-v1.jsonl      # exit 0
large-session.jsonl: resumed 914 entries       # exit 0
```

Both exits are 0 and the file is **not** unchanged. Line 2 before and after:

```
before  {"type":"message","timestamp":"2025-12-08T22:41:05.306Z","message":{"role":"user",
         "content":[{"type":"text","text":"alright, read @packages/coding-agent/src/main.ts …"}]}}

after   {"id":"ent_1a0c65805f10","seq":0,"parent_id":null,"timestamp_ms":1790034052593,
         "kind":{"type":"message","message":{"role":"user","id":"msg_1a0c65805f11",
         "timestamp_ms":1790034052593,"content":[{"type":"text","text":""}]}}}
```

Measured on that line: the original text is **668 characters**, the rewritten text is
**0**. The header lost `provider`, `modelId`, `thinkingLevel` and `branchedFrom` as well.

## The cause, in one function

`heal_legacy_line` (`crates/epsilon-session/src/store.rs:1461-1522`), the migration path
that exists precisely to make pre-v2 files readable:

```rust
let text = map
    .and_then(|m| m.get("message"))
    .and_then(|m| m.get("content"))
    .and_then(|c| c.as_str())      // pi sends an ARRAY of parts, not a string
    .unwrap_or("")                 // …so every message silently becomes empty
    .to_string();
```

Three defects in five lines, each invisible because the other two hide it:

1. **`content` is an array of parts.** pi writes `[{"type":"text","text":"…"}]` and also
   `[{"type":"toolCall","id":…,"name":…,"arguments":…}]`. `as_str()` on an array is `None`,
   and `unwrap_or("")` turns "I do not understand this" into "there was nothing here".
2. **Every message becomes `Message::User`.** The branch ignores `message.role` entirely,
   and pi's fixture has 484 assistant messages, 448 tool results, 55 user messages and 3
   `bashExecution` messages. All of them come back as user turns.
3. **Timestamps are read as numbers.** pi writes an ISO string (`"2025-12-08T22:41:05.306Z"`),
   so `as_u64()` is `None` and every entry is stamped `now_ms()` — the whole transcript is
   re-dated to the moment of the import.

## What pi's files actually contain

From `before-compaction.jsonl`: `message` 990, `model_change` 5, `thinking_level_change` 5,
`compaction` 2. Roles: `assistant` 484, `toolResult` 448, `user` 55, `bashExecution` 3.
Content parts: `{"type":"text","text":…}` and
`{"type":"toolCall","id":…,"name":…,"arguments":…}`.

The entry kinds other than `message` and `compaction` are healed into
`Custom { custom_type: "legacy", data: <the raw value> }` — lossless, but not *applied*, so a
model switch or a thinking-level change in a pi file is currently invisible to the harness.

## Two more decisions the run forces

- **It rewrites a foreign file in place.** `--resume` on a pi file converted it to Epsilon's
  envelope, header included. The plan for the importer already said import-as-copy, never
  in-place, because both formats call themselves version 3 and neither can read the other
  after conversion. The current behaviour is the opposite of that, and it happens on the
  *resume* path, not behind an import command.
- **Nothing warns.** `preserved_unknown()` reports entries kept *unparsed*; these entries are
  parsed and emptied, which no report covers. A conversion that loses content must say so
  before it writes.

## What a fix must do

1. Read `message.role` and map each role to the right variant: `user` → `Message::User`,
   `assistant` → `Message::Assistant`, `toolResult` → `Message::ToolResult`;
   `bashExecution` has no equivalent and belongs in `Custom`, preserved.
2. Convert the content **array**: `text` parts to text blocks, `toolCall` parts to tool
   calls, and anything unrecognized preserved rather than dropped.
3. Accept both timestamp spellings (ISO string and epoch number).
4. Refuse to *write* a converted foreign file without either copying it first or asking —
   and report what the conversion could not carry.
5. Tests: a pi-shaped user message, assistant message with tool calls, and tool result each
   heal with their text, role and timestamp intact; an unrecognized role and an unrecognized
   content part are preserved, not dropped; and a round trip through `persist_migration` on a
   pi fixture loses nothing.

Until that lands, **`--resume` on a pi session file is destructive** and should not be done
without a backup.

## The v2 question, answered by running it

pi's shipped fixtures are v1-shaped, so the run above did not settle what happens to a
*current* pi file — one whose entries carry `id`/`parentId`, which is what pi's own
`migrateV1ToV2` adds (`session-manager.ts:230-242`). So one was built: every non-header line
of `large-session.jsonl` given an `id` and a `parentId` chained to the previous entry.

```
$ epsilon --resume pi-v2.jsonl --print hi
open …\pi-v2.jsonl: session line #2: valid JSON that is not an entry, a header or a legacy
entry — a shape this build does not know. Refusing rather than dropping the line, because
a session written by a newer Epsilon is not a damaged session
exit=1
```

Refused, at the first entry, with a diagnosis that is honest about what it knows and does
not touch the file. So the port re-check was right about **v2** files and wrong about the
fixtures — and the two failures are different in kind:

| pi file | today | why |
|---|---|---|
| v1 (fixtures) | **opens, and empties every message** (G19) | legacy healing reads a shape it cannot actually read |
| v2 (current pi) | refused at line 2 | the envelope has no `kind`, so the line is "a shape this build does not know" |

The second is a missing importer, which is honest work. The first is a defect, because it
*succeeds*. Fixing the healer makes v1 files genuinely readable; the importer (slice 4B) is
what makes v2 files readable at all.

## What is not claimed here

- ~~Whether pi *v2* files (entries carrying `id`/`parentId`) refuse or take a different
  path~~ — answered above: they refuse at line 2.
- Whether a full conversion round-trips without loss for the entry kinds other than
  `message` — the `Custom` fallback keeps their bytes, and that was not exercised against a
  written file.
