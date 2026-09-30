# M230 — a subagent's tokens reach the session's numbers

**Ledger row A11, closed.** M229 measured the gap and specified the three insertions; this is the
change. A `task` child's provider calls are the parent's spend, and they now appear in every number the
user can read.

## What was invisible, and where it was visible

M229's finding, re-measured here in the test's own failure output — the delegated bill of 7,000 in /
3,000 out, absent from a session that had spent it:

```text
panicked at crates/epsilon-core/tests/billed_tool_usage.rs:127:
the tool's own provider call is the session's spend too:
Usage { input: 200, output: 100, … }        <- the parent's two turns, and nothing else
```

The tokens were stored on the task entry (`lib.rs:3954`) and persisted in the tool result
(`exec.rs:839` → `ToolResultMessage.usage`). They were read by nothing that adds up.

## The change, in three places that move together

| # | where | what |
|---|---|---|
| 1 | `core/src/lib.rs`, the turn's result loop | `usage_total.combine(tool_usage)`, and `session_usage_by_model[details["model"]] += usage` when the tool names a model |
| 2 | `session_usage()` / the header / the cost panel | follow (1) with no change of their own — they read the two counters the loop now feeds |
| 3 | `/task list` row (`app/commands.rs`) | the child's own `N in / M out`, formatted with the existing `format_tokens` |

**Why in the loop and not at the tool boundary.** `exec.rs` cannot reach the turn's local
`usage_total`, so a fold there would land in the per-model map while the header kept under-reporting
— two numbers disagreeing about the same tokens, which is the class this repository treats as a
defect. Folding where the turn's total is live is what makes the transcript, the header and the cost
panel move as one.

**A bill with no model still counts.** `usage_total` takes every `Some(usage)`; the per-model row needs
`details["model"]`, because pricing a bill without a model means inventing a rate (M141's defect). So
an unattributed tool call under-reports *where* it went, never *whether* it was spent — and that
asymmetry is a test, not a comment.

## Verification

**Battery `m229-code` GATE PASS** — `audit/evidence/gate-m229-code.log`: **1,655 tests across 66
binaries** against M229's 1,652 across 65: **+3 tests and +1 binary** (the new
`crates/epsilon-core/tests/billed_tool_usage.rs`). Gate shape: check-links 0 s, check-lint-tables 0 s,
fmt 3 s, clippy 35 s, test 324 s, demo 22 s, real-check 7 s. Run with `CARGO_PROFILE_DEV_DEBUG=0` /
`CARGO_PROFILE_TEST_DEBUG=0`.

**Three tests, written before the fix and watched to fail:**

- `a_tool_that_bills_its_own_provider_call_is_in_the_turns_total` — the total *and* the per-model row,
  because either alone passes against a half-done fix.
- `a_bill_with_no_model_still_counts_toward_the_total` — and that no empty-string model row appears.
- `tasks_command_shows_what_a_child_cost` — a **foreground** child, so the assertion cannot be
  skipped by a task that has not settled; the background test above it keeps its original shape.

**The control**: with the fold disabled, both core tests fail and print the exact numbers —
`input: 200, output: 100` where the session had actually spent 7,000/3,000 more. The threshold is
deliberately larger than the faux provider's own turn (100/50), because the first version of the
second test passed against the broken build: 200 ≥ 100 was satisfied by the parent's own turns. **A
test whose threshold the broken build already meets is not a test** — that is what the first run of it
showed, and the number was raised rather than the assertion weakened.

## Limits

- **Only the `task` tool reports usage today**, so this changes nothing user-visible until a second
  billing tool exists — and the *rule* is what lands, not a feature. The `/task` row is the one
  surface a user can see today, and it now prints the child's spend.
- **A background child's spend appears when the completion notice lands**, not while it runs: the
  tool result carrying the usage only exists once the child settles. A user watching a long task sees
  nothing until it finishes.
- **The per-model row needs the tool to name its model.** The `task` tool does (`details.model` is the
  child model); a future billing tool that forgets would land in the total and nowhere else.
- **No test drives the real `TaskTool` through the loop** — the fixture is a tool that bills, which is
  what the rule is about. The path from *subagent* to *tool result* is covered by `task_scenarios.rs`;
  the path from *tool result* to *totals* is covered here; nothing covers the two together, which is
  the seam a future change to `run_task`'s `TaskHostResult` could break silently.
