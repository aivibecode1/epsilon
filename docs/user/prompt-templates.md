# Prompt templates

A prompt template is a saved prompt you send by name. Type `/review src/parser.rs` and the
model receives the contents of your `review.md` template with `$1` replaced by the argument.

The point is repeatability. The instructions you give a code review, a commit message, or a
release check should be the same every time — not whatever you happened to type.

## Where they live

| Location | Scope |
|---|---|
| `<your config dir>/prompts/` | yours, in every project |
| `<workspace>/.agents/prompts/` | this repository |
| `<workspace>/.epsilon/prompts/` | this repository |

Both project directories are scanned, so use whichever your repository already uses for
skills. If a name appears in more than one place, the project wins — a repository can ship
its own `review` without you editing your personal config.

`/reload` picks up changes you have made since startup.

## Finding one you already wrote

`/templates` lists what is installed, with each template's description and argument hint:

```
prompt templates: /commit — Write a commit message (takes <path>) · /review — Review a file for correctness
```

With nothing installed it says where to put files rather than printing an empty list.

## Writing one

Any `.md` file in those directories becomes a template. The filename is the name:

```
~/.config/epsilon/prompts/review.md
```

````markdown
---
description: Review a file for correctness
argument-hint: <path>
---
Review $1 carefully.

Look for:
- logic errors and unhandled edge cases
- tests that do not cover the change
- anything that would surprise a reader
````

Everything after the frontmatter is sent to the model. The frontmatter is optional; without
it, the first line of the file becomes the description shown in the picker.

## Arguments

| You write | It expands to |
|---|---|
| `$1`, `$2`, … | that argument, or nothing if you did not give one |
| `$@` or `$ARGUMENTS` | all arguments, joined by spaces |
| `${1:-none}` | argument 1, or the word `none` if you left it out |
| `${@:-none}` | all arguments, or `none` if you gave none |
| `${@:2}` | arguments from the second onward |
| `${@:1:2}` | the first two arguments |

Arguments containing spaces stay one argument if you quote them, so
`/commit "fix the parser"` is a single argument.

### The defaults are worth using

`${1:-all}` is what makes a template usable both ways:

````markdown
Search the codebase for $1 and explain what you find.
````

`/search-pattern auth` searches for `auth`. `/search-pattern` alone searches for `all`.

### A literal dollar

Write `$$` for a dollar sign — useful when the prompt is about prices or shell variables:

````markdown
Report anything costing more than $$5 per GB.
````

### Substitution happens once

An argument is inserted literally, even if it contains `$1`. This is deliberate: it means a
pasted shell transcript cannot trigger another placeholder in your template.

## Which wins, a template or a built-in?

Templates are expanded **before** slash commands are parsed, so a template may use a
built-in's name. If your repository has a `help.md`, `/help` sends your text rather than
showing the command list — rename it if you want both.

## Things that do not work

- **Templates cannot be nested.** A template's name is not expanded again in its own output.
- **Arguments are not shell syntax.** No pipes, no `$(...)`, no globbing — arguments are
  split on whitespace, and quotes group. A template body is a prompt, not a command.
- **There is no `--prompt` flag yet**, so a template cannot be expanded from the command
  line; it works in the interactive session only.

## Custom commands

A custom command is a markdown file in a `commands/` directory. It uses the same
file shape and the same argument grammar as a template (`$1`, `$@`,
`${1:-default}`, `${@:2}` — see Arguments above), but the TUI treats the body
differently: it expands the body and resubmits it through the same submit path
as typed text. A body that is plain prose is sent as a prompt; a body that
names a built-in command runs it. A body naming another command or template
file is not expanded again — one expansion per submit, never a chain.

| Location | Scope |
|---|---|
| `<your config dir>/commands/` | yours, in every project |
| `<workspace>/.agents/commands/` | this repository |
| `<workspace>/.epsilon/commands/` | this repository |

Project directories load only in a trusted project, like templates and skills.
`/reload` picks up changes since startup. The popup shows each file with its
scope (`[u]` yours, `[p]` the project's), `/help` indexes the commands, an
unknown `/name` suggests the nearest one, and RPC `get_commands` lists
commands and templates without executing either.

Three rules keep commands safe. First, **a built-in always wins over a
same-named command file**: a `commands/trust.md` is ignored with a warning and
`/trust` runs the built-in, so a repository file can never hijack `/trust`,
`/login`, `/preset` or `/plan`. (Templates keep the older rule from "Which
wins" above — a `prompts/help.md` still shadows `/help` — because changing it
would break existing files; new executable behaviour takes the strict rule.)
Second, **project beats user** on a name clash in both directories. Third,
commands run in the interactive TUI only: headless, HTTP and RPC modes never
expand them, so a `/name` there stays literal text.

What a command file cannot do: no new frontmatter keys (only `description` and
`argument-hint` are read), no shell execution, no file references, no scoping
keys, no recursion into other files.
