# Plan 011: Refresh docs and README to m31 reality

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `docs/plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat abd045a..HEAD -- README.md docs/history.md docs/glossary.md docs/design/epsilon-architecture.md docs/design/epsilon-decisions.md CHANGELOG.md`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: docs
- **Planned at**: commit `abd045a`, 2026-08-27
- **Issue**: —

## Why this matters

`abd045a` (m31) landed three user-visible changes — `/compact`/`/timeline`/`/fork`,
`MEMORY.md` in the prompt tiers, and opencode-parity wildcards — but
`docs/history.md` still ends at M28, `README.md` still claims the pre-m31
tool/slash set, and `docs/design/epsilon-architecture.md` was only hot-patched for
webfetch. A newcomer reading the docs learns a stale harness, and a returning
contributor cannot tell what `abd045a` shipped. Stale docs are worse than
missing docs — this plan makes the docs match the code with the smallest
honest edits.

## Current state

Relevant files, each with one line on its role:
- `README.md` (210 lines) — reader-first quick start, crate table, credits, doc map; currently claims 249 tests / "read/write/edit/bash/grep/glob/ls" tool set, slash set without /compact/timeline/fork
- `docs/history.md` (88 lines) — milestone timeline M0..M28 + ADR index; M29/M30/M31 rows missing
- `docs/glossary.md` (222 lines) — term definitions; missing Compact/Timeline/Fork, MEMORY.md tier, wildcard parity terms
- `docs/design/epsilon-architecture.md` (429 lines + 2 hot patches) — system overview, crate table, agent loop, tool/safety/session/compaction/prompt/provider sections; §4 tool table and §4 concurrency note already say "webfetch deferred"
- `docs/design/epsilon-decisions.md` (574 lines, 18 ADRs) — ADR-005/007/008 cover safety/compaction/prompts; no ADR amendment for m31's MEMORY.md additive tier
- `CHANGELOG.md` (335 lines) — Unreleased entries through M28, no M29/M30/M31 entries

Excerpts as of `abd045a`:
```md
// README.md Quick start — stale tool list
Epsilon reads, writes, edits, greps, globs, lists, and runs shell commands
inside its declared workspace.
```

```md
// docs/history.md — ends at M28
| M28 | Custom OpenAI-compatible providers (`[provider_custom.<name>]`, `<name>/<model>` ids) |
```

```rust
// crates/epsilon-core/src/prompt.rs — MEMORY.md tier landed in m31 (evidence docs must cite)
// load_project_instructions now reads ~/.config/epsilon/MEMORY.md + workspace/MEMORY.md additive
```

```rust
// crates/epsilon-safety/src/wildcards.rs — opencode-parity regex (evidence)
// '*' -> ".*", '?' -> ".", '\' -> '/', meta-escaped, ** no-op collapsed
```

Repo conventions to honor:
- History rows are terse: `| Mxx | One-line scope |` — see `docs/history.md:10-47`.
- README stays reader-first: quick start → crates → mechanisms → testing → doc map → credits. Milestone tables live only in `docs/history.md` + `CHANGELOG.md` (per `docs/qa/05-documentation-organization.md`).
- One README at root, no per-crate READMEs (ADR-014 + qa/05).
- Human tone: `humanizer` skill — no generic AI prose, no marketing, no em-dashes, no empty claims; docs teach, then describe.
- ADRs are append-amend, never rewrite history — add an "Amendment (M31)" paragraph under the affected ADR.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Check | `cargo fmt --all -- --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets` | exit 0, 0 warnings |
| Tests | `cargo test --workspace --lib 2>&1 \| tail -n 20` | all pass (reference counts only, not a gate for docs) |

## Suggested executor toolkit

- Skills: `humanizer` for README/glossary prose; `improve` docs category only for staleness checks.
- References: `docs/design/epsilon-architecture.md` §1-3 for crate table wording; `docs/milestones/M28-custom-providers.md` as the last history-row exemplar.

## Scope

**In scope** (the only files you should modify):
- `README.md`
- `docs/history.md`
- `docs/glossary.md` (add 3–5 glossary entries only)
- `docs/design/epsilon-architecture.md` (one paragraph in §7 or §8 for MEMORY.md tier; crate table stays 11 rows)
- `docs/design/epsilon-decisions.md` (one Amendment paragraph under ADR-008 or ADR-012 for MEMORY.md)
- `CHANGELOG.md` (one Unreleased subsection for M29/M30/M31 — keep existing M0..M28 untouched)
- `docs/milestones/M29-*.md`, `M30-*.md`, `M31-*.md` (create only if missing and only 30–60 lines each — keep them short, per `docs/milestones/M27-catalog-drift-ci.md` as exemplar)

**Out of scope** (do NOT touch, even though they look related):
- Any Rust source (`crates/**`) — code is frozen for this plan.
- `docs/research/**` — generated analysis, not hand-edited.
- `docs/plans/**` other than this plan's own execution.
- `docs/qa/**` beyond at most one cross-link fix — deep rewrites are a separate milestone.

## Git workflow

- Branch: `advisor/011-docs-refresh-m31`
- Commit style: `docs(m31): refresh README/history/glossary/architecture to shipped reality` (see `git log --oneline` — docs commits use `docs:` prefix)
- One commit for all doc edits; push only when operator instructs.

## Steps

### Step 1: Confirm drift and collect shipped facts

Run drift check. Then collect the three facts the docs must state:
- Slash set now includes `/compact`, `/timeline` (alias `/branch`), `/fork` plus existing 8 (see `crates/epsilon-tui/src/picker.rs:212 SLASH_ITEMS`).
- Prompt CONTEXT tier now includes `MEMORY.md` (global `~/.config/epsilon/MEMORY.md` + workspace `MEMORY.md` additive with `AGENTS.md`) — see `crates/epsilon-core/src/prompt.rs` `load_project_instructions`.
- Wildcards now opencode-parity regex (`*`→`.*`, `?`→`.` escaped, `\`→`/`, `**` collapsed) via `regex` crate — see `crates/epsilon-safety/src/wildcards.rs`.

**Verify**: `grep -n "SLASH_ITEMS\|Command::Compact" crates/epsilon-tui/src/picker.rs | head` shows Compact/Timeline/Fork; `grep -n "MEMORY.md" crates/epsilon-core/src/prompt.rs | head` shows the tier.

### Step 2: Update README.md (minimal honest edits)

Edit only these spots:
- Quick-start "Eleven crates" table: `epsilon-tools` row still lists 7 tools — keep it, but ensure it matches `docs/design/epsilon-architecture.md` deferred wording (no new tool claimed).
- "Mechanisms worth knowing" list: add one bullet `**Slash-navigable sessions**: /timeline lists branches, /fork copies JSONL to a new branch file (m31) — keeps session trees observable without leaving the TUI.`
- "Documentation map" table: keep rows, ensure history row points to `docs/history.md` which you will update next.
- Credits table: keep as-is (no new harness needed).
- Do NOT change test counts unless you ran `cargo test --workspace --lib` and have the number — leave the 249 line or replace with "see docs/history.md for suite totals" to avoid a stale number.

**Verify**: `cargo fmt --all -- --check` → exit 0 (md files don't gate fmt, but keep it clean).

### Step 3: Update docs/history.md

Append after M28:

```
| M29 | Steering queue badge + drain-on-abort restore (TUI polish) |
| M30 | Todo tool + checklist rendering; edit diff preview + TUI unified diff |
| M31 | Reference gap closure: /compact, /timeline, /fork, MEMORY.md tier, wildcard parity, webfetch deferred honestly |
```

Keep the "Nothing is currently scheduled under future" coda.

**Verify**: `grep -c "^| M" docs/history.md` increments by 3.

### Step 4: Update docs/design/epsilon-architecture.md and decisions

- In `docs/design/epsilon-architecture.md` §8 (Prompt construction): add one sentence after the tier list: `Workspace and global MEMORY.md files (when present) are injected as CONTEXT-tier blocks alongside AGENTS.md, first-match-wins within tier, additive across tiers (m31, Hermes memory pattern).`
- In `docs/design/epsilon-decisions.md` under ADR-008 (or ADR-012 if you prefer), add: `**Amendment (M31).** CONTEXT tier extended with MEMORY.md (global + workspace) — additive, hash-tagged, collapsible-if. Deferred webfetch stays rule-only until 010 ships.`

**Verify**: `grep -n "MEMORY.md" docs/design/epsilon-architecture.md docs/design/epsilon-decisions.md` shows both hits.

### Step 5: Update docs/glossary.md

Add 4 entries (alphabetical within group, keep grouping):

- **Compact** — Manual `/compact` reclaims context by summarizing older ranges; complements auto-compact at 80%.
- **Timeline** — `/timeline` (alias `/branch`) lists `branch_entries()` with lane/snippet; selection navigates via `navigate_to`.
- **Fork** — `/fork` copies the current JSONL to `*-fork-<id>.jsonl` preserving the tree.
- **MEMORY.md** — Durable project/global memory file injected into CONTEXT tier; survives compaction.

Keep each entry to 2 lines max, with a `See` pointer to `docs/design/epsilon-architecture.md` § where relevant.

**Verify**: `grep -n "Compact\|Timeline\|Fork\|MEMORY" docs/glossary.md` shows 4 new hits.

### Step 6: Update CHANGELOG.md

Under `## [Unreleased]` prepend (do not disturb M28..M0 sections):

```md
### M31 - Reference gaps G-M1..G-M5
- **/compact, /timeline, /fork** (`picker.rs` + `app.rs` run_command): manual compaction, branch timeline picker, JSONL fork.
- **MEMORY.md tier** (`prompt.rs`): global + workspace memory files in CONTEXT tier, hash-versioned, collapsible-if.
- **Wildcard parity** (`wildcards.rs` + `regex` dep): opencode `wildcard.ts` parity (`*`→`.*`, `?`→`.`, `\`→`/`, meta-escaped).
- **webfetch deferred honestly** (`docs/design/epsilon-architecture.md`): rule exists, tool ships in plan 010.

### M30 - Todo + diff polish
- Todo tool + checklist rendering; edit diff preview + TUI unified diff.

### M29 - Steering queue polish
- Steering queue badge + drain-on-abort restore to editor.
```

Keep each bullet to one line plus optional second for file anchors.

**Verify**: `head -n 40 CHANGELOG.md` shows M31 at top of Unreleased.

## Test plan

No new Rust tests — docs-only plan. Verification is file-level:
- `grep -rn "M31\|MEMORY.md\|SLASH_ITEMS" README.md docs/history.md docs/design/epsilon-architecture.md | head` shows expected hits.
- `cargo fmt --all -- --check` → exit 0.
- `cargo clippy --workspace --all-targets` → 0 warnings (unchanged).

## Done criteria

Machine-checkable. ALL must hold:
- [ ] `grep -c "^| M31" docs/history.md` == 1 and `grep -c "^| M29" docs/history.md` == 1
- [ ] `grep -q "MEMORY.md" docs/design/epsilon-architecture.md` and `grep -q "M31" docs/design/epsilon-decisions.md`
- [ ] `grep -q "Compact\|Timeline\|Fork" docs/glossary.md`
- [ ] `grep -q "M31" CHANGELOG.md`
- [ ] `cargo fmt --all -- --check` exits 0
- [ ] No files outside the in-scope list are modified (`git status`)
- [ ] `docs/plans/README.md` status row for 011 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:
- The code at the locations in "Current state" doesn't match the excerpts (drift since `abd045a`).
- `docs/history.md` or `CHANGELOG.md` structure differs from excerpts (different heading levels) in a way that makes the append ambiguous.
- Adding MEMORY.md wording would require touching prompt assembly logic (it shouldn't — docs only).
- A step's verification fails twice after a reasonable fix attempt.

## Maintenance notes

- If plan 010 ships webfetch, revisit `docs/design/epsilon-architecture.md` §4 tool table and README crate table to flip deferred → shipped in one line.
- If a new milestone lands, append one history row and one CHANGELOG subsection — do not rewrite prior rows.
- Reviewers should scrutinize that no doc claims a feature not in `abd045a` (especially test counts).
