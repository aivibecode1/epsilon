# Provenance — `grill-me`

## What this is

An **adaptation** of a skill that ships with the Hermes Agent reference harness.

- **Upstream:** `references/hermes-agent-2026.9.24/optional-skills/software-development/grill-me/SKILL.md`
- **Author / organisation:** Rafael Zendron (`rafaumeu`), building on the `grilling` mechanic
  from Matt Pocock's `mattpocock/skills`, packaged for Hermes Agent
- **Licence:** **MIT**. Hermes' `LICENSE` reads "Copyright (c) 2025 Nous Research"; the
  upstream frontmatter names the same licence. The same licence is recorded for Hermes in
  this repository's `CREDITS.md`.

`references/` is read-only input. The upstream file is untouched; this is a separate
artefact under `.agents/skills/`.

## What was changed

**Kept.** The mechanic, which is the whole value: model the plan as a design tree, ask
everything answerable now, recompute the frontier, repeat, and synthesise only when the
frontier is empty. Also kept — the coverage branches, the recommendation-with-every-question
rule, and the "write no code during the interrogation" rule.

**Stripped.** Every Hermes tool name and the invocation model built on it:

- `search_files`, `read_file`, `terminal`, `delegate_task`, `execute_code`, `web_extract` —
  replaced with this repository's real tools (`grep`, `read`, `glob`) and with cargo as the
  thing that answers what compiles and what passes.
- The `❓ Q1` / `➡️ Recommendation` answer template, the "Hermes tags" frontmatter, the
  `platforms` and `related_skills` keys, the industry persona tables, and the
  prerequisites section.
- The reference to `delegate_task` for "heavy exploration" — exploration here is reading the
  tree, or asking a subagent for it, and neither changes the procedure.

**Added.**

- The rule stated as a repository rule rather than a pitfall: *a question answerable from
  the workspace is looked up, never asked*. Upstream says this once, in passing, as
  "facts are your job; decisions are the user's". It is the single most load-bearing line
  for an agent running here, so it is step 2 of the procedure.
- The **Branches the tree must cover** block, retargeted: `AGENTS.md` §5 reuse-before-add
  and the ADR rule for behaviour changes, and §2's wire-contract and permission-boundary
  invariants, which are this repository's version of "what happens when it fails".
- The **Test Matrix** body shape, and a **When NOT to use it** section. Upstream has a
  checklist but no matrix.

## Why it earns its place

`AGENTS.md` §12 tells an agent how to brief a subagent. It says nothing about interrogating
a *plan* before it is decomposed or implemented, and the failure this prevents — a plan
whose unresolved branches get decided silently during implementation — is invisible in
review, because by then the decisions look like requirements.

## When an agent should reach for it

Before implementing anything with more than one real decision in it: an auth flow, a schema
or wire change, a migration, a new front-end surface, or any plan the user describes with
words like "somehow", "probably", or "we'll see". Also before splitting work across
subagents, so the subagents inherit settled decisions instead of each making their own.

Not for reviewing code that already exists, and not for a change with one obvious
implementation.