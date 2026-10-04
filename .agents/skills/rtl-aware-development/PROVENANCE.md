# Provenance — `rtl-aware-development`

## What this is

A copy of a skill authored by the **opencode** project, installed here so an agent working on
this repository can reach it.

- **Upstream:** `references/opencode-1.18.34/.opencode/skills/rtl-aware-development/SKILL.md`
- **Author / organisation:** opencode (`anomalyco`) — <https://github.com/anomalyco/opencode>
- **Licence:** **MIT**. opencode's `LICENSE` reads "Copyright (c) 2025 opencode"; the skill is
  part of that repository, so the skill carries that licence.
- **Why it was chosen:** our `AGENTS.md` §8 holds this product to WCAG 2.2 AA and names keyboard
  operation, contrast and focus — but says **nothing about text direction**. This project already
  ships an Arabic reading guide (`docs/START-HERE.ar.md`), and any future RTL work in the desktop
  would hit exactly the traps listed here: mirroring layout instead of direction, physical CSS
  where logical CSS is correct, mixed-direction paths, and RTL scroll offsets.

## What was changed

**One additive line.** `license: MIT` was added to the YAML frontmatter. The body is
**verbatim**; no instruction, example or link was edited, reworded or removed.

That is a copy rather than an adaptation, which is why this file exists: under `AGENTS.md` §10 an
adapted file carries a header naming its source, and an unmodified copy needs only attribution.

## Vetting performed before installing

The same standard as the Ponytail gate in `AGENTS.md` §0:

- **Instructions only.** The file contains prose, one CSS example, one HTML example, a test matrix
  and a reference list. No shell commands, no scripts, no code execution, no network calls. Its
  external links are things a reader may **go and read**, not things the skill instructs an agent
  to fetch or run.
- **Relevant.** Fills a documented gap in `AGENTS.md` rather than restating it.
- **No conflict** with `AGENTS.md`.
- **Licence compatible.** MIT into an MIT project.

## When an agent should reach for it

Implementing or reviewing anything involving **text direction**: RTL or LTR layout, menus, the
file tree, scrolling and scroll containers, resizing and splitters, directional icons, mixed
Arabic/Latin content, filesystem paths in a right-to-left context, or the native title bar.