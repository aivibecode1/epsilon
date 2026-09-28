# Battery `m192b` — GATE PASS

**Source:** `3baff78` (M192 source) + `a6ab1bf` (the clippy fix below).
**Date:** 2026-09-28. **Log:** `target/gate-m192b.log`.

## Result

| Stage | Exit | Time |
|---|---|---|
| `fmt --check` | 0 | 2 s |
| `clippy --workspace --all-targets -- -D warnings` | 0 | 46 s |
| `test --workspace` | 0 | 268 s |
| `demo` | 0 | 6 s |
| `check -p epsilon-cli --features real` | 0 | 1 s |

**63 targets · 1,458 results · 0 failed.**

Per-crate lib suites read from the log: `epsilon-cli` **125**, `epsilon-core` 150,
`epsilon-session` 26.

`epsilon-cli --lib` was 116 at M191 and is 125 here — **+9**, which is the nine new tests
and nothing else. Recorded as a per-binary delta rather than inferred by subtracting
totals, which is the practice the earlier milestones established.

## Progression

| Battery | Total |
|---|---|
| M188B | 1,438 |
| M189B | 1,446 |
| M190B | 1,447 |
| M191 | 1,449 |
| **M192B** | **1,458** |

## The failed first attempt

`m192` failed clippy on one line:

```
error: unused variable: `dir`
   --> crates\epsilon-cli\src\rpc_mode.rs:3268:21
    |
3268 |         let (agent, dir) = agent_with_two_runs().await;
    |                     ^^^ help: if this is intentional, prefix it with an underscore
```

`dir` came from destructuring the helper's return and was never read — the test asserts on
paths obtained from the store, not on the directory the helper was handed.

The same run's other stages had already passed: **63 targets, 1,458 results, 0 failed**.
So the defect was invisible to the test suite and to `cargo test`; only `-D warnings` caught
it. Worth recording because it is the ordinary case — a lint is a different reviewer than a
test, and the gate runs both.

Fixed in `a6ab1bf` as `_dir`, and `m192b` re-ran on the fixed bytes.

The raw log of the failed run is `target/gate-m192.log` (untracked by design —
`.gitignore:42` excludes `audit/evidence/*.log`, because the `.md` beside it is the record
meant to be read). It was not preserved under an `audit/evidence/*.log` name; that path
would have been silently untracked, which is why the summary lives here instead.

## Controls, and what they proved

Both controls ran against the M192 source; the log for each is the focused `rpc` suite, not
a full battery, because a control's job is to show the mutation changes behaviour.

| Control | Mutation | Result |
|---|---|---|
| **G** | `rebind_to` writes nothing and returns `Ok(())` — the fork file appears, the agent is never rebound | **3 failed**: *"the agent must point at a new file"*, *"clone must produce a new file"*, and the round trip |
| **H** | `rebind_to` swaps the mirror but never re-projects | **2 failed**: *"and the agent must see it as empty — left: 4, right: 0"*, *"the agent must carry NO messages from the previous session — 4 left over"* |

### Control H took two attempts, and the first was not a control

The first attempt at H added

```rust
if false {
    agent.reproject_current_branch()?;
}
```

**above** the live call rather than deleting it. The re-projection therefore still ran, and
H passed all 59 tests while nominally disabling the one statement it was written to
disable. Re-armed by deletion it fails 2.

The guard worth keeping: after arming a control, assert the disabled construct is
**absent from the file** (`rpc_mode.rs` contains zero occurrences of
`reproject_current_branch()?`), not merely unreachable. A control that greps for its own
mutation proves the mutation is present; one that greps for the *absence* of what it
disabled proves the control is off. This is now in
`agents/memory/01-plan-execution.md` alongside the M189 billing and tree controls — the
third control in this project to pass for a reason unrelated to its mutation.

### The gap H exposed is now covered

Every pre-existing M192 test checked **file paths and entry counts**, all of which a stale
conversation satisfies. The defect H made reachable: a session file that says "empty" while
the agent carries the previous session's messages into the next provider call.
`new_session_empties_the_conversation_not_just_the_file` was added for it, and asserts the
store and the agent's own history independently, because either alone is satisfiable by
accident.

While writing it, one assertion was replaced as vacuous:
`texts.iter().any(|t| t.contains("hello") || !t.is_empty())`. `!t.is_empty()` holds of every
message that exists, so the disjunct asserts only that the run produced *something* — it
passes whether or not the conversation is right.

## Parity figure

`23 of 33` pi client commands, derived by matching pi's 33-member `RpcCommand` union against
the `Op` enum **variant by variant**, resolving every wire name through serde rather than
through the Rust identifier.

**The inherited "18 of 33" was itself wrong by one.** `follow_up`'s primary name lives in
`#[serde(rename = "follow_up", alias = "followup")]`, so the variant is `Followup` and the
string `follow_up` never appears as an identifier — the same blindness a marker search has
to serde attributes, recorded under "Protocol surfaces" in `AGENTS.md`. Ten remain:

```
set_steering_mode  set_follow_up_mode  compact        set_auto_compaction
set_auto_retry     abort_retry         bash           abort_bash
export_html        get_fork_messages
```

Of those ten, **two are wiring** — `LoopLimits` already carries `steering_mode` and
`followup_mode`, and `PendingQueue::drain` already honours `QueueMode::All` versus
`OneAtATime`; what is missing is a live setter. The other eight need capability that does
not exist in core yet: `compact`, `set_auto_compaction`, `set_auto_retry`, `abort_retry`,
`bash`, `abort_bash`, `export_html`, `get_fork_messages`. An RPC op that cannot refuse is
worse than an absent one, so those are not being added as acknowledgements.

## Installation

Not yet installed at the time of writing. `m192` changes RPC behaviour that an editor
client exercises and nothing in-tree can reach, so the release must be refreshed before
this evidence is final.
