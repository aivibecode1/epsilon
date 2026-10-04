# Provenance — `ux-triage`

## What this is

An **adaptation** of a skill that ships with the Hermes Agent reference harness. Renamed from
`adversarial-ux-test` to say what the agent actually ends up doing, which is triage.

- **Upstream:** `references/hermes-agent-2026.9.24/optional-skills/dogfood/adversarial-ux-test/SKILL.md`
- **Author / organisation:** Omni, Comelse
- **Licence:** **MIT**. Hermes' `LICENSE` reads "Copyright (c) 2025 Nous Research"; the
  upstream frontmatter names the same licence. The same licence is recorded for Hermes in
  this repository's `CREDITS.md`.

`references/` is read-only input. The upstream file is untouched.

## What was changed

**Kept.** The hostile-persona idea and its justification — a technically correct surface can
still be unusable, and the persona catches friction a bug report will not. The five
questions that define a persona. The rant written in character. The **mandatory**
pragmatism filter into RED / YELLOW / WHITE / GREEN, with its filter criteria. The
click-count heuristic: more than five steps to the one task is a RED finding whatever the
persona's technical level.

**Deleted.**

- The browser tour: navigating to a URL, following links, clicking through a site. This
  repository is a terminal agent and a native desktop IDE. Nothing here is browsed.
- Every screenshot, and the console-error check on every page.
- Ticket creation in full — the RED/GREEN ticket template, the YELLOW catch-all ticket, the
  ten-ticket cap, the `ux-review` label and the report's ticket links. There is no tracker
  wired into a repository-local skill, and inventing one is how a skill becomes a tool
  nobody can run.
- The "test on staging, register as a new user" rules, and the industry persona table,
  which described products this repository does not have.

**Retargeted.** Step 2 ("browse as the persona") is now: run the product and attempt their
one task, and read the screen at the sizes it is used at — 80x24 and 100x30. The upstream
instruction to screenshot every pain point becomes the stricter rule `AGENTS.md` §5 already
holds: a frame is not asserted until someone has looked at it. The persona list is now the
four that actually exist here: a first-day terminal operator, a keyboard-only operator, a
screen-reader operator on the desktop, and someone on a narrow window.

**Added.** Step 7 — turn every RED into evidence before anyone fixes it — and the explicit
note that a persona pass is **not** an accessibility conformance test, because `AGENTS.md`
§8 is the standard and a persona who never mentions contrast does not make the contrast
fine. Also the **Test Matrix** and **When NOT to use it**.

## Why it earns its place

`AGENTS.md` §8 sets the floor for the UI and §5 insists frames be looked at, but neither
tells an agent how to find the *friction* between a conforming surface and a usable one.
The filter is what keeps this from becoming a complaint queue: raw persona output is noise,
and the four buckets are what turn it into work.

## When an agent should reach for it

After a batch of surface changes, before a demo or a release, and whenever someone asks
"would a real person actually use this". Bring it to a surface that has grown without
anyone approaching it as a first-time operator.

Not for correctness, security or performance review, not for accessibility conformance, and
not before anything has shipped.