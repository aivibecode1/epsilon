---
name: two-stage-review
description: Use when a change is finished and someone other than its author has to judge it, when a delegated implementation comes back for review, or when a change must be checked for doing what was asked before it is judged on how it is built.
---

# Two-Stage Review

Judge a change twice, in this order, by two different readers: first whether it is **what was
asked for**, then whether it is **well built**.

## The order is the point

**Spec compliance comes first, strictly.** A spec gap found after a quality pass costs two
passes, because the code the quality reviewer approved has changed underneath them. The
reverse costs nothing. Missing a requirement and adding an unasked-for one are both spec
failures, and both are cheap to name early and expensive to find late.

The quality review does not start until spec compliance passes. Not in parallel, not "at the
same time".

## The rules

1. **The implementer's self-review never substitutes** for either stage. The implementer is
   the least independent reader of its own work: it knows what it meant, and reads the diff
   looking for that.
2. **The reviewer is not the author.** `AGENTS.md` §12 requires it, and the ordering is worth
   nothing without it.
3. **The loop closes.** A reviewer reports issues, the implementer fixes them, and **the
   same reviewer looks again**. A fix that was not re-reviewed is a claim, not a fix.
4. **No open issues move on**, and "close enough" is not compliance.

## What the spec is, here

The task's stated requirements, **plus the repository rules that apply to it**: an ADR for a
behaviour-level change, the evidence discipline for anything a user sees, documentation with
resolving links, licence and attribution. A spec review that checks only the literal request
has reviewed the wrong spec, and the quality review will not catch the difference — it is
not looking for it.

## Avoid / Prefer

| Avoid | Prefer |
|---|---|
| One reviewer judging both at once | Spec first, on its own, with its own verdict |
| Starting quality work before spec passes | Waiting for a real pass, not a hopeful one |
| The author approving its own work | A reader who did not write it |
| Fixing and moving on | Re-review by the same reviewer |
| A review that is really a second opinion | A verdict that can be wrong |

## Test Matrix

| Stage | Catches | Cannot catch |
|---|---|---|
| Spec compliance | Missing requirements, scope creep, unasked-for behaviour | Whether the code is idiomatic or safe |
| Code quality | Duplication, a wrong layer, a missing failure path | Whether the change was wanted at all |
| Self-review | Almost nothing; what the author intended | Its own blind spots |

## When NOT to use it

- A mechanical change with one obvious implementation. Review it; do not stage it.
- When no independent reader is available. Say so in the report. A self-review presented as a
  review is worse than an admitted gap, because it stops the next person looking.
- It is not a substitute for the author's own tests. It judges a finished change.