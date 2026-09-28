# Gate evidence — m186

Six pi RPC commands, closing the largest genuine gap in the port's breadth (9 of pi's 32
commands before this).

- **Source commit:** `80a91b3`
- **Log:** [`gate-m186.log`](gate-m186.log) — tracked, complete, ends in `GATE PASS`
- **Build:** `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`, Windows via
  `pwsh -File scripts/gate.ps1`

## The five gates

| Gate | Exit | Time |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | 2 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 368 s |
| `cargo test --workspace` | 0 | 729 s |
| `cargo run -q -p epsilon-cli -- --demo` | 0 | 37 s |
| `cargo check -p epsilon-cli --features real` | 0 | 52 s |

## Test totals

**63 targets, 1,425 results, 0 failed** — 51 test binaries + 12 zero-test doc-test targets.

`m185` measured **1,411** across the identical 63-target structure. The per-target lists were
compared directly, not the totals:

```
m185 non-zero targets: ... 74, 78, 106, 112, 150, 390   sum 1411
m186 non-zero targets: ... 74, 92, 106, 112, 150, 390   sum 1425
added:   [92]
removed: [78]
```

**Delta +14, entirely `epsilon-cli --lib` (78 → 92).** Every other target is byte-identical in
count, so no subtraction across unrelated measurements was needed.

## What was added

| Command | pi source | Core capability it uses (all pre-existing) |
|---|---|---|
| `cycle_thinking_level` | `rpc-types.ts:36` | `ThinkingLevel::next()` |
| `get_available_thinking_levels` | `rpc-types.ts:37` | `ThinkingLevel::ALL` |
| `set_model` | `rpc-types.ts:33` | `catalog::resolve_model_for_run`, `Agent::set_model_id` |
| `get_available_models` | `rpc-types.ts:34` | `catalog::curated()`, `row_credentials_ready` |
| `cycle_model` | `rpc-types.ts:35` | `catalog::curated()`, same-provider slice |
| `get_last_assistant_text` | `rpc-types.ts:68` | `Agent::history()` |

Every one refuses rather than half-acknowledges. A model that does not resolve is an error,
not a silent keep-the-old-model: that failure surfaces one request later as a provider error
naming a model the user never chose, which is the RPC-01 failure mode this port already had a
defect for. `cycle_model` stays inside one provider for the same reason.

## Negative control — the fifth of its kind this session

**The first version of this slice had no test of `set_model` succeeding.**

```
control: remove `agent.set_model_id(resolved.info.id)` from the handler
result:  all 25 RPC tests PASSED
```

Every `set_model` test I had written asserted a *failure*: an unresolvable name, a missing
model, a bare parse. A handler that refuses everything correctly passes all of them. The
control found it, not a reading of the code.

### The first correction was also wrong

The new test asserted the agent starts somewhere other than the target model, and failed on
**that precondition** — the default already is `faux-1`. Setting a model to the value it
already holds passes against a handler that does nothing, which is exactly the control. The
precondition a test depends on has to be the thing the control breaks. With the agent parked
on `m186-starting-model` first:

```
control -> 2 FAILED
  "core must actually hold the new model -- a success reply is not evidence"
  left: "m186-starting-model"  right: "faux-1"
real    -> 29 passed
```

## Two pre-existing tests became false and were rewritten

This is the cost of implementing an op a test had claimed was unknown, and it is worth
recording because the tests *passed* while asserting something untrue.

| Test | Was | Now |
|---|---|---|
| `ops_deserialize_with_snake_tags` | used `set_model` as the example of the `Unknown` fallback | uses `set_auto_compaction` (still genuinely unimplemented) and additionally asserts the six new variants parse |
| `unknown_op_reports_failure_with_the_echoed_id` | used `set_model`, `cycle_model`, and a pi-shaped `modelId` frame | uses `get_tree`, `export_html`, `fork` |

The second test's example set is now small and shrinking. That is intended friction: the next
person to implement one of those has to update the test, and cannot leave a passing assertion
behind that describes the opposite of the code.

## Other things the tests caught

- **Eight of my own assertions read the payload at the top level.** `ok_line` puts it under
  both `data` and `ok`; `command`/`success`/`id` are top level. All eight read `reply["x"]`.
- **A regex repair pass ate the `"]["` separator** and produced `reply["datareply["x"]`.
  Repaired, and the lesson recorded in the commit: an index expression is not regex-shaped
  text.
- **A slice-based edit removed the file's closing brace**, leaving an unclosed `mod tests`.
  Recovered by restoring the tail verbatim rather than reverting the file, which would have
  cost 495 lines of work.

## What was not verified

- **No real editor client was driven over stdio.** The dispatch path is tested in-process
  (`dispatch_line`), which is where every decision lives, but the stdio loop itself — the
  line reader, the writer, the flush on a partial line — is exercised only by the existing
  loop tests. A new op that writes a multi-KB model list is the case most likely to expose a
  line-splitting bug, and it was not driven through a pipe.
- **No `bash`, `fork`, `get_tree`, `switch_session`, `new_session`, `export_html`,
  `clear_queue`, or compaction-over-the-wire.** Those remain `unknown op`; 26 of pi's 32
  commands are still unimplemented.
- **`set_model` was verified only against `faux-1`**, the one catalog row that needs no
  credentials. The credential-refusal branch (`no credentials for <provider>`) was not
  executed, because no test in the workspace sets a provider key — and a test that
  depended on one would be env-gated, not deterministic.
- **The installed release was not rebuilt.** `C:/Users/Hp/.cargo/bin/epsilon.exe` is the
  M185 build and has none of these six commands.
