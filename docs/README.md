# Documentation

Epsilon's documentation is organised by **what you need now**. The numbered documents
are the authoritative entry points; the subject folders hold detail that would make an
entry point too large.

The code and [`04-STATUS.md`](04-STATUS.md) are the authority. There is no archive: a
document that is no longer true is corrected or deleted, not kept beside the one that is.

## Start here

| If you want to… | Read |
|---|---|
| see what Epsilon is and run it | [README](../README.md) |
| know the release classification and verified evidence | [RELEASE.md](../RELEASE.md) |
| get a guided tour in ten minutes | [01-OVERVIEW](01-OVERVIEW.md) |
| know what is true right now | [04-STATUS](04-STATUS.md) |
| use or configure the product | [05-USER-GUIDE](05-USER-GUIDE.md) |
| look up a slash command | [07-COMMANDS](07-COMMANDS.md) |
| change the code | [AGENTS](../AGENTS.md) and [03-DEVELOPMENT](03-DEVELOPMENT.md) |
| understand the terminal | [09-TUI-REFERENCE](09-TUI-REFERENCE.md) |
| add a tool, server, skill or frontend | [10-EXTENSION-POINTS](10-EXTENSION-POINTS.md) |
| understand the design | [02-ARCHITECTURE](02-ARCHITECTURE.md) and [design/epsilon-decisions](design/epsilon-decisions.md) |

## The numbered documents

| File | What lives there |
|---|---|
| [01-OVERVIEW.md](01-OVERVIEW.md) | Product tour, first run, folder map and reading order |
| [02-ARCHITECTURE.md](02-ARCHITECTURE.md) | Crates, dependency edges and execution paths |
| [03-DEVELOPMENT.md](03-DEVELOPMENT.md) | Build, test, battery, machine rules and recovery |
| [04-STATUS.md](04-STATUS.md) | What works, the latest gate, limitations and next work |
| [05-USER-GUIDE.md](05-USER-GUIDE.md) | Task map for install, auth, models, permissions, sessions, goals |
| [06-GLOSSARY.md](06-GLOSSARY.md) | Harness, turn, tool call, compaction, lease and the rest |
| [07-COMMANDS.md](07-COMMANDS.md) | Every slash command, alias, argument and confirmation |
| [08-ARCHITECTURE-REVIEW.md](08-ARCHITECTURE-REVIEW.md) | The reference comparison, the gaps it found, and the work waves that follow |
| [09-TUI-REFERENCE.md](09-TUI-REFERENCE.md) | The frame, regions, focus rules, keys, themes and states |
| [10-EXTENSION-POINTS.md](10-EXTENSION-POINTS.md) | Every seam: tools, MCP, extensions, skills, agents and frontends |

## Detailed references

| Folder / file | What lives there |
|---|---|
| [design/epsilon-decisions.md](design/epsilon-decisions.md) | The ADR log — why the product is shaped this way |
| [design/epsilon-skills.md](design/epsilon-skills.md) | Skill discovery, the catalogue budget, and opt-in built-ins |
| [user/](user) | Install, configuration, auth, models, permissions, sessions, goals, skills, subagents, reverse engineering, extensions, templates, troubleshooting |
| [parity/pi-tui.md](parity/pi-tui.md) | The live Pi terminal-IDE comparison |
| [references/README.md](references/README.md) | The six reference harnesses and the REA integration, one page each |
| [../scripts/README.md](../scripts/README.md) | The fourteen-leg battery and every repository check |
| [README credits](../README.md#credits) | Attribution and licences |

## Reading paths

**New to coding agents?** [01-OVERVIEW](01-OVERVIEW.md) with
[06-GLOSSARY](06-GLOSSARY.md), then [02-ARCHITECTURE](02-ARCHITECTURE.md).

**Using the product:** [05-USER-GUIDE](05-USER-GUIDE.md) → the specific page under
[user/](user) → [07-COMMANDS](07-COMMANDS.md) when you need the exact spelling.

**New to this codebase:** root [README](../README.md) →
[02-ARCHITECTURE](02-ARCHITECTURE.md) → the crate you are touching → its tests.

**Reviewing a change:** [AGENTS](../AGENTS.md) → the relevant ADR in
[design/epsilon-decisions.md](design/epsilon-decisions.md) → the test that names the
behaviour.

**Deciding what to build next:** [04-STATUS](04-STATUS.md), then the security and
accessibility limits it names.

## Conventions

- The numbered documents are the entry points. Subject folders hold detail.
- A document is current or it is gone. Do not add a "history" copy beside a live one;
  correct the live file and record the decision in the ADR log or `04-STATUS.md`.
- Every link is checked by `scripts/check-links.py`; every command in a guide must have
  been run.
- Plain language. Define a term at first use. Examples over adjectives.
