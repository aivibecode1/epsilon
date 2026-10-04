# Credits

Epsilon stands on other people's work. This file names it, so that a reader can go and check.

Every fact below — author, link, licence — was read from that project's **own** `LICENSE`,
`README` and package metadata in the vendored copy under `references/`, not from memory.
Where something could not be verified from those files, the row says so rather than guessing.

## Reference harnesses

Epsilon began as a port of **pi** and has since been shaped by five more projects. All six are
**MIT-licensed**, and Epsilon is MIT, so code from any of them could legally have been adapted
with attribution. In the event, **no third-party source code was copied into this repository**:
what was taken is behaviour, structure and design decisions, described below and documented in
[`docs/references/`](docs/references/README.md). That is why there is no
`THIRD_PARTY_NOTICES` file carrying vendored bodies.

| Project | Author / organisation | Repository | Licence |
|---|---|---|---|
| **pi** | Mario Zechner (`earendil-works`) | https://github.com/earendil-works/pi | MIT — "Copyright (c) 2025 Mario Zechner" |
| **DeepSeek Harness** | DeepSeek | https://github.com/deepseek-ai/deepseek-harness | MIT — "Copyright (c) 2026 DeepSeek" |
| **Hermes Agent** | Nous Research | https://github.com/NousResearch/Hermes-Agent | MIT — "Copyright (c) 2025 Nous Research" |
| **opencode** | opencode (`anomalyco`) | https://github.com/anomalyco/opencode | MIT — "Copyright (c) 2025 opencode" |
| **OpenTUI** | opentui (`anomalyco`) | https://github.com/anomalyco/opentui | MIT — "Copyright (c) 2025 opentui" |
| **codewhale-ratatui** | Codewhale Contributors | https://github.com/Hmbown/codewhale-ratatui | MIT — "Copyright (c) 2026 Codewhale Contributors", and "Portions adapted from Codewhale: Copyright (c) 2024-2025 DeepSeek-TUI Contributors" |

### What each one taught us

**pi** — the agent harness this project is a port of, and the reason its loop is shaped the way
it is: a two-level cycle of provider turns and tool batches, with steering and follow-up queues
that let a person interrupt a run without cancelling it. Epsilon's `epsilon-core` exists
because of pi's design.

**DeepSeek Harness** — bounded capture of subprocess output, subprocess lifecycle discipline,
design tokens, and above all the shape of an IDE workflow: side bar, editor, agent panel,
status. We took the workflow. We deliberately did **not** take its carrier — DSH's own desktop
is an Electron shell that points a `BrowserWindow` at a web profile over HTTP, and Epsilon's
desktop is a native window. That is a difference in what each project is for, not a criticism;
see ADR-020 in [`docs/design/epsilon-decisions.md`](docs/design/epsilon-decisions.md).

**Hermes Agent** — the discipline of an observer that knows what it has actually seen: read
observations scoped to a lifetime token, verification evidence attached to a claim rather than
asserted beside it, and recovery paths that assume something went wrong.

**opencode** — permission rules worth the trouble of reading closely, change provenance, and
editing guided by real language-server feedback rather than by a model guessing. It is also the
project that put OpenTUI into production, which is how it came to our attention.

**OpenTUI** — three ideas, taken without the library: layout declared as relationships and
resolved in one place; key binding separated from command routing; and an agent-facing
`AGENTS.md` that briefs delegates with owned files and required checks, and asks for an
independent review of anything actionable. The reasoning for not adopting the library itself
is in [`docs/references/TUI-DECISION.md`](docs/references/TUI-DECISION.md).

**codewhale-ratatui** — the presentation-layer discipline of a component kit that owns paint
and nothing else, and the terminal-profile fallbacks Epsilon actually has (`NO_COLOR`,
ASCII-only glyphs) plus the discipline of pinning a profile per test so a frame is compared
against a known rendering environment. Its whale-and-ocean identity belongs to its authors and
is not ours; we took the craft, not the character. Epsilon has **no** workbar and **no**
reduced-motion path — those are gaps, not parity, and are recorded as such in
[`docs/references/codewhale-ratatui.md`](docs/references/codewhale-ratatui.md).

## Tools

| Project | Author / organisation | Repository | Licence |
|---|---|---|---|
| **Ponytail** | DietrichGebert | https://github.com/DietrichGebert/ponytail | MIT — "Copyright (c) 2026 DietrichGebert" |
| **improve** | shadcn | https://github.com/shadcn/improve | MIT — "Copyright (c) 2026 shadcn" |

(The Ponytail repository's commit history records the author identity as
`Peter <8146306+p-clements@users.noreply.github.com>`, which is consistent with that
handle.)

**Ponytail** is the minimalism discipline this project works by, and it is installed as an
agent skill at `.agents/skills/ponytail*`. What we took is its ladder — does this need to
exist, is it already here, does the standard library do it, does the platform do it, is an
installed dependency enough — and its rule that the ladder runs *after* reading the problem,
never instead of reading it. It is also explicit that it must never be used to justify
skipping input validation, error handling that prevents data loss, security, or accessibility,
and this repository's `AGENTS.md` repeats that limit on purpose.

**improve** ([shadcn/improve](https://github.com/shadcn/improve)) is installed as an agent
skill at `.agents/skills/improve`, from `skills/improve` in that repository; the copy in this
workspace declares itself version 1.1.0 and MIT. It is a senior-advisor skill: it surveys a
codebase and writes **plans for a different, less capable executor**, and its first hard rule
is that it never edits source itself. Two ideas from it shaped how this project runs audits:
the plan is the deliverable, and it must be self-contained enough to execute with no memory of
the conversation that produced it. Epsilon's own record-keeping follows that shape — findings
in `docs/STATUS.md` and `audit/03-defects.md` are written to be acted on by whoever picks them
up next, not to be re-derived.


## Data

**[models.dev](https://models.dev)** supplies model catalogue research via
`scripts/regen_catalog.py`. It is a data source, not a code dependency.

## Epsilon

Licensed under the [MIT Licence](LICENSE). Copyright © 2026 the Epsilon authors.
