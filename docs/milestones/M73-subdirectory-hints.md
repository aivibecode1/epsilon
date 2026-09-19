# M73 — subdirectory hints: the conventions next door

**Scope:** port-plan item 11 (Hermes `agent/subdirectory_hints.py:72,30,48`).

## The problem

Only the workspace-root `AGENTS.md`/`CLAUDE.md` reached the model. In a monorepo — including this
one, where every crate has its own rules — the model working in `crates/epsilon-tools/` was guessing
at conventions that were written down one directory away. The failure is quiet: nothing errors, the
work is just slightly wrong in the way the crate's own instructions exist to prevent.

## What landed

**Recording.** `Agent::note_touched` remembers the directory of every tool call that passed
validation and will run. It is called from the batch preflight, where M72's `resource` (the call's
declared resource key) is already computed — so this feature costs one `HashSet` insert and reads
nothing. Directories outside the workspace are ignored, and so is the workspace root itself (already
loaded as project context). The set is sorted on read, because the prompt must not depend on the
order directories were discovered.

**Loading.** `prompt::load_subdirectory_hints(workspace, touched)` walks up from each touched
directory and collects the `AGENTS.md`/`CLAUDE.md` family. Every bound is explicit, and each one has
a reason:

| Bound | Value | Why |
|---|---|---|
| Ancestor walk | 5 levels | A hint governs the subtree it sits in; walking further pulls in whatever is above the workspace on a shared drive. |
| Excluded | the workspace root | The root files are already in the same tier — a hint loader that repeats them doubles every prompt. |
| Dedup | by content digest | A vendored copy of one `AGENTS.md` in three directories is one block: cheaper and less confusing. |
| Files per prompt | 5, nearest first | When a tree has more hints than the budget, the closest directory governs what is being edited. |
| Per-file size | 4 KB, announced | An unbounded include is how a prompt becomes unaffordable; the cut says so instead of ending mid-sentence. |

Hints join the **CONTEXT** tier after the root files, as `<project_instructions path="crates/x/AGENTS.md" hash="…">`
blocks — the existing shape, so the content hash stays observable and the cache discipline is
unchanged.

## Key decisions

- **Recorded from the call, not from the result.** The *intent* is what locates the conventions: a
  call that failed on a missing file still says which part of the tree is in play. It also keeps this
  a preflight-only concern, with no dependency on how the batch was scheduled (M72 splits it into
  concurrent groups).
- **A set, not a list.** The hint is the same whichever call discovered the directory, and a retry
  must not change the prompt's shape.
- **Reuse the resource key.** M72 already made every tool declare what it acts on, resolved against
  the execution cwd — so the hint loader needed no new argument parsing and cannot disagree with
  where the call actually went.
- **Lazy, and it stays lazy.** Hints appear only after the work reaches the directory. The cost is
  real and stated: the first request after a new directory is discovered changes the CONTEXT tier,
  which is one prefix cache re-read. That is the price of the feature, and it is paid once per
  directory rather than on every turn — the second and third requests in the test are asserted
  byte-identical.
- **Per process, not persisted.** A resumed session re-discovers directories as it works. Persisting
  the set would be a session-format change for a feature whose inputs are one tool call away.
- **The root file is not duplicated**, and an untouched crate's file never appears — both asserted,
  because both are the failure mode of a loader like this.

## Verification

| Mutation | Result |
|---|---|
| the touched-dir recording removed | 2 of 4 loop tests fail (the hint never arrives; the no-duplication test follows) |
| content dedup removed | `the_same_file_in_two_places_is_one_block` fails |
| the file budget removed | `the_nearest_directories_win_when_there_are_too_many` fails |
| the depth bound removed | `the_walk_stops_after_a_few_levels` fails |

New tests: 8 unit tests in `prompt.rs` (labelling, the ancestor walk, the depth bound, dedup,
nearest-first ordering, the announced truncation, the empty and outside-workspace cases, and
order-independence) and 4 loop tests in `crates/epsilon-core/tests/subdirectory_hints.rs` driving the
real loop — the central one asserts that request 1 does **not** carry the crate's conventions and
request 2 does, read back from the provider's recorded requests.

Gate: `scripts/gate.sh m73` — fmt clean, clippy zero warnings, **674 tests, 0 failed**, demo
completes, `real`-feature build compiles.

## Plan status

Item 11 is **ported**. Tier 3 still has 12 (`@`-reference expansion) and 13 (typed corruption
reasons) open.
