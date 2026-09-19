# M72 — the batch planner: parallelism where it is safe, order where it matters

**Scope:** port-plan item 10 (Hermes `agent/tool_dispatch_helpers.py:117`).

## The problem

The old rule was all-or-nothing: **any** `Sequential` tool in a batch made the whole batch a
barrier. Since `write` and `edit` are `Sequential`, a model that sent three edits to three
*different files* in one message paid for them one at a time. The rule was safe and easy to reason
about, and it left real parallelism on the floor in the single most common batch a coding agent
produces.

## What landed

Tools can now declare **what they act on**:

```rust
fn resource_key(&self, arguments: &serde_json::Value, base: &Path) -> Option<PathBuf>
```

`base` is the same directory the tool will be handed as `cwd` when it runs, so the planner's view
and the execution's view are the same by construction rather than by agreement. `read`, `write`,
`edit`, `grep`, `glob` and `ls` implement it (`resolve_in_workspace`, the existing lexical
normalizer, so `/ws/a.rs` and `/ws/./a.rs` are one file); everything else keeps the default `None`.

`plan_batch` turns a batch into a `BatchPlan`:

- **Barrier** — some call is `Sequential` with **no** resource (`bash`, `task`). Its reach is
  unknown: a shell command can touch anything, so one call at a time, as before.
- **Segments** — otherwise, calls are grouped by conflict into ordered chains; the chains run
  concurrently.

Three rules decide a conflict, and each is a decision rather than a detail:

1. **Prefix, not equality.** A `grep` over `src/` conflicts with a `write` to `src/lib.rs`: the
   paths differ as strings and are the same resource in fact. Equality would have let the search
   read a file mid-rewrite.
2. **Two readers never conflict.** Nothing needs ordering between them, and this is what keeps a
   batch of workspace-wide searches (all keyed on the root) concurrent instead of accidentally
   serialized by their own shared key.
3. **Same resource ⇒ one chain, in call order.** Not "the same key is deduplicated" but *ordered*:
   "read this file, then edit it" and "edit this file twice" mean what they say. The chain is
   built by union-find over conflicting pairs, not by greedy bin-packing — greedy would put call 3
   in a second chain and run it *beside* call 1 while the model asked for it after.

Inside a chain, calls run in order; the semaphore permit is taken per call (not per chain), so a
long chain never holds a slot while it waits. Results still commit in model order regardless of
completion order (the DSH rule the pool already followed).

## Key decisions

- **The role comes from `ExecutionMode`, not from a second vocabulary.** Readers declare
  `Parallel`, mutators `Sequential` — the distinction the pool already enforced. A separate
  read/write flag could disagree with the mode; one source cannot.
- **Blocked calls do not serialize anything.** A call that failed validation, was denied by the
  gate, or names an unknown tool never executes; it is scheduled as unkeyed-and-parallel so it
  costs the batch nothing. Under the old rule it did.
- **`base` rather than an absolute path in the arguments.** The tool author does not have to know
  where the workspace is; the planner supplies it, and it is the same value execution uses.
- **A directory key is legitimate.** `ls`/`grep`/`glob` key on the directory they search; the
  prefix rule is what makes that work with the file keys of mutations.
- **`todo` needs no key** — it is pure in-memory state (verified), not a file, so it stays
  `Parallel` and keyless. Declaring a resource it does not touch would create false ordering.

## Test-caught bugs

The first version of the concurrency test used a `tokio::sync::Barrier` as a rendezvous: two calls
would only complete if they overlapped. It **passed while the batch was fully serialized** — a
waiter that times out is still counted as an arrival, so the first (serialized) call released the
second, and both reported success. Replaced with an in-flight counter whose high-water mark is
asserted directly: it measures overlap without any primitive that can be fooled, and the test
runtime fell from 5.0 s (waiting out timeouts) to 0.33 s.

The shell test had the mirror-image flaw: with one call in the batch, "waited alone" was also what
a *missing partner* looks like, so it could not distinguish serialization from an absent peer. It
now runs two calls to two different files — which overlap on their own, as the sibling test proves
— plus the shell call that must stop them.

Two further test premises were wrong and were fixed rather than accommodated: calls fed as separate
`FauxStep`s are separate *turns*, not a batch (they never exercised the planner at all), and one
`conflicts()` mutation was a no-op because the early-return above it already decided the answer.

## Verification

| Mutation | Result |
|---|---|
| every pair conflicts (one chain, the old behaviour) | `edits_to_different_files_run_at_the_same_time` fails |
| chain order reversed | `edits_to_the_same_file_run_one_after_the_other` fails |
| barrier rule removed (a shell call no longer serializes) | `a_shell_call_keeps_the_whole_batch_serialized` fails |

New tests: 8 unit tests over `plan_batch` (including blocked calls, prefix conflicts, and readers
that must not wait for each other), 3 loop-level tests that measure overlap and order through the
real loop, and 1 in `epsilon-contracts` asserting the default `resource_key` is `None` with the mode
that makes `None` mean "unknown reach".

Gate: `scripts/gate.sh m72` — fmt clean, clippy zero warnings, **662 tests, 0 failed**, demo
completes, `real`-feature build compiles.

## Plan status

Item 10 is **ported**. Tier 3 still has 11 (subdirectory `AGENTS.md` hints), 12 (`@`-reference
expansion) and 13 (typed corruption reasons) open.
