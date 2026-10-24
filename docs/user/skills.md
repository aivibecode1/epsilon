# Skills

A skill is a reusable procedure stored as a `SKILL.md` file. Epsilon puts the
skill's **name and one-line description** in the model's context. The full body
loads only when it is needed: either the model calls the `skill` tool, or you
choose it. That split is why a small catalogue costs little context.

## Built-in skills (opt-in)

Epsilon ships twelve skills compiled into the binary. **None of them is active
until you enable it.** That is deliberate: a procedure that silently enters every
prompt spends context on advice you did not ask for.

Run `/skills` in the terminal IDE to open the **skills browser**. The list shows
built-ins first, then discovered skills; `up`/`down` move the selection and the
detail block shows the description, why it earns a place, its compact scope and
the exact `/skill` command that changes it. `Enter` puts that command on the
prompt; `esc` closes. Enable one with:

```text
/skill verify-before-claim on
```

Disable it with `/skill verify-before-claim off`. Enabling writes a normal
`SKILL.md` under `<config dir>/builtin-skills/<name>/`; disabling removes that
copy only if it is unedited. If you edit the installed copy, Epsilon leaves it
alone and says so — an edited file is your work. The choice is a persisted
default for this user and takes effect on the next turn in the current session.

| Skill | What it does | Why it is useful |
|---|---|---|
| `verify-before-claim` | Checks a claim against code, command output, or a rendered frame before stating it. | A confident unchecked claim is the most expensive failure a coding agent produces. |
| `onboard-codebase` | Maps an unfamiliar repository: entry points, core loop, state, tests, invariants. | Prevents README-level guesses and edits made in the wrong layer. |
| `make-a-change` | The smallest correct change, with a test that fails first, then the real verification command. | Stops drive-by rewrites and unproven fixes. |
| `review-a-change` | Reviews twice: first against the request, then against the implementation. | Catches work that is correct in form but answers a different request. |
| `two-stage-review` | Judges spec compliance first, implementation quality second, with a verdict per finding. | A quality pass over the wrong change is work spent on the wrong problem. |
| `ponytail` | Forces the smallest solution that works; asks whether the thing needs to exist before building it. | Deletes speculative code instead of adding it, on any coding task. |
| `improve` | Surveys a codebase and writes self-contained plans for another executor; never edits source. | Turns an audit into handoff plans a cheaper model can execute. |
| `ux-triage` | Dogfoods a surface as a hard persona and separates friction from defects. | Finds usability problems that correctness tests cannot see. |
| `write-a-commit` | Inspects the diff, stages explicit paths, writes an imperative message, verifies before committing. | A commit outlives the session; a careless one is permanent. |
| `debug-a-failure` | Reproduces, names the layer, proves causation with one variable at a time, then fixes. | Guessing at the wrong layer turns one bug into several. |
| `write-a-handoff` | A compact handoff: objective, state, decisions, changes, verification, open work. | Context windows end; decisions should not vanish with them. |
| `reverse-engineer-anything` | Uses REA's MCP tools for binaries, Electron apps, .NET assemblies and sites, with evidence rules. | Adds real analysis without pretending its findings are certain. |

Most built-ins are written for Epsilon, informed by the reference harnesses in
[`../references/`](../references/README.md) and by the skills in this
repository's own `.agents/skills/`. Two are adapted from MIT-licensed upstream
skills and keep their credit in `/skills` and in the
[README](../README.md#credits): **Ponytail** by DietrichGebert and **improve**
by shadcn.

## Your own skills

Put a directory with a `SKILL.md` in any root Epsilon reads:

```text
<workspace>/.agents/skills/<name>/SKILL.md     # portable, project
<workspace>/.epsilon/skills/<name>/SKILL.md    # Epsilon-native, project
~/.agents/skills/<name>/SKILL.md               # portable, user
~/.config/epsilon/skills/<name>/SKILL.md       # Epsilon-native, user
```

The same discovery also reads `.claude/skills`, `.codex/skills`,
`.gemini/skills`, `.cursor/skills`, `.windsurf/skills`, `.opencode/skill` and
`.dsh/skills`. A skill written for another harness usually works unchanged. A
later root wins a name collision; discovery names both paths when it happens.

The minimal file:

```markdown
---
name: release-notes
description: Turn a range of commits into user-facing release notes.
---

# Release notes

1. Read `git log <from>..<to>`.
2. Group by user-visible change, not by file.
3. Name the reader's benefit in the first sentence.
```

Rules:

- **Name**: lowercase `a-z`, `0-9`, hyphens; at most 64 characters; no leading,
  trailing, or doubled hyphen.
- **Description**: required, one line, at most 1,024 characters. It is the only
  text the model sees before deciding to load the skill.
- **Body**: the procedure. Write it for a capable reader who has the repository
  and not your session.
- **`disable-model-invocation: true`**: the model cannot load it; a user can.

Bundled files (scripts, references) are listed in the skill response, up to ten
names, with the skill directory as the base path. The bodies are not pasted in.

## Scopes and disabling

The browser labels each discovered skill with where it came from (`project`,
`user`, or a path). `/skill <name> off` writes `[skills] disabled` in the global
config; the catalog and the `skill` tool both stop offering it on the next turn.
`/reload` re-reads roots, templates and agent definitions after you add a file.

A project's skills are **untrusted input** until you trust the project config.
That is why a fresh checkout can show the trust prompt before its `.agents/skills`
load. The same rule covers `.claude`, `.cursor`, and every other project root.

## When a skill is the wrong tool

A skill is advice, not a capability. If the work needs a new action (a new API, a
new file format, a new service), it needs a tool, an MCP server, or an extension.
Bundling a skill that tells the model to do something it cannot do is worse than
not bundling it at all.

## Related

- [Extension points](../10-EXTENSION-POINTS.md) — when a skill is not enough.
- [Command reference](../07-COMMANDS.md) — `/skills`, `/skill`, `/reload`.
- [Design note](../design/epsilon-skills.md) — discovery, budgets and the decision
  to make bundled skills opt-in.
