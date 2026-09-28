# Installed binary — M190/M191, 2026-09-28

**Binary:** `C:\Users\Hp\.cargo\bin\epsilon.exe`, **21,312,000 bytes**, installed from
`d4df88e` (M190 D27 fix) with `cargo install --path crates/epsilon-cli --features real
--force`. 6m 41s.

M191 (`d015d10`) added tests only, so the shipped bytes are the M190 build.

## D27 confirmed over a real wire

Seven frames written to the child's stdin, replies read from stdout
(`scripts/drive_rpc2.py` in the session scratch; the harness is described below):

```
[0] set_session_name     data=OBJECT{name}
[1] prompt               data=OBJECT{started}
[2] get_tree             data=OBJECT{leafId,tree}  roots=1 nodes_with_children=2 depth=2
                               entry_keys=['id', 'parentId', 'timestamp', 'type']
[3] get_entries          data=OBJECT{entries,leafId}
[4] get_messages         data=ARRAY[1]
[5] get_commands         data=ARRAY[35]
[6] get_session_stats    data=OBJECT{cacheReadTokens,cacheWriteTokens,contextTokens,
                                        costIsFloor,inputTokens,models,outputTokens,
                                        totalCostUsd}
```

**`get_messages` and `get_commands` return `ARRAY`, not `OBJECT`.** That is D27's second
half, on the wire: M188 wrapped both, so a pi client iterating the reply would have got
key names instead of messages, and `.length` would have returned 1 instead of 35.

**`get_tree` nests.** A real node, from the same run:

```json
{ "children": [ { "children": [ { "children": [ { "children": [ {
    "children": [],
    "entry": { "id": "ent_1a0e739fa4d5",
               "parentId": "ent_1a0e739fa4b2",
               "timestamp": "2026-09-28T08:55:31.661Z",
               "type": "message" },
    "label": null, "labelTimestamp": null } ] } ] } ] } ] }
```

Five levels deep on a session of a few messages, `entry_keys` exactly pi's four
(`id`, `parentId`, `timestamp`, `type`), and `timestamp` an **ISO-8601 string with
millis and a `Z`** — not the `timestampMs: number` M189 sent.

## The date conversion, measured rather than believed

M190's milestone recorded `epoch_millis_to_iso8601` as *"reasoned, not measured"*.

- **200,000 random instants** across the `u64` millisecond range compared against Python's
  `datetime.fromtimestamp(..., tz=utc)`: **0 mismatches**. Includes `2000-02-29`
  (divisible by 400 → leap day) and `2100-01-01` (divisible by 100 but not 400 → *not* a
  leap day), the two cases that catch a botched century rule.
- Two **hand-computed expected values in the Rust test were wrong** — the 2026 year-end
  instant was off by 14,600,000 ms because the year arithmetic was done in my head. The
  function was correct both times. This is now a comment in the test.

## What this probe cannot tell you

- **No fork was created on the wire.** The `depth=2` / `roots=1` result is a *linear*
  session. The unit test covers a real `navigate_to` fork; driving one over stdio needs
  session-file manipulation the frame protocol has no command for (`fork` is one of the
  12 unimplemented pi commands).
- **The orphan and self-parent root cases are unexercised here** — they need a
  hand-edited session file.
- `get_session_stats` returned an object but the model was uncredentialed
  (`no credentials for provider "openrouter"`), so the cost fields are a floor. The two
  priced branches were measured in M188's probe against a priced catalog model.
- **The `prompt` frame started a run but no assistant text came back**, because the
  configured provider has no credentials on this machine. The `started` acknowledgement is
  what is being verified here, not a completed turn.

## Provenance correction, 2026-09-28

Two `cargo install --force` runs were launched **concurrently against the same
`D:/eps-target`** — one intended for the M189 source, one for M190. That violates the
one-build-owner-at-a-time rule, and the audit trail initially looked alarming: the M189
install reported `Finished release profile in 18m 19s` and `Replacing ... epsilon.exe`
*after* the M190 install had reported success and been probed.

**Resolved by measurement, not by argument.** Re-probing the installed binary after both
completed returns `get_messages` = `ARRAY[1]` and `get_tree` with
`entry_keys = [id, parentId, timestamp, type]` — **M190/D27 shapes**. An M189 binary
would return `data = OBJECT{messages}` and a flat branch. The 18-minute run therefore
compiled the M190 source: it waited on the shared target-dir lock until after the M190
edits were on disk, then built those bytes. Both runs report 21,312,000 bytes.

So the claim above stands — the installed binary is the M190 build — but it now rests on a
re-run probe rather than on the install log's ordering. The lesson is recorded in
`agents/memory/01-plan-execution.md`: a `cargo install` that finishes late describes the
tree it *compiled*, not the tree it was *launched* against, and two installs sharing a
target directory are two installs racing for one lock.
