# The skills in `.agents/skills/`

Two skills remain, and this file records why the other six are gone — so the deletion
is not "helpfully" reversed by an agent who has not read the reasoning.

## What is here

| Skill | What it is | Why it earns its place |
|---|---|---|
| `improve` | A read-only codebase survey that produces a prioritised plan for another agent | A real, reusable procedure, and it correctly refuses to edit source. 45 KB. |
| `ratatui-tui` | A general ratatui reference — layout, widgets, visual system, architecture | Not an Epsilon procedure, but a good one. `references/ai-harness.md` in particular is the most directly useful file in the directory. 67 KB. |

## What was removed, and the evidence

Six `ponytail-*` skills were deleted on 2026-09-28. They belong to a different project.
The evidence is in the files themselves, not in a judgement call:

**`ponytail-gain` renders a scoreboard for benchmarks this repository does not have.**
Its own text: *"The figures are the published benchmark medians (5 everyday tasks: email
validator, debounce, CSV sum, countdown timer, rate limiter; three models: Haiku,
Sonnet, Opus). … Source: `benchmarks/` and the README."*

There is no `benchmarks/` directory in Epsilon. The tasks are not coding tasks. A skill
whose only output is a marketing card for someone else's measurements is not a
procedure.

**`ponytail-debt` parses a comment format this codebase does not use.** It greps for
`ponytail:` markers and expects each to name a `ceiling:` and an `upgrade:`. Nineteen
`ponytail:` comments exist here, and they are prose asides:

```rust
// ponytail: budget-based `{"type":"enabled","budget_tokens":N}` returns
// ponytail: O(rest of file) per read — that scan is what makes the counts exact
```

Running the skill as written would produce a ledger of empty cells.

**All six ship slash-command mode switching** — `/ponytail lite|full|ultra`, "stop
ponytail", "normal mode". That is a chat-plugin interaction model, not something a
repository-local skill can implement.

**`ponytail-audit` explicitly excludes correctness, security and performance.** A review
that cannot find a real bug is a style pass with a confident name. This repository has a
defect ledger precisely because it takes the opposite position.

The general principle, which is the reason this file exists: **a skill earns its place
by naming a procedure that works in this repository.** Six of eight did not, and their
presence was worse than their absence — they read as rigour, so an agent would follow
them and produce confident nonsense.

## Note on `ratatui-tui`

It has uncommitted modifications inherited from a previous session (6 files). Those were
**not** reviewed before this session's work began and are **not** covered by the audit.
Decide whether to keep them on their own merits before committing.
