# Provenance — `one-three-one-rule`

## What this is

A **verbatim copy** of a skill authored by the **Hermes Agent** project.

- **Upstream:** `references/hermes-agent-2026.9.24/optional-skills/communication/one-three-one-rule/SKILL.md`
- **Author / organisation:** Nous Research — <https://github.com/NousResearch/Hermes-Agent>
- **Licence:** **MIT**. Hermes' `LICENSE` reads "Copyright (c) 2025 Nous Research".

## What was changed

**Nothing.** The file is byte-identical to upstream. This is a copy, not an adaptation, which
is why attribution lives here rather than in a header in the file.

## Why it earns its place

`AGENTS.md` §14 says engineering judgment overrides the file whenever there is a good reason —
but it gives **no format for showing that judgment to a person**. This is that format: a
one-sentence problem, exactly three genuinely different options with their trade-offs, one pick
with the reasoning, then a definition of done and a plan that follow the pick.

The "three, not two" constraint is the useful part. Two options is a false choice presented as a
decision; four is an unreadable list. Three forces the third to be a real alternative rather
than a strawman, which is where the thinking usually was going to happen anyway.

About 70 lines of prose, so the context cost of having it available is small.

## Vetting performed before installing

The Ponytail-gate standard from `AGENTS.md` §0:

- **Instructions only.** Prose and a format. No shell, no scripts, no code execution, no network
  calls.
- **Relevant and additive.** Fills a documented gap; restates nothing `AGENTS.md` says.
- **No conflict** with `AGENTS.md`.
- **Licence compatible.** MIT into an MIT project.

## When an agent should reach for it

The user asks for options on a decision with real trade-offs, or an architectural call is being
made that would want an ADR anyway.

## Sibling skills considered and not installed

The same Hermes corpus contains `decision-questionnaire`, which turns a decision blocked on
someone else's knowledge into a Markdown questionnaire for them. Skipped: this repository is
worked by agents, and it assumes a human stakeholder in a loop to fill one in.