# M70 — message sanitization: what the model actually sent

**Scope:** candidate 18 from the Hermes mining report (`agent/message_sanitization.py:195,296,551,714`) — a candidate the consolidated plan had not carried, so it was added to the plan as
**item 17** and ported in the same pass. (Tier 2 was already complete; this was next by leverage,
not by plan order.)

## What landed

Two failure classes that every harness with eight provider wires meets eventually, both of which
Epsilon handled by *discarding* the model's intent:

- **Tool-call arguments that are not valid JSON.** `providers::finish` did
  `serde_json::from_str(buf).unwrap_or_else(|_| json!({}))`. A model emitting `{"path": "a.rs",}`
  or a payload cut off by the output limit therefore invoked the tool with **no arguments** — and
  the error the model read was "missing field `path`", which describes the symptom and hides the
  cause. `epsilon_llm::sanitize::parse_tool_arguments` now recovers, in order: a JSON string that
  contains JSON; a trailing comma before a closer; a payload cut mid-string or mid-object (the
  unterminated tail and any half-written member are dropped, then brackets are closed); and
  single-quoted JSON when no double quote is present to make that ambiguous. Each repair reports
  which fix it applied. What cannot be recovered is handed on **verbatim**, so the tool failure
  quotes what the model actually sent.
- **Duplicate tool-call ids in one message.** `unique_call_ids` renames later duplicates
  deterministically (`<id>#2`), preserving the first occurrence — providers reject duplicates, and
  the result pairing was otherwise ambiguous: two calls, one id, two results, no way to say which
  answered which. The rename happens in the same assembly step, so the stored transcript carries
  unique ids and the loop's result pairing stays exact.

Two supporting changes:

- `ToolDescriptor::basic_validate` now names what arrived, bounded to 200 characters with the full
  length appended. "arguments must be a JSON object" told the model what was expected; it did not
  tell it what it had sent.
- **The interrupted-sequence repair moved into the live loop.** `epsilon_session::repair::
  repair_on_open` already closed dangling calls when a *file* was reopened. The same shape in a live
  process — an abort or a crash between the assistant's tool-call block and its results — was sent
  as-is on the next request, and providers validate tool-call pairing, so the 400 arrived one request
  after the mistake. `Agent::close_dangling_tool_calls` now runs at the top of every `run()`, gives
  each unanswered call an error result that says it was never executed, and emits a `Notice` so the
  closure is visible rather than silent.

## Key decisions

- **Repair syntax, never semantics.** No field is invented, no call is reordered, no intent is
  guessed. The one place where "repair" could have become "invention" — closing `{"path":` into
  `{"path": null}` — is refused by test.
- **A repair that recovers nothing is refused.** Truncation recovery can legitimately reach `{}`
  when the cut landed before anything readable. Returning that as a success would be the old
  silent-`{}` behaviour wearing a repair's clothes, so it is reported as unreadable instead. The
  test asserting this was written after the implementation produced exactly that result.
- **The empty payload stays valid.** `""` and `"{}"` are legitimate no-argument calls, not failures:
  a tool that takes no arguments must not be told its arguments were unreadable.
- **Loud where it matters, quiet where it does not.** Unreadable arguments are visible (the tool
  result quotes them); a successful repair is not announced, because the repair either produced the
  arguments the model meant (and the tool worked) or produced something invalid (and validation
  fails loudly). Hermes logs full arguments because it has a logging surface; Epsilon reports through
  the tool result, which is the reader that matters.
- **Reasoning echo needs no new policy.** The mining note asked for a per-family echo policy;
  Anthropic and Bedrock already strip thinking blocks on the way out and the openai-compat and
  Google wires never replay them. Verified rather than re-implemented (`grep -n "Thinking"
  crates/epsilon-llm/src/providers/*.rs`).

## Test-caught bugs

- **The unwrap branch was unreachable.** A JSON string *is* valid JSON, so the generic parse
  succeeded first and the double-encoded case was reported as "arguments must be an object" — the
  precise repair it needed never ran. The string case is now checked before the generic one.
- **A cut landing mid-key left a dangling colon.** `{"a": 1, "b":` truncates to `{"a": 1, "b":`,
  which does not close into valid JSON by adding brackets. `drop_incomplete_member` removes the
  half-written member — and only that member.
- **The truncation repair produced `{}`** for a payload whose every member was incomplete, which is
  the bug this milestone exists to remove. Caught by the ambiguity test, fixed by refusing empty
  recoveries (above).
- The healthy-transcript scenario initially asserted *message counts*, so the run's own assistant
  reply failed it. Rewritten to count tool results — the thing under test.

## Verification

Mutation-tested, one mechanism at a time, each reverting to green after:

| Mutation | Result |
|---|---|
| arguments fall back to `{}` (the old behaviour) | 2 wire fixtures fail |
| `unique_call_ids` disabled | the duplicate-id fixture fails |
| `close_dangling_tool_calls` disabled | 2 of 3 sequence scenarios fail, quoting the unanswered ids |
| empty recovery allowed | 2 sanitize tests fail |

New tests: 10 in `epsilon-llm/src/sanitize.rs`, 3 in `tests/wire_fixtures.rs` (through the real wire
assembly, not the unit API), 3 in `epsilon-core/tests/sequence_repair.rs` (through the real loop), 4
in `epsilon-contracts/src/tool.rs`.

Gate: `scripts/gate.sh m70` — fmt clean, clippy zero warnings (with `-D warnings`), **637 tests, 0
failed**, demo completes, `real`-feature build compiles.

## Port-plan status

Tier 1 and Tier 2 are complete. This pass added Tier 3's **item 17**; the rest of Tier 3 (edit-matcher
stages, path-conflict planning, subdirectory hints, `@`-reference expansion, typed corruption reasons)
and Tier 5 (LSP diagnostics, workspace checkpoints, prompt templates, session stats, persistent PTY)
remain — LSP and PTY are *L*-sized design questions, not slices.
