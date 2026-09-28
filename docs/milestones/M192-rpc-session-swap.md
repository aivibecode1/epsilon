# M192 — fork, clone, new_session, switch_session

## What

pi's four session-swap commands, read from `rpc-mode.ts` rather than from `rpc-types.ts`.
They bring RPC client-command coverage from **18 of 33 to 23 of 33** — and establishing
that number honestly took reading pi's `RpcCommand` union against the enum variant by
variant, because the inherited "18" was wrong. See *The count was wrong too* below.

They also wire `SessionStore::fork_branch_at`, which had **eight references, every one of
them a test**, and no production caller. It is pi's fork primitive, written and tested in
M-something and then left unwired — the same class as `branch_with_summary`, and the reason
this milestone is worth more than four new ops.

## Reading the implementation changed the design

`rpc-types.ts` gives each command a request and a response type. Read on its own it says
four independent commands with four unrelated payloads. `rpc-mode.ts` says something
different:

```ts
case "fork": {
    const newPath = sessionManager.fork(entryId, ...);
    await rebindSession();                       // <-- shared
    return success(id, "fork", { text, cancelled });
}
```

`clone`, `new_session` and `switch_session` each call **`rebindSession()`** before
replying. A type declaration cannot show you that, and skipping it is silent: the file gets
written, the reply looks right, and the next prompt persists to a file nobody is reading.

So the milestone is really one helper. `rebind_to` is `rebindSession` for epsilon's shape,
and its two halves are ordered for a reason:

1. **`attach_session_store`** swaps the durable mirror, so every later append lands in the
   new file. This must come first — the other way round, the re-projection would read the
   old file.
2. **`reproject_current_branch`** re-derives the model-facing conversation from the new
   store. Without it the agent sends the *old* session's messages to the provider while
   persisting to the *new* file.

## Two more findings from reading pi

- **`get_entries` with an unresolvable `since` is an ERROR in pi** — `Entry not found`.
  M189 chose to over-send, deliberately, and the reasoning still holds (over-sending is
  recoverable; under-sending looks like data loss). This is a **recorded divergence**, not
  a silent one.
- **`clone` refuses when there is no leaf**: `Cannot clone session: no current entry
  selected`. A clone that produced an empty session and reported success would tell a
  client "duplicated" about a session with no history. Epsilon refuses, with the same
  cause named.

## Controls

### G — write the file, never rebind

The classic half-implementation: every shape check passes, the reply is well formed, and
the next prompt goes to the file nobody is reading.

**3 failures**, each naming the missing rebind:

```
the agent must point at a new file
clone must produce a new file
(the round trip)
```

### H — rebind, never re-project

This is the control that found a real gap, and it took **two attempts**.

The first attempt passed all 59 tests. The reason was my own error: I added
`if false { reproject_current_branch()?; }` **above** the live call instead of deleting the
call. The re-projection still ran, so the control passed while claiming to disable exactly
that. Re-armed by deletion — `reproject_current_branch()?` count in the file goes to 0,
which is the assertion worth making mechanically:

**2 failures**, both reporting the previous session's 4 messages carried into the "new" one:

```
and the agent must see it as empty            left: 4   right: 0
the agent must carry NO messages from the previous session: 4 left over
```

That gap was real and every other test missed it, because they all checked **file paths and
entry counts** — which a stale conversation satisfies. The session file says "empty" while
the agent carries the old messages into the next provider call. The new
`new_session_empties_the_conversation_not_just_the_file` test exists because of it.

### A vacuous assertion, found while writing the replacement

The first version of that test ended with `texts.iter().any(|t| t.contains("hello") || !t.is_empty())`.
`!t.is_empty()` is true of every message that exists, so the disjunct asserts only that the
run produced *something* — the assertion passes whether or not the conversation is right.
Replaced with a check that names the content that must be there.

## Two incidental defects the tests found

**Temp-dir collision.** The test helper keyed its directory on
`std::thread::current().id()`, which is **identical for every `#[tokio::test]` on the same
worker**. The four new tests are async and so are the existing ones, and they deleted each
other's sessions mid-run. Now keyed on `new_id("t")` — unique per test, which is what the
name always implied.

**A test that would pass on Linux and fail on Windows.** `switch_session` takes a path, and
the round-trip test interpolated one into a JSON string literal:
`"sessionPath":"{original:?}"`. A Windows path is full of backslashes, and that is a JSON
escape error. The frame is now built as a **value** and serialized. The wrong way round for
a harness whose primary platform is Windows: the test would have been green everywhere the
product is not developed.

## The count was wrong too

Recording "18 of 33" and then adding four gave 22, and that arithmetic was wrong twice.

pi's `RpcCommand` is a union of 33 object literals, so the true denominator is exact.
Matching each against epsilon's `Op` enum **variant by variant** — resolving each
variant's actual serde name, not its Rust identifier — gives **23 of 33**, and 10
unimplemented:

```
set_steering_mode   set_follow_up_mode   compact        set_auto_compaction
set_auto_retry      abort_retry          bash           abort_bash
export_html         get_fork_messages
```

The inherited 18 was low by one, and the reason is **D26's own trap**. `follow_up`'s
primary wire name lives in a `#[serde(rename = "follow_up", alias = "followup")]`, so the
Rust variant is `Followup` and the string `follow_up` never appears as an identifier. Any
count derived by grepping the enum for wire names misses it — which is the same reason a
marker search is blind to serde attributes, recorded under "Protocol surfaces" in
`AGENTS.md`.

**A parity count computed by a script that has never been shown to be right is a claim,
not a measurement.** The way to make it one is to derive the denominator from the
reference's own type union (33, exact) and to resolve every implementation name through
serde rather than through the identifier. A count that requires trusting the previous
count is a count that drifts.

## Verified over a real wire, and what that found

`audit/evidence/installed-binary-2026-09-28-m192-m193.md` — frames into the installed
release's stdin, JSON out of its stdout.

All four ops work, and the properties that matter were measured rather than asserted:
`fork` names what it forked from and shortens the conversation from 2 entries to 1; the
tree re-derives to 1 root; `new_session` is empty in **both** senses (0 entries, 0
messages); `switch_session` restores the original's 2 entries.

**It also found two defects that 1,458 passing tests did not:**

- **D29** — `get_entries` was serving epsilon's internal entry shape
  (`kind`, `parent_id`, `seq`, `timestamp_ms`) while `get_tree`, four lines away, served
  pi's. Two ops, one reference type, two shapes. The in-process tests passed because they
  asserted `get_entries` against itself. This is D27's twin, and the lesson is the same
  one twice: **a wire probe is not a stricter unit test, it is the only kind of check that
  can see what a client receives.**
- **A new session was named differently from a fork** — `ses_….jsonl` beside
  `2026-09-28T10-28-30-472Z_ses_….jsonl` in the same directory. Nothing failed; two
  spellings in one directory simply sort into two groups.

Both are fixed in M193 (`6a19af1`, `1b79f8a`).

## Verification

Battery **`m192b` — GATE PASS**. Source `3baff78`, clippy fix `a6ab1bf`.
Full record: `audit/evidence/gate-m192b.md`; raw log `target/gate-m192b.log`.

| Stage | Exit | Time |
|---|---|---|
| `fmt --check` | 0 | 2 s |
| `clippy --workspace --all-targets -- -D warnings` | 0 | 46 s |
| `test --workspace` | 0 | 268 s |
| `demo` | 0 | 6 s |
| `check -p epsilon-cli --features real` | 0 | 1 s |

**63 targets · 1,458 results · 0 failed.** `epsilon-cli --lib` **125**, up from 116 at M191
— **+9**, the nine new tests, read as a per-binary delta rather than inferred from totals.

The first battery, `m192`, **failed clippy** on one unused test binding while every other
stage passed with the same 1,458 results. Only `-D warnings` caught it; no test would have.
Fixed in `a6ab1bf` and re-run on the fixed bytes.

## Limits

- **`new_session` puts the new file in the current session's directory**, with `cwd`
  derived as that directory's parent. pi takes the runtime host's own answer. A client that
  wants sessions in a configured location has no way to say so.
- **The `cwd` that `switch_session` computes is discarded** (`let _ = cwd;`). Reading the
  target's recorded cwd is right — reusing the current session's would reinterpret every
  relative path in the file — but nothing consumes it yet, because the run loop takes its
  working directory from the process. The line is kept because deleting it would lose the
  reasoning, and named so the next person knows it is a stub, not an oversight.
- **`fork` does not re-parent the original.** pi's fork leaves the source session alone and
  epsilon matches it, so a client cannot tell a fork from a move except by watching paths.
- **Orphan and self-parent tree roots are implemented and covered in M190's in-process
  tests**, but not reachable over a real wire yet — no `get_fork_messages`.
- **A fork of a fork is untested.** The store's own tests cover the primitive; the RPC path
  does not.
