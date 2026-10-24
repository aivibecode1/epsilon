# Subagents

A **subagent** is a child agent that Epsilon's `task` tool starts in its own
context. A **subagent role** is the reusable definition that says what such a
child is: a name, a system prompt and a tool subset. The built-in roles live in
`crates/epsilon-core/src/builtin_subagents/`; enabling one writes an ordinary
agent definition under `<config dir>/builtin-agents/`.

This is the same split DeepSeek Harness uses: the model-facing delegation tool is
`subagent`, and a child is addressed as a subagent, not as a "harness". "Harness"
in Epsilon means the whole program.

## Built-in subagent roles (opt-in)

None is installed until you ask. `/subagent` opens the **subagent browser**: the
four rows carry what each role does and why it is useful; `up`/`down` moves the
selection, `Enter` puts the install command on the prompt, and `Esc` closes.
Enable one with:

```text
/subagent explore on
```

Disable it with `/subagent explore off`. An edited copy is left in place and
reported, like a built-in skill. The choice takes effect on the next turn.

| Role | What it does | Why it is useful | Tools |
|---|---|---|---|
| `explore` | Read-only reconnaissance: maps the repository, traces a path, returns evidence and unknowns. | Gives the parent a child that cannot edit the tree. | `read`, `grep`, `find`, `glob`, `ls`, `lsp`, `todo` |
| `review` | Two-stage review: spec compliance first, then implementation quality; reports `file:line`. | Makes the independent second reader a first-class role. | `read`, `grep`, `find`, `glob`, `ls`, `lsp` |
| `verify` | Runs the project's own build and test commands and reports exact results. | Separates proving a change from making it. | `read`, `grep`, `find`, `glob`, `ls`, `bash` |
| `implement` | One scoped change, with a failing test first and a report of what was not verified. | A worker role for delegated edits inside the parent's permission policy. | all tools |

"The tools" is a **subset**, not a second permission layer. The child still passes
the parent's permission gate for every call; a tool absent from the list is not
offered at all, and a tool present in the list can still be refused.

## Using one

Ask for it in plain language:

```text
Use the review subagent on the current diff.
```

or let the model choose from the `task` tool's list. A delegated child has its own
session and depth; it inherits the parent's skills, overlays and model unless the
definition names another model. `/tasks` shows running children and their state.

If a built-in role does not fit, write your own Markdown file with `name`,
`description`, optional `tools`/`model`, and a body that is the child's system
prompt. Put it in `~/.config/epsilon/agents/` (user) or `.epsilon/agents/` in a
trusted project.

## How this differs from a skill

A skill is advice loaded into the current conversation. A subagent is a separate
agent with its own context, system prompt and tool subset. Use a skill when the
model needs a procedure; use a subagent when the work should be isolated, bounded,
or delegated.

## Related

- [Skills](skills.md) — opt-in procedures for the current agent.
- [Extension points](../10-EXTENSION-POINTS.md) — tools, MCP and process extensions.
- [Command reference](../07-COMMANDS.md) — `/subagent`, `/tasks`.
