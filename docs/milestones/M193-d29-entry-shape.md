# M193 — D29: the same defect, one function over

## What

Two defects that **1,458 passing tests could not see**, both found by driving the
*installed* binary over real stdio and reading raw JSON.

- **D29** — `get_entries` served epsilon's internal entry shape while `get_tree`, four
  lines away, served pi's. One reference type, two wire shapes.
- **A naming split** — a fork was `2026-09-28T10-28-30-472Z_ses_….jsonl` and a new session
  `ses_….jsonl`, in the same directory, for no visible reason.

Neither broke anything. That is what made them survive.

## Why D27 happened twice

D27, three milestones ago: `get_tree` returned a flat array where pi returns a recursive
forest, and 1,446 tests passed because **the tests asserted my shape**. The lesson was
written into `AGENTS.md`: *read the implementation, and the client that consumes the
payload, not only the type declaration.*

D29 is that defect again. `get_entries` returned raw `SessionEntry` structs — `kind`,
`parent_id`, `seq`, `timestamp_ms` — while its neighbour, serving the same
`SessionEntryBase`, returned `{type, id, parentId, timestamp}`. The M190 tests for
`get_entries` asserted **epsilon against itself**, so raw and converted agreed by
construction.

I had written the rule down and then violated it in the same area, twice, at two different
levels. So the rule is now stated in the form that actually generalises:

> **An in-process assertion compares epsilon to epsilon. Only a real frame in and JSON out
> compares epsilon to the reference.** Not a stricter test — a different kind of check.
>
> **And when two code paths serve the same reference type, the test that matters compares
> THEM to each other**, because that is the only comparison whose expected value does not
> come from the implementation.

Both sentences are in `AGENTS.md` under "Protocol surfaces", where D26's rule already
lives — the third time the same blindness has cost real work, so the rule had to go where
the next person will actually look.

## The fix, for the class

Both ops now call `entry_as_pi`. One conversion, two callers: a second shape becomes
*impossible* rather than merely absent. A test pinning the right shape would have left a
third op free to invent one.

`SessionStore::new_session_path()` owns the file-naming convention, so a second caller
cannot invent a third name.

## The tests, deliberately two kinds

| Test | Expected value comes from | Why it is not the other kind |
|---|---|---|
| `get_entries_and_get_tree_agree_on_every_entries_shape` | the *other handler* | A fixed literal would be my guess again, and a guess is what produced both defects. What cannot be wrong is that one reference type is served two ways — checkable **without knowing the right answer**. |
| `every_get_entries_entry_has_pis_four_fields` | `session-manager.ts` | Pins the four names, and asserts `parent_id`, `seq`, `timestamp_ms`, `kind` are **absent**. The defect was their *presence*, so their absence is what pins the fix. |
| `a_fork_and_a_new_session_are_named_by_the_same_convention` | the *other op's name* | The convention belongs to the store and is free to change; what cannot be right is one directory holding two conventions. |

## Control I

`get_entries` restored to raw structs — **2 failed**, printing the defect:

```
left:  [id, kind, parent_id, seq, timestamp_ms]
right: [id, parentId, timestamp, type]
```

## Control H, and the control that was not one

Recorded in full in `audit/evidence/gate-m193b.md`. The short version: my first attempt at
H added `if false { reproject_current_branch()?; }` **above** the live call instead of
deleting it, so the re-projection still ran and the control passed all 59 tests while
nominally disabling the statement it existed to disable. Re-armed by deletion it fails 2,
both reporting 4 messages carried into the "new" session.

That was the **third** control in this project to pass for a reason unrelated to its
mutation. The guard is now mechanical: after arming a control, assert the disabled
construct is **absent from the file**, not merely unreachable.

## Verified over a real wire

`audit/evidence/installed-binary-2026-09-28-m192-m193.md`.

All four M192 ops work end to end: `fork` names what it forked from and shortens the
conversation 2 entries → 1 with the tree re-deriving to a single root; `new_session` is
empty in **both** senses (0 entries, 0 messages); `switch_session` restores the original's
2 entries; `get_messages` returns a bare `ARRAY[0]`, confirming D27's fix on the shipped
artifact three milestones after it landed.

## The probe's own failures, which are part of the evidence

- It printed **`NO NEW FILE` directly beneath the fork file it had just listed** — it
  compared the post-fork directory against the *pre-run* list, which is necessarily empty
  because the first prompt creates the session. A probe reporting a false negative about
  the thing it exists to check is worse than no probe.
- An undrained **stderr pipe blocks the child** at 64 KB. The first inline version read
  stderr only after `terminate()`.
- **`--rpc` resolves its session against the PROCESS's cwd**, not the directory you
  launched from. An early probe wrote into the repository's own session file while
  appearing to work in a temp directory.

## Verification

Battery **`m193b` — GATE PASS**. Sources `6a19af1`, `1b79f8a`, `5f49cf1`.
Record: `audit/evidence/gate-m193b.md`; raw log `target/gate-m193b.log`.

| Stage | Exit | Time |
|---|---|---|
| `fmt --check` | 0 | 3 s |
| `clippy --workspace --all-targets -- -D warnings` | 0 | 78 s |
| `test --workspace` | 0 | 452 s |
| `demo` | 0 | 43 s |
| `check -p epsilon-cli --features real` | 0 | 14 s |

**63 targets · 1,461 results · 0 failed.** `epsilon-cli --lib` **128**, up from 125 — the
three tests above and nothing else.

Battery `m193` failed clippy in 2 seconds: `new_session_path` had been spliced **between
`fork_branch_at`'s doc block and its `pub fn`**, so the documentation described a different
function. Nothing was wrong with either method. Its other stages passed with the same 63
targets and 1,461 results the passing battery later reported — so the test total could not
distinguish the two runs, and only clippy could.

## Limits

- **pi's per-type entry payloads are not ported.** `SessionEntryBase` gives the four common
  fields; its subclasses add `message`, `thinkingLevel`, `modelId`, `summary`,
  `firstKeptEntryId`, `tokensBefore`. Epsilon emits only the base four. A client wanting a
  message entry's content still has to call `get_messages`. Inventing the subclass shapes
  from epsilon's `kind` enum would be another shape guessed rather than read.
- **`new_session` cannot choose a directory.** The file goes beside the current session.
- **A fork of a fork is untested**, in-process and over the wire.
- **Two sessions open at once is untested.** The `another run is already active` refusal
  seen in the probe is a *run* guard, not a session guard.
- **The probe is scratch, not committed.** It is a tool for one investigation;
  `scripts/` holds tools the project keeps. The next wire probe will have to rediscover the
  three traps above — which is the strongest argument for promoting it, and the reason that
  has not been done yet is that no second probe exists to justify the shape.
