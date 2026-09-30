# M229 — a subagent's tokens are stored, persisted, and shown nowhere

**Ledger row A11, verified rather than believed.** The row's headline — "a reader and no writer" —
is half stale and half one of the more serious money gaps this repository has open. No production code
changed; what this round did is find out which half is which, and where the fix goes.

## The compaction half was fixed at M145

> "`usage_accounts()` and `add_usage_accounts` exist and are read by `app/session.rs:227`; nothing
> constructs the entry."

Something does construct it. `core/src/lib.rs:2775` appends the summarizer's bill **on the entry that
incurred it**:

```rust
st.append(EntryKind::Compaction {
    summary, tokens_before, retained_tail, details,
    // M145/F22: the summarizer's real bill, on the entry that incurred it. The
    // response has been in scope here all along and the usage was discarded, so
    // a genuine provider call was free in every number the user could see.
    …
```

and `store.rs:1513` reads it back under the reason `compaction`, which is what the cost panel shows.
One entry rather than two, because `pop_last_message` pops the last entry to discard a failed turn
and a separate accounting row would have separated a summary from its cost.

## The delegated half is live, and it is the one that matters

A `task` child's tokens are carried in three places and aggregated in none:

| stage | where | what happens to the number |
|---|---|---|
| the child finishes | `core/src/lib.rs:3954` (`entry.usage = Some(outcome.usage)`) | stored on the task entry |
| the tool returns | `tools/src/task.rs:231` → `exec.rs:839` → `ToolResultMessage.usage` | **persisted to the transcript** |
| the provider stream | `core/src/lib.rs:3098` | sums **only assistant messages** — a tool's usage is not in `usage_total` |
| the turn's total | `core/src/lib.rs:3533` | `last_assistant.map(\|a\| a.usage)` — the last assistant message, again |
| the by-model cost map | fed from the same place as above | a subagent never enters it |
| `/task list` | `app/commands.rs:1792` | prints id, state, model, description, turns, preview — **no usage** |

So the money is durable and invisible: it is in the session file, and in no number the user can read.
Unbounded, because nothing bounds how many subagents a session may start.

**No test asserts any of it** — `grep` for a test that pairs `task`/`subagent` with `usage` finds none,
which is consistent with a gap nobody had measured.

## The design question the old row called a blocker, answered

The row said *"shipping the writer is a design decision (which model is a summarization billed to?),
not a patch."* That question is real for **summarization** — and irrelevant for **subagents**: the
child runs on a model the task tool already knows (`ran_on`, surfaced as `details.model` at
`task.rs:225`), so the bill has a model to be filed under, and it is also already in the task entry
(`s.model`) that `/task list` prints beside.

So the blocker is not a decision. It is three insertions:

1. fold `out.usage` into `session_usage_by_model` under the model `details.model` names, so the cost
   panel can show it (and so a scout on a cheap model is distinguishable from one on an expensive
   one, which is the reason `/task list` already prints the model);
2. add it to the **turn** total — that one needs the loop's local `usage_total`, which `exec.rs`
   cannot reach, so it is a change in the loop rather than at the tool boundary;
3. print it in the `/task list` row.

**Why nothing was changed this round, stated plainly:** (1) and (3) are small and (2) is not, and a
fix that lands in the cost panel while the header keeps under-reporting is *worse* than a documented
gap — two numbers that disagree about the same tokens is precisely the class this repository treats as
a defect. The sequence has to be one slice, with the by-model map, the turn total and the task row
moving together, and a test that a completed task's tokens appear in all three.

## Verification

Docs-only: `check-links.py` PASS. No battery is claimed, because no code changed. Every line cited
above is a place a reader can check in one look, and the two claims that are *not* measured — how
often a session actually starts subagents, and what share of spend they represent — are named as the
first thing the fixing slice should measure, because a money defect whose size is unknown is still a
money defect.

## Limits

- **This is a reading of the code, not an observation of the product.** I did not run a session that
  spawns a subagent and compare the header against the provider bill, which is the measurement that
  would put a number on it.
- **The compaction half is verified by construction** (the writer at `lib.rs:2775`, the reader at
  `store.rs:1513`) and by the tests those two lines' doc comments cite; I did not re-run them this
  round.
- **`/task list` may have another surface for usage** that this reading missed — `commands.rs:1792`
  is the row format, and the numbers it interpolates are id, state, description, turns and a preview.
