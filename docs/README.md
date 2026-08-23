# Epsilon Documentation Map

Reading order for humans and agents joining this project.

## 0. Start here

1. `AGENTS.md` (repo root) — the agent behavior contract; also the spec for Epsilon's own
   future runtime behavior.
2. `docs/qa/` — narrative design Q&A captured from real conversations. Read in number order:
   - `01-why-rust.md` — language/runtime rationale, honest costs included
   - `02-lego-brick-methodology.md` — how the system is built and debugged brick by brick
   - `03-partial-vs-full-port.md` — what exists after each milestone phase
   - `04-finished-harness-vision.md` — the destination: UX + full inheritance map

## 1. Evidence base (Phase 1-2 research)

- `reports/harness-analysis.md` — source-level audit of pi / DSH / Hermes / opencode:
  architecture, strengths, weaknesses, adoptable patterns, anti-patterns.
- `reports/comparative-analysis.md` — cross-harness comparison table, best-of verdicts,
  reuse shortlist and anti-shortlist.

## 2. Design (Phase 3)

- `design/epsilon-architecture.md` — components, data flow, module boundaries, v1 exclusions,
  verified dependency versions.
- `design/epsilon-decisions.md` — ADR log. Every major choice with alternatives, rationale, and
  an honest "is this truly best?" answer. ADR-017 holds the build order.
- `design/epsilon-skills.md` — bundled/referenced/excluded skill catalog.

## 3. Build records

- `docs/milestones/M*.md` — one dev log per implemented milestone: what was built, key code
  decisions made while coding, gotchas hit, how to verify.

## Conventions

- ADR = decision + evidence; QA notes = the reasoning conversation behind decisions;
  milestone logs = applied reality. When they disagree, newest wins and the older file gets a
  superseding note.
