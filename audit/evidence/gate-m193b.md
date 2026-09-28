# Battery `m193b` — GATE PASS (D29 + session-naming)

**Source:** `6a19af1` (D29) + `1b79f8a` (session naming) + `5f49cf1` (doc-block splice).
**Date:** 2026-09-28. **Log:** `target/gate-m193b.log`.

## Result

| Stage | Exit | Time |
|---|---|---|
| `fmt --check` | 0 | 3 s |
| `clippy --workspace --all-targets -- -D warnings` | 0 | 78 s |
| `test --workspace` | 0 | 452 s |
| `demo` | 0 | 43 s |
| `check -p epsilon-cli --features real` | 0 | 14 s |

**63 targets · 1,461 results · 0 failed.**

`epsilon-cli --lib` **128** (was 125 at M192B — **+3**: the D29 pair and the naming
comparison). `epsilon-session --lib` 26, unchanged, as expected: `new_session_path` adds no
test of its own because the naming property belongs to the RPC path where the drift was.

## Progression

| Battery | Total | `epsilon-cli --lib` |
|---|---|---|
| M190B | 1,447 | 114 |
| M191 | 1,449 | 116 |
| M192B | 1,458 | 125 |
| **M193B** | **1,461** | **128** |

## What this battery is really about

**1,458 tests passed in M192B while `get_entries` was serving the wrong wire shape.** Not
one of them could have failed: they compared the handler against its own output. D29 is
recorded in full in `audit/03-defects.md`; the installed-wire measurement is in
`audit/evidence/installed-binary-2026-09-28-m192-m193.md`.

So the three extra tests here are not "coverage added" in the usual sense. They are the
first assertions in this area whose expected value does not come from the implementation:

| Test | Expected value comes from |
|---|---|
| `get_entries_and_get_tree_agree_on_every_entries_shape` | **The other handler** — one reference type cannot be served two ways |
| `every_get_entries_entry_has_pis_four_fields` | `session-manager.ts` — and asserts the four internal names are *absent* |
| `a_fork_and_a_new_session_are_named_by_the_same_convention` | **The other op's name** — one directory cannot hold two conventions |

## Controls

| Control | Mutation | Result |
|---|---|---|
| **G** | `rebind_to` writes nothing — fork file appears, agent never rebound | **3 failed** |
| **H** | mirror swapped, conversation never re-projected | **2 failed** (first attempt was inert — see below) |
| **I** | `get_entries` restored to raw `SessionEntry` structs | **2 failed**, printing `left: [id, kind, parent_id, seq, timestamp_ms]` / `right: [id, parentId, timestamp, type]` |

### Control H's first attempt was not a control

```rust
if false {
    agent.reproject_current_branch()?;
}
```

added **above** the live call rather than deleting it. The re-projection still ran, and the
control passed all 59 tests while nominally disabling the statement it was written to
disable. Re-armed by deletion it fails 2.

The mechanical guard: after arming a control, assert the disabled construct is **absent
from the file** — `rpc_mode.rs` contains zero `reproject_current_branch()?` — not merely
unreachable. A control that greps for its own mutation proves the mutation is present; one
that greps for the absence of what it disabled proves the control is off.

This is the third control in the project to pass for a reason unrelated to its mutation
(M189's billing, M189's tree, this one). All three are in
`agents/memory/01-plan-execution.md`.

## The failed first attempt

`m193` failed clippy in **2 seconds**:

```
error: missing documentation for a method
   --> crates\epsilon-session\src\store.rs:1513:5
    |
1513 | /     pub fn fork_branch_at(
```

The method was fine. I had inserted `new_session_path` **between `fork_branch_at`'s doc
block and its `pub fn` line**, so the documentation now described a different function and
the target had none. The insert was anchored to a byte offset — the third file damaged that
way this session, after a doc-comment injection and an unbalanced-brace splice.

Its other stages had passed with the same **63 targets, 1,461 results** the passing battery
later reported, which is a useful coincidence: the test total could not distinguish the two
runs, and only clippy could.

## Installation

Refreshed after M192 (`3baff78`, 21,486,080 bytes) and probed — that probe is what found
D29. **The installed binary does not yet contain the M193 fixes**; it must be reinstalled
before the D29 and naming claims can be said to hold on the shipped artifact rather than
only in the source and the suite.
