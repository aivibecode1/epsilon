# Design Q&A — Documentation Organization (README & AGENTS.md)

> Captured 2026-08-23, after the question "How many README.md will we have?" exposed that two
> convention decisions lived only in conversation, not in writing.

## The questions that prompted this note

1. Shouldn't AGENTS.md help agents working ON this repo, not just describe the product?
2. How many README.md files do we end up with? (Practice: one, at the root.)

## Decisions

### Exactly ONE product README.md — at the repository root

> **Amended 2026-09-17:** navigation READMEs are allowed, including `docs/README.md` (the docs map, with
> task-oriented reading paths) and product reference guides live in `docs/user/`. The
> amendment: the root README remains the only *product entry point*; `docs/README.md` is
> navigation for the documentation tree, not a competing product description; `user/` guides
> hold current product facts so the root README stays short. The original decision below is
> historical — including its "eleven crates" wording (twelve since M79).

- `docs/README.md` was **deleted**: it answered "where do I start?" in competition with the
  root README. Its unique content (reading order per audience) lives in the root README's
  *How to read this repository* section.
- **No per-crate READMEs.** The eleven crates (twelve since M79's `epsilon-lsp`) are
  workspace-internal path dependencies, never
  published individually; their documentation home is rustdoc (`lib.rs` headers) plus the crate
  table in the root README.
- **Trigger condition for revisiting**: publishing a standalone crate (e.g. `epsilon-contracts`
  to crates.io) earns that crate its own README at that moment. Not before.

### Two AGENTS.md jobs, two homes

| Job | Home |
|---|---|
| Development contract for humans/agents writing Epsilon's code | root `AGENTS.md` (loads automatically into coding-agent context) |
| Runtime behavior contract of the Epsilon product | `crates/epsilon-core/assets/agent-contract.md`; embedded via `include_str!` when the M7 prompt tier lands |

Rationale: pi keeps the same split — root AGENTS.md = repo rules; system prompt assembled in
code and shipped beside the binary. Conflating them meant the repo's own development rules
(commands, gates, laws) existed nowhere.

### Documentation must move with the code (the rule this note institutionalizes)

See AGENTS.md § *Documentation while building*: history-timeline rows in `docs/history.md`,
qa notes for design conversations, ADR amendments, and milestone logs are part of "done",
not an optional afterthought. (Since the 2026-08 README restructure, milestone tables
live here and in `docs/history.md`, not in the root README — that file stays
reader-first.)
