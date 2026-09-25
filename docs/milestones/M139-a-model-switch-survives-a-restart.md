# M139 — a model switch survives a restart (port fidelity: `getSessionContextSettings`)

**Status:** design accepted, not yet implemented — this file states the gap and the plan
**Port target:** `references/pi-0.85.1/packages/coding-agent/src/core/session-manager.ts:362-374`
**Closes:** the "model switch does nothing on resume" item in `agents/memory/next-hardening.md`

## The gap, re-derived from source on 2026-09-25

`model_change` and `thinking_level_change` entries are decoded, typed, and persisted
correctly (`crates/epsilon-session/src/store.rs:1954-1965`). They are durable and
faithful on the write side.

Nothing ever reads them back. `ModelChange` appears in exactly three places in the whole
workspace, none of them a consumer:

| Site | What it is |
| --- | --- |
| `store.rs:1954` | the decoder that creates it |
| `search.rs:333` | a search index entry |
| `tests/session_store.rs:1037`, `:1090` | tests that assert the decode |

`ModelViewEntry` (`store.rs:19-42`) has exactly three variants — `Message`, `Summary`,
`BranchSummary` — and **none carries a model or a thinking level**. So a resumed
conversation comes back on whatever model the launch flag or config names, regardless of
what the user chose in the previous session. The switch is faithfully recorded and
faithfully ignored.

## What pi does, and the part that is easy to miss

`getSessionContextSettings` (session-manager.ts:362-374) walks the selected path and
tracks two things:

```ts
for (const entry of path) {
    if (entry.type === "thinking_level_change")  thinkingLevel = entry.thinkingLevel;
    else if (entry.type === "model_change")      model = { provider, modelId };
    else if (entry.type === "message" && entry.message.role === "assistant")
        model = { provider: entry.message.provider, modelId: entry.message.model };
}
```

Two behaviours, and the second is the one a naive port misses:

1. **Last change wins.** The loop has no break; the final `model_change` on the branch is
   the one that applies. Only entries on the selected path count, so switching models and
   then branching back restores the earlier model — the answer is a property of the
   path, not of the file.
2. **An assistant message also sets the model.** This is not a detail. It means a session
   with **no** `model_change` entry at all still recovers its model from the last
   assistant turn. A port that only reads `model_change` produces `None` for every
   session that predates the entry type, or that was written by a build which never
   emitted one — and then falls back to the launch default, which is the current bug
   wearing a different hat.

A third property worth stating because it is easy to get backwards: the function returns
`model: null` when the path has neither. That `null` is meaningful — it means "this
session never ran", and the caller keeps its own default. It is not "use a blank
model".

## The plan

**`SessionContextSettings { model: Option<ModelRef>, thinking_level: Option<String> }`**
returned by a new `SessionStore::context_settings(&self)`:

- Walks `branch_entries()` — the *selected* path, so a branch restores its own model.
- `ModelChange` sets the model; an `Assistant` message sets it too (pi's second rule).
- `ThinkingLevelChange` sets the level, defaulting to the pi default of `"off"` when the
  path says nothing, because that is what pi returns and the caller must be able to tell
  "off" from "never set".
- Last one on the path wins; no break.

**The consumer.** `epsilon-cli`'s resume path (`main.rs`, the `SessionStore::open` call
site) reads the settings after repair and applies them, *before* the agent is built, and
only when the user did not pass an explicit `--model` on this invocation. That precedence
is not a detail either: a flag is a deliberate instruction for *this* run, and a resumed
session's setting is a default. Flag > session > config.

**Honest scope limit.** `--thinking-level` is a separate concern from the model, and
whether the TUI can change thinking level mid-session is a larger question. This slice
restores both values on resume because the same 20-line walk produces them; it does not
add a mid-session thinking-level UI, which is recorded as not-done rather than implied.

## The consumer

`epsilon-cli`'s resume path applies the result immediately after
`restore_history`, and only when the user passed no `--model` on this invocation.

The precedence is the point: **an explicit `--model` is a deliberate instruction for
this run, while the session's model is a remembered default.** So the order is
flag > session > project/config — the same order `candidate_raw` already builds, with
the session slotting in only when the user expressed no preference. Getting this
backwards would be worse than the original bug, because a user who explicitly names a
model and is silently overridden has no way to notice.

The resolved model is run through `resolve_model_for_run` first, so a session that
recorded a partial or aliased id still lands on a real catalog row rather than failing
at the provider. The fallback is the raw id, which is what pi does for a custom model.

The change is announced on stderr:

    resumed session was last used on anthropic/claude-sonnet-4; continuing there (pass --model to override)

Silently switching would be a different bug: the user asks "why is it using that model",
and an answer that is not printed is an answer nobody gets. The line also carries the
override hint, because the one time this is *wrong* is when the user wanted the default —
and they need to know the flag exists.

## Verification

Battery **`m139b`**, log at `audit/evidence/gate-m139b.log`; negative control at
`audit/evidence/m139-negative-control.txt`.

**The first round (`m139`) failed and is not cited as evidence.** It caught two things
worth recording, because both are mistakes that look fine in a diff:

1. **`dead_code` on the test helper** — the three CLI tests had been appended to the end
   of `main.rs` *after* the last `}` that closes the `#[cfg(test)] mod print_outcome_tests`
   module, so they sat at module scope and were being compiled into the **release
   binary**. They passed, which is exactly why this was easy to miss: a test outside a
   test module is still a test, and the compiler only complains that the helper is
   unused. Tests belong inside the module, not at the bottom of the file.
2. **`collapsible_if` in `store.rs`** — the assistant-message arm was written as
   `EntryKind::Message { message, .. } => if let Message::Assistant(a) = message`,
   which the nested `if let` is really a match pattern. The `let` chain also reads as an
   *early* exit, when it is a *filter*: the arm must fall through to `_ => {}` for
   user and tool messages. Collapsing it into
   `EntryKind::Message { message: Message::Assistant(a), .. } => ...` states the
   condition in the pattern and leaves the fallthrough obvious.


**Result: `GATE PASS`. 61 test binaries, 1,204 passed, 0 failed, 1 ignored** — up from
1,196 at M138, and the delta is exactly the 8 new tests.

Eight tests, all of them written before the consumer and all of them about a rule a
plausible port gets wrong:

In `crates/epsilon-session/tests/session_store.rs`:
1. `the_last_model_change_on_the_path_wins` — two switches, second wins.
2. `an_assistant_message_alone_names_the_model` — no `model_change` anywhere.
3. `a_path_with_no_model_at_all_reports_none_not_a_default` — `None`, not a default.
4. `the_thinking_level_defaults_to_off_and_a_later_entry_wins` — pi's `"off"`.
5. `the_settings_survive_a_close_and_reopen` — the defect is a *restart* defect.

In `crates/epsilon-cli/src/main.rs`:
6. `an_explicit_model_is_never_overridden_by_the_session`
7. `the_sessions_model_takes_over_when_no_flag_was_given`
8. `a_matching_model_is_left_alone`

**The negative control** empties the `EntryKind::Message` arm — pi's third rule — and
`an_assistant_message_alone_names_the_model` fails with `left: None`. The two
`model_change`-based tests still pass, which is the point: the control isolates the one
rule a `model_change`-only implementation drops, rather than breaking everything and
proving nothing.

`resumed_model_to_use` was extracted from `build_runtime` specifically so tests 6–8
exercise the production rule. The first draft of test 6 was a closure re-implementing
the precedence inside the test — that would have tested the copy while the product ran
something else, which is worse than no test at all. The extraction is the honest fix.

## Limits — what this slice does not do

- **The TUI has no model picker wired to this.** The values are restored; changing model
  mid-session from the TUI is a separate feature and is not implied by it.
- **`thinking_level` is restored on resume but not settable.** The walk produces it
  because it is the same loop, and `SessionContextSettings` exposes it, but nothing
  applies it to the agent yet. Recorded rather than quietly implied.
- **Only the CLI resume path consumes it.** `serve` mode and the RPC loop go through the
  same `build_runtime`, so they inherit the behaviour; that is by construction, not by
  test, and is listed under "What nothing exercises".
- **A session whose model no longer exists in the catalog** falls back to the raw id,
  which pi also does for custom models. If the id is stale *and* unknown, the failure
  appears at the provider rather than at open. Unchanged from pi.

## What nothing exercises

- **`build_runtime` end-to-end.** Tests 6–8 call `resumed_model_to_use` directly. No test
  starts a real resume through `main()` and asserts the agent's model, so the wiring
  between `RunParams.model_explicit`, `SharedRun.model_explicit` and the call site is
  verified by the compiler and by reading, not by execution.
- **The announcement on stderr.** Nothing asserts that the
  `resumed session was last used on …` line is printed, only which model is chosen.
- **A branched session.** The walk uses `branch_entries()`, so a branch should restore
  its own model, but there is no test for it. It is the one property listed above that
  is argued from the code rather than demonstrated.
- **A session carrying a `model_change` written by an older build**, or one imported from
  pi with a partial `modelId`. The catalog-resolution fallback is untested.

## Why this is worth doing now

The write side is already correct and tested, so the gap is invisible to every existing
test — the entries round-trip perfectly and are then dropped on read. A defect that only
appears on *resume* after a *successful* switch is exactly the kind that survives a long
time: nothing in a normal session ever exercises it, and the failure is "the model
reverted" rather than an error, so it reads as intended behaviour.

It is also the smallest remaining port-fidelity gap with a user-visible symptom, and
the owner's brief is a *faithful* port of pi before importing ideas from elsewhere.
