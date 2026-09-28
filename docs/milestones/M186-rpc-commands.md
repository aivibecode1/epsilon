# M186 — Six pi RPC commands, and a control that found I had tested no success path

Phase 4 work, and the largest genuine gap in the port's *breadth* rather than its depth.

## What

RPC mode implemented 9 of pi's 33 commands. These six more:

| Command | pi source | Core capability used (all pre-existing) |
|---|---|---|
| `cycle_thinking_level` | `rpc-types.ts:36` | `ThinkingLevel::next()` |
| `get_available_thinking_levels` | `rpc-types.ts:37` | `ThinkingLevel::ALL` |
| `set_model` | `rpc-types.ts:33` | `catalog::resolve_model_for_run`, `Agent::set_model_id` |
| `get_available_models` | `rpc-types.ts:34` | `catalog::curated()`, `row_credentials_ready` |
| `cycle_model` | `rpc-types.ts:35` | `catalog::curated()`, same-provider slice |
| `get_last_assistant_text` | `rpc-types.ts:68` | `Agent::history()` |

The point worth recording: **every one mapped onto core capability that already existed.**
`ThinkingLevel::ALL` and `.next()` were written for the TUI's `/think`. `catalog::curated()`
backs `/model` and `--list-models`. `Agent::set_model_id` and `history()` are the model's
state. They were being refused as `unknown op` for no reason at all — this was a wiring gap,
not missing work.

## Key decisions

**Every one refuses rather than half-acknowledges.** A model that does not resolve is an
error, not a silent keep-the-old-model. That failure surfaces one request later as a provider
error naming a model the user never chose, which is exactly the RPC-01 failure mode this port
already had a defect for.

**`cycle_model` stays inside one provider.** Crossing providers mid-cycle would park the user
on a model with no credentials, and the run would fail on the *next* request rather than here.

**pi's `provider` + `modelId` pair is rejoined into one reference** rather than resolved
through a second lookup path. Epsilon's catalog keys on `provider/model`; a second resolver
that could disagree with the first is the kind of thing that makes two entry points behave
differently and nobody can say why.

**`get_last_assistant_text` returns `null`, not `""`,** when there is no text. A client
deciding whether the agent spoke cannot otherwise tell "no text" from "text that happened to
be empty".

## The control, and what it found

This was the fifth time this session a negative control found a vacuous test, and the second
time it found one in code I had just written.

**The first version of this slice had no test of `set_model` succeeding.**

```
control: remove `agent.set_model_id(...)` from the handler
result:  all 25 RPC tests PASSED
```

Every `set_model` test I had written asserted a *failure*: an unresolvable name, a missing
model, a bare parse. A handler that refuses everything correctly passes all of them. I had
shipped a feature with no test of it working.

### The first correction was also wrong

The new test asserted the agent starts somewhere other than the target model, and it failed on
**that precondition** — the default already is `faux-1`. Setting a model to the value it
already holds passes against a handler that does nothing, which is precisely the control.

The rule that came out of it: **the precondition a test depends on must be the thing the
control breaks.** With the agent parked on `m186-starting-model` first:

```
control -> 2 FAILED
  "core must actually hold the new model -- a success reply is not evidence"
  left: "m186-starting-model"  right: "faux-1"
real    -> 29 passed
```

## Two pre-existing tests were asserting something untrue

The cost of implementing an op that a test had claimed was unknown. Both tests *passed* while
describing the opposite of the code:

| Test | Was using | Now uses |
|---|---|---|
| `ops_deserialize_with_snake_tags` | `set_model` as the example of the `Unknown` fallback | `set_auto_compaction` (still genuinely unimplemented) |
| `unknown_op_reports_failure_with_the_echoed_id` | `set_model`, `cycle_model`, a pi-shaped `modelId` frame | `get_tree`, `export_html`, `fork` |

The second test's example set is now small and shrinking, which is the intended friction: the
next person to implement one of those has to update the test, and cannot leave a passing
assertion behind that describes the opposite of the code.

## Gotchas worth keeping

- **Eight of my own assertions read the payload at the top level.** `ok_line` puts it under
  *both* `data` and `ok`; `command`/`success`/`id` are top level. All eight read
  `reply["x"]` and got `Null`.
- **A regex repair pass ate the `"]["` separator** and produced `reply["datareply["x"]`. An
  index expression is not regex-shaped text.
- **A slice-based edit removed the file's closing brace**, leaving an unclosed `mod tests`.
  Recovered by restoring the tail verbatim rather than reverting the file, which would have
  cost 495 lines of work.

## Verification

- `epsilon-cli --lib`: 94 passed (29 in `rpc_mode`)
- Battery `m186`: **1,425 results, 0 failed** across 63 targets — exactly +14 over m185, all
  in `epsilon-cli --lib` (78 → 92), every other target identical in count
- Battery `m187`: **1,427 results, 0 failed** — the two history-integrity tests

## What nothing exercises

- **No real editor client was driven over stdio.** `dispatch_line` is tested in-process, and
  that is where every decision lives, but the stdio loop itself — line reader, writer, flush on
  a partial line — is only covered by the pre-existing loop tests. `get_available_models` with
  no provider filter returns hundreds of rows, which is the case most likely to expose a
  line-splitting bug, and it was not driven through a pipe.
- **22 of pi's 33 commands are still unimplemented**, including `bash`, `fork`, `get_tree`,
  `switch_session` and `compact`. Named individually in `audit/02-port-fidelity.md`.
- **`set_model` was verified only against `faux-1`**, the one catalog row that needs no
  credentials. The credential-refusal branch never executed, because no test in the workspace
  sets a provider key — and a test that depended on one would be env-gated, not deterministic.
- **`get_last_assistant_text` was verified with a single text block.** A message with several
  blocks, or with thinking before the text, is not covered.
