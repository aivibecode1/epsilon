# Skill discovery, catalogue budget, and built-ins

This is the design note behind [`docs/user/skills.md`](../user/skills.md). It
records why skills are shaped this way, not how to write one.

## What a skill is

A directory with a `SKILL.md`, or a flat `<name>.md` file, carrying frontmatter
(`name`, `description`, optional `disable-model-invocation`) and a Markdown body.
The Agent Skills shape is shared by Claude Code, Codex, Gemini CLI, Cursor,
Windsurf, opencode, Pi, DeepSeek Harness and Epsilon. Discovery reads every one
of those roots, so a skill written for another harness usually works here.

Epsilon reads one level below each root. It does not recurse into monorepo
subdirectories, and it does not read plugin trees, because those are installed
software with a different trust story. The divergence is stated in
`crates/epsilon-core/src/skills.rs`, not hidden.

## Progressive disclosure

The model sees only the catalogue: one line per skill, `name: description`.
The body loads when the model calls the `skill` tool, or when a user chooses it.
This keeps a large installed skill collection from consuming context on every
turn. It also means the description is the whole advertisement: a vague
description makes the skill invisible in practice.

The catalogue has a budget rule from the reference studies: keep the bundled
catalogue under roughly 1,500 tokens of name-and-description text. Epsilon ships
twelve built-ins; their descriptions are short enough that enabling all twelve
still leaves room for a project's own skills. The catalogue budget grows with the
selection, so adding a skill means removing one or showing it earns the row.

## Discovery rules

- **Name:** lowercase `a-z`, `0-9`, hyphens; at most 64 characters; no leading,
  trailing, or doubled hyphen.
- **Description:** required, one line, at most 1,024 characters.
- **Frontmatter:** a small flat parser, not YAML. Block scalars and quoted
  scalars are handled; unknown keys are ignored.
- **Collisions:** a later root wins. A project skill therefore overrides a user
  skill of the same name. Discovery reports both paths. This is a deliberate
  divergence from Pi, which keeps the first and reports the loser.
- **Failures:** an unreadable root, an unreadable file, an invalid name or a bad
  description is a startup notice, not silence.

## Trust

Project roots (`.epsilon/skills`, `.agents/skills`, `.claude/skills`, and the
rest) are repository-controlled. They load only after the workspace is trusted.
User roots (`~/.config/epsilon/skills`, `~/.agents/skills`, and the other
harness conventions) always load: they are the user's own files, in the user's
home directory.

## Opt-in built-ins

Epsilon compiles twelve skills into the binary, in
`crates/epsilon-core/src/builtin_skills.rs` with assets beside it. They are
**off until enabled**:

- `[off]` rows are shown by `/skills` with their description and a sentence on
  why the skill is useful.
- `/skill <name> on` writes `<config>/builtin-skills/<name>/SKILL.md`, plus any
  files the body refers to (the `improve` skill carries three reference pages).
- Discovery then treats it as an ordinary skill; there is no second loader.
- `/skill <name> off` removes the copy only if every owned file is byte-identical
  to the bundled text. An edited copy is the user's work and is left in place
  with a message; a missing bundled file is repaired on the next enable.

Why this shape rather than a second registry:

1. **One catalogue, one loader.** Compiling skills into the binary but keeping a
   separate runtime path for them would create two answers to "which skills
   exist". Writing an ordinary file reuses every existing rule.
2. **The user's directory stays theirs.** The built-in root is separate from
   `~/.config/epsilon/skills`, so removal cannot touch a skill the user wrote or
   edited.
3. **Off means absent from the prompt.** An enable-list would silently disable
   existing discovered skills when a config file gained a `[skills]` section;
   file presence avoids that. Built-ins are the only population that starts
   absent, and that is what "opt-in" means.

The category is deliberately small: procedures that are stable across projects,
cheap to describe, and expensive to improvise differently every time. A skill
that only repeats generic advice is not bundled. See
[`docs/user/skills.md`](../user/skills.md) for the list and the selection
rationale.

## Attribution

Most built-ins are written for Epsilon, informed by the reference harnesses and
by the skills under `.agents/skills/`. **Ponytail** by DietrichGebert and
**improve** by shadcn are adapted from their MIT-licensed upstream skills; the
`credit` field on [`BuiltinSkill`](../../crates/epsilon-core/src/builtin_skills.rs)
carries that attribution into `/skills`, and the README credits name the
projects. REA's workflow is credited in `docs/references/rea.md` and in the skill
text itself.
