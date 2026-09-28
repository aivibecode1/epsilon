# Installed binary verification — 2026-09-28, after M188

- **Binary:** `C:/Users/Hp/.cargo/bin/epsilon.exe`
- **Size:** 21,141,504 bytes (was 21,088,256 at M186)
- **Build:** `cargo install --path crates/epsilon-cli --features real --force`, ~19 min
- **Installed from:** `7fbb784`, source `30cb04c` (M188)

Replaces the M186 build. Carries all four new commands and the D26 wire fix.

## Content markers

`clear_queue`, `get_messages`, `get_commands`, `get_session_stats`, `costIsFloor` — 3
occurrences each (enum, handler `command` string, test name). `followUp` 3.

**`follow_up` shows 0 literal occurrences, and that is expected** — it is now a serde
`rename` attribute, not a string in the binary. A marker check cannot verify a wire name; the
frames below can. Worth remembering: the marker technique that has worked all session is
blind to exactly the class of change D26 was.

## D26 verified on the wire, both spellings

```
[a1] {"type":"follow_up"}   success=True  {'queued': 'followup'}   <- pi's documented name
[a2] {"op":"followup"}      success=True  {'queued': 'followup'}   <- the legacy spelling
```

**Before M188, `a1` returned `unknown op: follow_up`.** Both now reach the handler, and
`a1` arrived under pi's `type` tag rather than `op`, so it is reachable from a genuinely
pi-shaped client.

## The four new commands, over real stdio

```
[a3] get_session_stats  ctx=0  in=0  out=0  cost=None floor=False models=[]
[a4] get_messages       0 messages
[a5] get_commands       37 commands, first='models', !define defined=[False]
[a6] clear_queue        steering=0 followUp=2
[a7] get_tree           success=False  unknown op: get_tree
```

- **`a6` drained 2 follow-ups** queued by `a1` and `a2` — the pi-shaped frame, the legacy
  frame, and the drain all in one pipe session. This is the `clear_queue` arm end to end.
- **`a5`'s `!define` reports `defined: false`** rather than being omitted, so a palette
  shows it and knows it is unsupported.
- **`a7` refuses honestly** rather than fabricating a tree.
- **`a3` is `null`, not `$0.00`,** before anything ran.

## `costIsFloor` earning its place — both branches, live

With the **faux** provider, whose model is not in the price registry:

```
{"costIsFloor": true, "totalCostUsd": 0.0, "models": ["unknown"], "contextTokens": 10}
```

With the **default** model, a real catalog row:

```
{"costIsFloor": false, "totalCostUsd": 7.2e-05, "models": ["deepseek/deepseek-chat-v3.1"]}
```

This is why the flag exists and why `0.0` alone would have been a lie. An unpriced model
gives a real total of zero with tokens behind it; a client reading `0.0` as "free" would
report a session that spent money as costing nothing. The flag is the difference between
"nothing spent" and "spent an unknown amount".

Note the tokens are `0` in both — the deepseek call failed for want of credentials, but the
**failed assistant message still carries `model_id`**, which is what the per-model split
reads. The bill is attributed even when the call errored, which is the behaviour that makes
a failed run visible in a cost report.

## Not verified

- **No real provider call succeeded.** The deepseek run failed for want of credentials; its
  cost figure comes from a catalog price applied to a zero-token error response, so it
  proves the *pricing path* and not a real bill.
- **No two-model session was driven through the wire.** The per-model split is unit-tested
  with one model and the `"unknown"`/real-model contrast above; a session that switches
  mid-run and reports two rows is the case the split exists for and remains unmeasured.
- **`get_messages` with a large transcript was not driven.** The two-message reply is 992
  bytes on one line; the framing risk is a transcript of thousands, and the 557-row model
  list (365 KB, M186) remains the largest reply actually measured.
- **No live extension, no third-party extension** — all extension end-to-end work still uses
  `ext-fixture`.
