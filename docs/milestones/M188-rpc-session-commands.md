# M188 — Four more RPC commands, and a probe that found a wire name pi documents and epsilon refused

## What

`clear_queue`, `get_messages`, `get_commands`, `get_session_stats`. The first three were wiring.
The fourth was not, and finding out why is most of this milestone.

## The real blocker behind `get_session_stats`

The numbers a session's stats need existed — but only in the TUI's `UiState`, which is a
**view**. `RpcMode` has no `UiState`, so it could not read the session total in principle, not
merely "had not been wired yet".

The run loop's own `usage_total` is deliberately run-scoped: CORE-02 keeps a run's turn
context, model and transcript out of the session, and folding a run's spend into a session
field is not that, but it is adjacent enough to be worth asking about rather than assuming.

**Decision: a fact about a session belongs on `Agent`, not in a view that happens to be on
screen.** Two accessors, added in `epsilon-core`:

| Accessor | What it is for |
|---|---|
| `session_usage()` | The flat total, folded once per completed run |
| `session_usage_by_model()` | The same usage split by the model that produced it |

### Why the per-model split is load-bearing

Price is a property of the model that consumed the tokens. Pricing a flat total at the
*current* model under-bills every response a previous model produced — and the error is
silent, because the total is still non-zero and looks plausible.

The control proved the shape of the failure rather than just the fact of it:

```
control: no per-model split
reply:  {"inputTokens":100,"outputTokens":50,"totalCostUsd":null,"models":[]}
```

**Tokens without a bill.** A client that trusts `totalCostUsd: null` as "free" rather than
"unpriceable" reports a session that spent real money as costing nothing. That is why the
reply carries `costIsFloor` as well as the figure, and why an unrun session reports `null`
rather than `$0.00` — a session that never called a provider has no bill, and `$0.00` is a
statement about a request that never happened.

`UiState` has `usage_by_model` for the same reason, arrived at independently. That is
reassuring rather than suspicious: the TUI's footer and this reply now read the same core
accessor through the same `breakdown_per_model`, so they cannot disagree.

## The wire-name defect, found by a probe printing its own reply

I wrote `r#"{"op":"follow_up",...}"#` because that is what pi documents
(`rpc-types.ts:30`). A test I had just added failed its precondition, and the probe I added
to explain it printed:

```
follow reply: {"command":"follow_up","error":"unknown op: follow_up","success":false}
```

**Epsilon accepted only `followup`.** Serde's `rename_all = "snake_case"` turns the variant
`Followup` into `followup`, and a pi-shaped client sending the documented name was refused.
This is a pre-M186 defect: `follow_up` has been on the wire-name list in the audit since the
RPC surface was written, and nothing caught it because

**the existing test read the variant name, not the wire name.** `ops_deserialize_with_snake_tags`
deserialized the enum and compared what serde produced against the variant list — so it agreed
with the code by construction, and could not notice a name no client would send.

Two tests now close it: `pi_spelled_op_names_reach_their_handlers` dispatches the exact
strings from `rpc-types.ts:27-31` and asserts on the reply's `success`, and
`the_legacy_followup_spelling_still_reaches_its_handler` keeps the old spelling working.

**The rule, and it is the general one: a protocol test must go through the wire.** Deserializing
a type proves the type parses. Only dispatching a frame and reading the reply proves the
protocol accepts what a client will send.

### `alias` alone was not enough

```rust
#[serde(alias = "followup")]  // ignored
#[serde(rename = "follow_up", alias = "followup")]  // works
```

`#[serde(other)] Unknown` on the same enum means serde never reaches the alias check for a
name it does not recognise — the catch-all absorbs it first. The explicit `rename` is what
makes pi's name the variant's own name; the `alias` is then the legacy spelling. Worth
recording because the first attempt looked correct and silently did nothing.

## A test that could only pass, and I could only find out by measuring

`clear_queue_empties_both_queues_and_reports_both_counts` first ran against a run in flight,
timed out after 10s, and I assumed the drain had broken the run. It had not. A probe printing
the queue lengths showed:

```
after prompt: running=true  steer=0 followup=0
after follow: running=true  steer=1 followup=0
```

**The follow-up was already gone** — consumed at a run-loop drain point before the next
statement. A run in flight drains both queues at its own pace, so testing against one is a
race that passes when the scheduler is kind. The test now uses an **idle** agent, which is
both deterministic and the state where `clear_queue` is most useful: work queued that is not
about to be consumed.

## Key decisions

**`clear_queue` returns both counts, and both queues are drained.** pi's payload is
`{ steering, followUp }` (`rpc-types.ts:59`). Draining only steering would leave every
follow-up queued while reporting a clear — and follow-ups are the messages that run *after*
the agent would otherwise stop, so the user watches the run continue with no way to see why.
This is the same shape as the TUI defect where steering was restored to the editor and
follow-ups had nowhere to go.

**`get_commands` reads `picker::SLASH_ITEMS`, not a second table.** That constant's own doc
comment already says it is "pi's command registry shape". A hand-kept list in `rpc_mode.rs`
would drift, and a client building a palette would then offer commands the harness does not
accept. The test asserts against the same constant the handler reads, so a drift between them
is a test failure rather than a divergence nobody notices.

**`!define` is reported as `defined: false`, not omitted.** pi appends it
(`rpc-types.ts:141`) and it writes new slash commands into a config file. Epsilon has no
equivalent. A palette that hides it looks broken; one that lists it without the flag would
have a client write a file nothing reads.

**`sourceInfo.line` is 0 and `endLine` is null.** pi carries a real line range so an editor
can jump to the definition. Epsilon parses frontmatter rather than indexing it, so there is
no line to report. A client that jumps to a wrong line is worse than one that knows it cannot.

## Verification

Three controls, each failing on the defect it names:

| Control | Result |
|---|---|
| `clear_queue` drains steering only (the old behaviour) | FAILED — `followUp: 0` with one queued |
| no per-model split (flat total) | FAILED — `totalCostUsd: null, models: []` with tokens > 0 |
| the original `followup`-only wire name | FAILED — `unknown op: follow_up` |

- `epsilon-cli --lib`: **105 passed** (94 + 11)
- `epsilon-cli` rpc-focused: **40 passed** (30 + 10)
- Battery `m188`: see `audit/evidence/gate-m188.md`

## What nothing exercises

- **`set_model` to a credentialed model was still not run** — this machine has no provider
  keys, so `faux-1` is the only selectable row and the credential-refusal branch is only
  exercised by reading the code.
- **No real editor client was driven over stdio for these four.** `dispatch_line` is
  in-process. The M186 stdio probe covered the six added there; these four have not been
  probed through a pipe, and `get_messages` in particular can return a large array that the
  line writer has never had to frame.
- **No session that switched models was measured end to end.** The per-model split is
  asserted through one model's worth of rows. A two-model session is the case the split
  exists for, and the assertion that would catch a regression is the one I have not written.
- **`get_commands` is not compared against what the TUI would accept**, only against the
  constant both read. A command in `SLASH_ITEMS` that `submit_editor` rejects is a real
  defect and this milestone does not look for it.
- **Cache figures are not asserted.** `inputTokens`/`outputTokens` are checked against core;
  the cache columns are carried but unverified, because the faux provider does not bill cache
  traffic.
