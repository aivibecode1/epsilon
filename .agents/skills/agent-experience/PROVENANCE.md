# Provenance — `agent-experience`

## What this is

A **verbatim copy** of a skill authored by the **DeepSeek Harness** project.

- **Upstream:** `references/deepseek-harness-dsh-v0.2.1-rc.2/packages/preset/agent-preset/skills/agent-experience/SKILL.md`
- **Author / organisation:** DeepSeek — <https://github.com/deepseek-ai/deepseek-harness>
- **Licence:** **MIT**. DSH's `LICENSE` reads "Copyright (c) 2026 DeepSeek".

## What was changed

**Nothing.** The file is byte-identical to upstream. This is a copy, not an adaptation, which
is why attribution lives here rather than in a header in the file.

## Why it earns its place

`AGENTS.md` says nothing about **writing model-facing tool definitions**, and this repository has
a large surface that is exactly that: the built-in tools in `crates/epsilon-tools`, the MCP bridge
in `crates/epsilon-mcp`, and the skill loader in `crates/epsilon-core/src/skills.rs` — plus the
prompt-assembly and compaction layers that decide what the model actually sees.

Its advice is concrete and mostly counter-intuitive, which is the mark of something earned rather
than restated: *delete obvious constraints* (a rule the model learns from the call result costs
tokens); *describe behaviour, not implementation*; *put parameter rules on the parameter*; *say
each fact once*; *prefer bounded outputs and return identifiers for retrieving more*; and *measure
the change* by comparing first-turn prompt tokens before and after.

That last one is the one this repository most needs: we have spent this session making claims
measurable, and this is the same discipline applied to the one surface where we had no number.

## Vetting performed before installing

The Ponytail-gate standard from `AGENTS.md` §0:

- **Instructions only.** Nineteen lines of prose and one list. No shell, no scripts, no code
  execution, no network calls.
- **Relevant and additive.** Fills a documented gap; restates nothing `AGENTS.md` already says.
- **No conflict** with `AGENTS.md`.
- **Licence compatible.** MIT into an MIT project.

## When an agent should reach for it

Writing or changing a **model-facing** surface: a tool name, its description, a parameter schema,
or the system-prompt section that documents it. Also when designing a skill, deciding how a
context window is loaded, or shaping what a tool returns — bounded, with identifiers, rather than a
dump.

## Sibling skills considered and not installed

The same directory holds three Cordis skills (`cordis-composition-reference`,
`cordis-plugin-development`, `editing-cordis-compositions`). All three are about DSH's own plugin
composition format, which has no counterpart here: we compose through Cargo crates and
`epsilon-runtime` (`AGENTS.md` §2 invariant 2). Two of them also instruct
`plugin_manager install_bundle`, which executes plugin code and writes into `$DSH_HOME`. Skipped on
both counts — irrelevance and the body check.