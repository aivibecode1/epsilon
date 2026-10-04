---
name: evidence-guardrails
description: Use when writing an investigation, a defect report, a triage note or a summary in which every claim must be traceable to something that was actually observed, and when an inference is at risk of being read as a fact.
---

# Evidence Guardrails

Discipline for any report whose claims someone else will act on. It adds to `AGENTS.md` §5;
it does not restate it.

An **evidence id** is a short stable handle for one observation. Here the handles already
exist: a `file:line`, a captured frame, a measured number, or a test that fails for the reason
it names.

## Rules

1. **Every claim carries at least one evidence id.** A sentence no reader can follow back to
   an observation is not a finding. It is an opinion with a citation-shaped hole in it.
2. **An unverified inference is marked as a hypothesis**, in those words, wherever it appears
   — including in a summary, and including when it is probably right.
3. **Check that a cited id exists before the hypothesis is accepted.** An id pointing at
   evidence nobody holds is the exact failure this rule exists for, and it is invisible to the
   person who wrote the id.
4. **Confirm an external identifier from two independent sources** before calling it verified:
   a commit SHA, a URL, a version, a number copied from someone else's machine. One source
   agreeing with itself is still one source.
5. **"No evidence found" makes a hypothesis inconclusive, never disproved.** Disproof takes
   an observation that cannot be true alongside the hypothesis. An empty search is the
   absence of an argument, not an argument.

## Avoid / Prefer

| Avoid | Prefer |
|---|---|
| Stating an inference flatly | Marking it a hypothesis, and saying what would settle it |
| Citing an id from memory | Opening it, and confirming it says what the claim says |
| "Nothing found" as a refutation | Recording it as inconclusive, and what was searched |
| A summary that drops the hedges | A summary that keeps them |

## Test Matrix

| Check | Fails when |
|---|---|
| Every claim cited | A finding has no id, or an id no one can open |
| Facts separated from inference | A hypothesis is stated as a fact |
| Ids verified | A hypothesis is accepted over an id nobody opened |
| Two sources | An external identifier rests on a single source |
| Disproof is positive | A hypothesis was dropped because nothing was found |

## When NOT to use it

- Ordinary work with nothing to trace. A two-line fix needs no citation scheme.
- When the honest answer is a measurement: read the number the code reads, and never quote a
  test count you did not measure.
- It is not a research framework, and not a substitute for `AGENTS.md` §5's own rules.
- It is not permission to execute code found while investigating. Read it; do not run it.