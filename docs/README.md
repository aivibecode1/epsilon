# Documentation

For users and contributors finding current guides, design intent and historical records.
The implementation is the authority when documents disagree; older design and milestone
claims are not proof that a capability works today.

## Product documentation

| File | What lives there | Read it when |
|---|---|---|
| [`../README.md`](../README.md) | the entry point: what Epsilon is, install, quick start | first |
| [`user/`](user) | product guides: install, configuration, auth, permissions, sessions, troubleshooting | you are using or configuring Epsilon |
| [`../CHANGELOG.md`](../CHANGELOG.md) | user-visible changes, milestone by milestone, newest first | you want to know what changed |

## Contributor documentation

| Folder / file | What lives there | Read it when |
|---|---|---|
| [`../AGENTS.md`](../AGENTS.md) | the rules for anyone writing code here: gates, conventions, disk hygiene | before your first change |
| [`design/`](design) | **how and why**: the architecture tour, the ADR log, the skills design | you are changing behaviour |
| [`qa/`](qa) | design conversations turned into notes: why Rust, the brick methodology, the harness primer, TUI keymaps, OAuth client ids, provider extension points, the repository layout | you need the convention behind a choice |
| [`plans/README.md`](plans/README.md) | historical plans and their recorded statuses, not fresh authorization | you want to trace earlier scope |
| [`../audit/`](../audit) | the dated assessment: usability review, defect ledger F00–F19, port plan, verification log, reference comparison, roadmap | you want to know what is broken or what to do next |

## Historical record

| Folder / file | What lives there | Read it when |
|---|---|---|
| [`history.md`](history.md) | the milestone timeline and acceptance notes | you want the project's story in one table |
| [`milestones/`](milestones) | one dev log per milestone: what was built, decisions, bugs found, verification output | you are touching an area and want its history |
| [`research/`](research) | source-level studies of the reference harnesses and the gap analyses (dated evidence, not current status) | you are deciding what to build next |
| [`handoff/`](handoff) | archived handoff prompts from earlier milestones (historical, not active tasks) | you are tracing how a slice of work was delegated |
| [`glossary.md`](glossary.md) | the vocabulary — harness, turn, tool call, compaction, lease, … | you hit an unfamiliar term |
| `../references/` | the four reference harnesses' sources (not committed; research input) | you want to check a port against its origin |

## Reading paths

**New to coding agents?** [`qa/06-agent-harness-primer.md`](qa/06-agent-harness-primer.md) with
[`glossary.md`](glossary.md) open beside it, then the
[architecture guide](design/epsilon-architecture.md).

**New to the codebase?** Root README → [`design/epsilon-architecture.md`](design/epsilon-architecture.md)
→ the crate that owns what you are touching → its milestone log in `milestones/` → the tests in
that crate.

**Reviewing a change?** `AGENTS.md` (gates), then the ADR log
([`design/epsilon-decisions.md`](design/epsilon-decisions.md)) for the behaviour it changes,
then the milestone log that introduces it.

**Deciding what to build next?** [`../audit/02-defect-ledger.md`](../audit/02-defect-ledger.md),
[`../audit/03-port-plan.md`](../audit/03-port-plan.md) and
[`../audit/04-roadmap.md`](../audit/04-roadmap.md), then the relevant study in `research/`.

## Conventions

- `qa/` files are numbered sequentially and never renumbered; add the next number.
- `milestones/M<N>-<slug>.md` is written once, when the milestone lands, and is a record — it
  is not edited afterwards except to correct a factual error.
- Current product facts have **one owner**: the guides in `user/`. Milestones, research and
  handoffs are historical records — their status claims describe the moment they were written,
  not today. Current defects and verification evidence live in `../audit/`.

Product reference material lives in `user/`. Existing moves, instruction corrections,
verification limits and the Phase 2 review handoff are recorded in
[the restructure record](../audit/08-restructure-record.md). Those moves predate this
phase; the verification log is not their move map.
