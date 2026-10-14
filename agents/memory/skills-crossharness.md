# Cross-harness skill discovery

**Status: landed.** Battery `skills-crossharness-final-20261013` — 15/15 legs green,
2073 passed / 0 failed / 0 ignored.

## What changed

`SKILL.md` directories are an open convention, not an Epsilon one. Discovery read two
roots (`~/.config/epsilon/skills`, and the workspace's `.epsilon/skills` +
`.agents/skills`) and nothing else, so a user who had been running Claude Code, Codex or
Gemini CLI had skills on disk that this harness never saw — and was told "no skills
discovered".

The root table now lives in **`epsilon_core::skills`**:

* `user_roots()` — `.config/epsilon`, `.claude`, `.codex`, `.gemini`, `.agents` under the
  home directory.
* `project_roots(workspace)` — `.epsilon`, `.claude`, `.codex`, `.gemini`, `.cursor`,
  `.windsurf`, `.opencode`, `.dsh`, `.agents` under the workspace.
* `all_roots(workspace, project_trusted)` — the user tier always, the project tier only
  when trusted, deduplicated.

## Why the table is in `epsilon-core` and not `epsilon-runtime`

Two front ends need it: the composition path (`build_runtime`) and the TUI's `/reload`.
`epsilon-tui` cannot depend on `epsilon-runtime` — that edge would be a cycle in review
terms, the runtime is a *dev*-dependency of the TUI, and the shipped binary must not reach
into the thing that composes it. A table owned by runtime would have to be copied into the
TUI, which is the second-copy-of-composition defect `AGENTS.md` §2.2 names. What stays in
runtime is the genuinely compositional part: which launch this is, and whether it trusts
the workspace.

## Precedence

`discover` lets the **later** root win a name collision (a recorded divergence from pi,
which keeps the first). So the order is: user tier, then project tier; and inside a tier
the portable `.agents` alias is last so it wins — Gemini CLI's documented rule, followed so
one skill tree resolves the same way in every tool that reads it.

## Three defects found and fixed on the way

1. **`trust_requiring_resources` named one of the nine project roots.** A repository
   shipping `.claude/skills` produced a workspace the trust gate called clean: no prompt,
   and then no skills either, with nothing on screen to say why. Now the gate iterates
   `project_skill_roots`.
2. **The TUI's `/reload` skipped the trust gate** that every sibling path applies. It has a
   `!self.trust_review_pending` flag available and the agents reload right below it passes
   it; skills did not.
3. **The scope label looked up the wrong key.** `skill_lines` asked the scope map for the
   skill's own directory while the map is keyed by the root, so `get` returned `None` for
   every skill ever discovered and the `(user)` / `(project)` labels never rendered. This
   was my own uncommitted addition from an earlier turn, caught only when the frame test
   ran.

## Duplicate handling

Two levels. `dedupe_roots` collapses two spellings of one directory (running Epsilon in your
home directory otherwise puts `~/.claude/skills` in both tiers, and every skill would be
reported as shadowing itself, on every launch). `discover` already dedupes by skill name.

## Deliberately not scanned

Documented in the module comment in `skills.rs`: nested monorepo directories,
command-line-added roots, plugin skills, and rules files that are not skills
(`.clinerules`, `.cursor/rules`, `CONVENTIONS.md`, …). Each is a real feature somewhere and
each is out of scope rather than overlooked.

## Evidence

* `audit/evidence/skills-crossharness-final-20261013.log` — the battery.
* `audit/evidence/skills-crossharness-frame-20261013.log` — the `/skills` frame at 100×120,
  80×24 and 100×30. On this machine 27 skills from `~/.gemini/skills` and
  `~/.agents/skills` now load and are labelled `(user)`; the workspace's own is `(project)`.

## Disclosed

The first battery for this slice (`gate-gate-skills-crossharness-20261013`) went red on
clippy — `needless_borrow` on `&user.path()` — and I edited the source while it was still
running, so that log describes a mixed tree. It is not cited as evidence. The two rounds
after it are.
