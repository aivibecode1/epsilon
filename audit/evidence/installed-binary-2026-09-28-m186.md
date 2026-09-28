# Installed binary verification — 2026-09-28, after M186/M187

- **Binary:** `C:/Users/Hp/.cargo/bin/epsilon.exe`
- **Size:** 21,088,256 bytes (was 21,016,064 at M185)
- **Build:** `cargo install --path crates/epsilon-cli --features real --force`, 17m 45s
- **Installed from:** `fa770ca` (docs), last source `80a91b3` (M186) plus `38e7a5e` (tests)

Replaces the M185 build, which had the prompt templates but neither the six RPC
commands nor the history-integrity tests.

## Content markers

`get_available_thinking_levels`, `cycle_thinking_level`, `get_available_models`,
`get_last_assistant_text`, `cycle_model` — 3 occurrences each; `credentials_ready` — 1. The
count of 3 per command is the op name in the enum, the handler's `command` string, and the
test name; a count of 1 would have been suspicious in the other direction.

## End-to-end over real stdio

Eight frames written to the installed binary's stdin, replies read from stdout. This is the
`--rpc` loop itself, not `dispatch_line`, so the line reader, writer and flush are covered —
the gap the M186 evidence file recorded as unverified.

```
[ r1] get_available_thinking_levels   success=True  levels=['off','minimal','low','medium','high','xhigh','max']
[ r2] cycle_thinking_level            success=True  level=minimal
[ r3] get_state                       success=True  level=minimal
[ r4] set_model (provider/model pair) success=True  model=faux-1
[ r5] get_last_assistant_text         success=True  text=None
[ r6] get_available_models (filtered) success=True  models=1
[ r7] cycle_model                     success=True  model=faux-1
[ r8] get_tree                        success=False error=unknown op: get_tree
```

Three things this proves that the unit tests could not:

- **`r1` used pi's `type` tag, not `op`.** One enum reads both dialects, and the new ops are
  reachable from a pi-shaped client exactly like the old ones.
- **`r3` shows the change persisted.** After `cycle_thinking_level` moved the level to
  `minimal`, an independent `get_state` reports `minimal` — the reply was not an echo.
- **`r8` refuses honestly.** An unimplemented pi command gets `unknown op: get_tree` with its
  id echoed, not a fabricated success.

## The largest reply the surface can produce

`get_available_models` with no filter was driven afterwards, since it is the case most likely
to stress the line writer:

```
exit=0  elapsed=0.08s  stdout=365,819 bytes  reply lines=1
rows=557  providers=15  malformed rows=0  credentials_ready=7/557
```

One line, not 557 — the payload did not break the framing. 0.08 s for a 366 KB reply.

**`credentials_ready: 7/557` is the field earning its place.** Without it a client would offer
550 models this machine cannot run, and the failure would surface on the first request as a
provider auth error. The picker avoids this by grouping providers; the wire now carries the
same fact per row.

## Not verified here

- **No real provider, no live extension, no `set_model` to a credentialed model** — this
  machine has no provider keys, so `faux-1` is the only row that can be selected, and the
  credential-refusal branch is still only exercised by reading the code.
- **The prompt-template probe was not re-run**; the binary carries those markers unchanged and
  the M185 evidence covers the behaviour.
