# Epsilon Skill Selection

Epsilon implements the Agent Skills spec (ADR-015): a skill is a `SKILL.md` directory whose
frontmatter (name, description) enters the prompt catalog and whose body loads on demand via the
`skill` tool / read tool. This document selects the skills bundled with or referenced by Epsilon
v1, with justifications, and records exclusions.

Selection criteria, derived from Phase 1-2 evidence:

1. **Frequency x generality** - skills pay for their catalog cost only if they encode procedures
   the agent would otherwise improvise inconsistently (Hermes' "Footprint Ladder" reasoning:
   prefer skills over new core tools to keep every-call schema costs flat).
2. **Procedure stability** - content must change slower than model versions; volatile knowledge
   belongs in docs retrieval, not skills.
3. **Safety relevance** - procedures that reduce destructive-action risk earn priority
   (Prime directive 2 support).
4. **Spec compliance** - all skills ship as plain SKILL.md so users can fork/override them in
   project directories.

## Bundled skills (ship with Epsilon)

| Skill | Purpose | Justification |
|---|---|---|
| `commit-craft` | Repo-style-aware commit workflow: inspect status/diff/log, stage explicit paths, message conventions, when-to-refuse | Git discipline violations are the most common agent harm class across all four references; encoding pi's git rules as a procedure makes them loadable exactly when needed instead of always consuming context |
| `test-first-fix` | Bug-fix loop: reproduce -> failing test -> minimal fix -> run targeted tests -> full checks | Encodes the verify-don't-guess working loop; the single highest-value procedure for code quality; mirrors pi's test-harness discipline (suite harness + faux provider pattern) |
| `codebase-onboarding` | Structured first-pass exploration of an unfamiliar repo: manifests, entry points, module map, AGENTS.md discovery | Large-search delegation is a documented subagent strength (opencode task/explore agents); a fixed procedure prevents shallow README-only impressions - the exact failure mode Phase 1 was designed to avoid |
| `refactor-safely` | Mechanical refactors: find-all-usages before rename, keep behavior green, small commits, no drive-by reformatting | Encodes "read files in full before wide-ranging changes" (pi's rule) into a repeatable checklist |
| `dependency-audit` | Adding/upgrading dependencies: check advisories, license, maintenance signals, pin exact, note lifecycle scripts | Both pi (npm incident policy) and Hermes (exact-pin post-incident) converged on this; cargo-deny/cargo-vet equivalents for Rust make it mechanical |
| `doc-driven-answer` | Answering questions about libraries/APIs: locate authoritative local docs first, then vendor sources, cite paths, flag uncertainty | Direct implementation of Prime directive 4; prevents fabricated API claims without requiring web access |
| `session-recap` | Produce handoff summaries from session logs: decisions, state, unfinished work | Leverages Epsilon's JSONL tree format; supports multi-session continuity where compaction summaries alone lose structure |

## Referenced skill packs (documented pointers, not bundled)

| Pack | Why referenced only |
|---|---|
| Language-specific style packs (rust-idioms, ts-migration) | Too opinionated to bundle; teams should override per project; catalog stays small |
| Framework skills (react, axum, tokio patterns) | Volatile content violates criterion 2; better served by doc retrieval + MCP doc servers |

## Explicitly excluded

| Excluded | Reason |
|---|---|
| Browser automation / computer-use skills | No browser tool in v1 surface (architecture section 14); a skill cannot create missing capabilities |
| Data-science/notebook skills | Out of scope for v1 coding focus; NotebookEdit-class tools not built |
| Marketing/writing/content skills | Not coding tasks; generic LLM ability suffices; catalog slots are scarce |
| Self-modification / harness-extension skills | Deliberately excluded until sandboxed self-extension exists post-1.0 (HA-DSH: opt-in even there); bundling one now would advertise capability the permission model should gate explicitly |
| Cron/scheduling/ops skills | No scheduler subsystem in v1 |
| "Personality" skills (SOUL.md-style persona swaps) | Hermes carries identity in stable tier config, not skills; identity-as-skill conflates layers and breaks byte-stable prompts (ADR-008) |

## Catalog budget rule

Total bundled catalog must stay under ~1,500 tokens of name+description text (measured at build
time, CI-enforced). Beyond that, progressive disclosure savings evaporate - the same budget logic
as Hermes' footprint ladder applied to the catalog tier.
