# 10 — Repository layout: docs, audit, scripts

Captured 2026-09-15 (M63) after a reorganization pass. Read this before moving files again.

## The question

Where does a reader start, and where does each kind of writing belong?

## Decision

```
README.md          the product entry point: what it is, install, quick start, contents
AGENTS.md          the rules for writing code here
CHANGELOG.md       user-visible changes, newest first
LICENSE            MIT
audit/             reviews, defect ledgers, port plans, roadmaps — dated snapshots
docs/              everything else, indexed by docs/README.md
  design/          architecture, ADR log, skills design
  history.md       milestone timeline
  milestones/      one dev log per milestone (a record; written once)
  qa/              convention notes, numbered sequentially, never renumbered
  plans/           implementation plans in execution order
  research/        studies of the reference harnesses and gap analyses
  glossary.md      vocabulary
  handoff/         task handoffs
agents/            agent working memory: open-work notes, evidence pointers, next steps
.agents/skills/    repo-local skills — the proven procedure for each kind of work
.github/           CI: the full gate battery on Windows, confinement on Linux, MSRV check
assets/            the logo the root README embeds
scripts/           gate.sh and gate.ps1 (the verification battery, one per platform),
                   console-probe.py (drives the built TUI in a real console window),
                   rpc_probe.py (headless client smoke test), regen_catalog.py
crates/            the 12 workspace crates (eleven from ADR-014; epsilon-lsp joined in M79)
references/        the four reference harnesses — research input, never committed
```

Rules that follow from it:

- **The root holds entry points only.** Three markdown files and a `LICENSE`; everything else is a
  directory with one job (`crates/`, `docs/`, `audit/`, `scripts/`, `agents/`, `assets/`, and the
  dot-directories holding CI and the repo-local skills). Anything with a "when you need it"
  audience lives under `docs/`.
- **`audit/` is a dated snapshot, not a living document.** It is allowed to name defects that are
  still open and to contradict the code, because its value is that it records what was true at a
  point in time. New audits get new files; old ones are not edited into agreement with the code.
- **A milestone log is history.** It is written when the milestone lands and corrected only for
  factual errors. It is never rewritten to describe later work.
- **Plans execute once.** A plan that has been implemented stays where it is as a record of the
  scoping decision; new work gets a new numbered plan.
- **`docs/research/` is study material**, not a plan of record. It may describe mechanisms the
  project decided *not* to adopt.
- **Scripts stay runnable and small.** `gate.sh` must run all five gates; `rpc_probe.py` must
  exit non-zero when the RPC surface is broken. Both are executable documentation.

## Why not put everything under `docs/`?

The four root markdown files are the ones a hosting page or a fresh checkout shows first. Moving
them into `docs/` would hide exactly the files a newcomer needs. Everything else moved in.

## Cross-reference hygiene

Moving directories broke three links (two in `docs/glossary.md`, one in `docs/history.md`) that
pointed at `../design/...` from one level under `docs/`. Lesson: after any move, run a link check
over every `*.md` — parse markdown link targets, skip `http(s)://` and `#anchor` ones, resolve the
rest relative to the file that contains them — before committing. The checker is ten lines of
Python and finds what grepping for the old path does not, because most references in this repo
are prose (`` `docs/design/…` ``) rather than links.
