# The skills in `.agents/skills/`

Eight skills live here. Six of them were **deleted on 2026-09-28 and restored on
2026-10-02** — this file records both decisions and the evidence behind them, so neither is
"helpfully" reversed by an agent who has not read the reasoning.

## What is here

| Skill | What it is | Why it earns its place |
|---|---|---|
| `improve` | A read-only codebase survey that produces a prioritised plan for another agent | A real, reusable procedure, and it correctly refuses to edit source. 45 KB. |
| `ratatui-tui` | A general ratatui reference — layout, widgets, visual system, architecture | Not an Epsilon procedure, but a good one. `references/ai-harness.md` in particular is the most directly useful file in the directory. 67 KB. |
| `ponytail` | Reuse-before-build, stdlib-before-dependency, smallest-change discipline | Restored by owner instruction. See the reversal below. |
| `ponytail-review` | Review a diff for unnecessary complexity only | Same. Routes to a normal review for correctness/security/performance. |
| `ponytail-audit` | The same, over a whole repository | Same. |
| `ponytail-debt` | Harvest `ponytail:` comments into a deferral ledger | Same — with a caveat that matters here. |
| `ponytail-gain` | A one-shot scoreboard of Ponytail's own benchmark medians | Same. Its output is about Ponytail, not about Epsilon. |
| `ponytail-help` | A quick-reference card for the Ponytail modes | Same. |

## Why they were deleted the first time — and why that was, and was not, right

The 2026-09-28 deletion was a considered judgement, not housekeeping, and the reasoning is
recorded below because **two thirds of it still holds.**

**`ponytail-gain` renders a scoreboard for benchmarks this repository does not have.**
Its own text: *"The figures are the published benchmark medians (5 everyday tasks: email
validator, debounce, CSV sum, countdown timer, rate limiter; three models: Haiku, Sonnet,
Opus). … Source: `benchmarks/` and the README."* There is no `benchmarks/` directory here.
**Still true.** Its output is a marketing card about someone else's measurements, so it is
the one of the six with nothing to do with the code in front of it. It is kept because it
was asked for, not because it earns its place — treat its numbers as unverified claims.

**`ponytail-debt` parses a comment format this codebase does not use** in the strict form —
it expects each `ponytail:` marker to name a `ceiling:` and an `upgrade:`. The nineteen
markers in this repository are prose asides:

```rust
// ponytail: budget-based `{"type":"enabled","budget_tokens":N}` returns
// ponytail: O(rest of file) per read — that scan is what makes the counts exact
```

**Still true**, and it is the practical reason to distrust its output here: run it as
written and it reports a ledger of empty cells. Read it as a reminder to *write* well-formed
markers, not as a ledger of existing debt.

**All six ship slash-command mode switching** — `/ponytail lite|full|ultra`, "stop
ponytail", "normal mode". That is a chat-plugin interaction model. A repository-local skill
cannot switch the host harness's mode, so **the persistence half of the skill does not
function here.** What does function is the part that needs no host cooperation: the ladder,
the review/audit procedure, and the discipline of writing `ponytail:` markers when a
deliberate shortcut has a known ceiling.

**`ponytail-audit` explicitly excludes correctness, security and performance.** A review
that cannot find a real bug is a style pass with a confident name. **Still true, and
important**: this repository has a defect ledger precisely because it takes the opposite
position. A Ponytail audit is a *complement* to it, never a replacement — and this
project's own rule that a skill earns its place by naming a procedure that works here
still stands. Which is why the Epsilon ledger, not the audit, remains the thing that has to
catch the bugs.

## The reversal, and what it was based on

On **2026-10-02** the owner asked for Ponytail to be installed in this project and used as
an engineering discipline throughout the work — explicitly including its repository-wide
audit and its review of changes made. That is an owner decision about which discipline this
project runs under, and it supersedes the earlier judgement about which skills belong here.

It is recorded here rather than applied quietly because the original deletion was made on
recorded reasoning, and a silent reversal would be indistinguishable from an agent that
never read it. The reasoning above is unchanged and still applicable; what changed is that
the owner decided the discipline is wanted **despite** the objections, not that the
objections were wrong.

`audit/07-plan.md` item 0.5 ("delete the six `ponytail-*` skills") is superseded by this
entry.

## Installation

Installed from `https://github.com/DietrichGebert/ponytail` into
`.agents/skills/ponytail*/SKILL.md`, which is a directory DSH scans at project scope. Only
`SKILL.md` is vendored; the upstream repository's `benchmarks/`, `scripts/`, `hooks/` and
`examples/` are not, and no skill here needs them to run.

## Note on `ratatui-tui`

It has uncommitted modifications inherited from a previous session (6 files). Those were
**not** reviewed before that session's work began and are **not** covered by the audit.
Decide whether to keep them on their own merits before committing.