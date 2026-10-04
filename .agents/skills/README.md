# The skills in `.agents/skills/`

Sixteen skills live here. Six of them were **deleted on 2026-09-28 and restored on
2026-10-02** — this file records both decisions and the evidence behind them, so neither is
"helpfully" reversed by an agent who has not read the reasoning.

Three more were added on **2026-10-03** from the reference harnesses in `references/`, after a
four-way read-only survey of the **258** `SKILL.md` files they contain. That survey's reasoning,
including everything rejected and why, is in [§ Additions of 2026-10-03](#additions-of-2026-10-03).

Five further skills were added on **2026-10-03** from that same survey: three adapted from the
reference corpus, and two written from scratch because the survey judged that adapting their
parents would have been worse. They are in the same section.

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
| **`agent-experience`** | How to write a model-facing tool definition so the model finds it and spends context well | **[DeepSeek Harness] Verbatim copy.** `AGENTS.md` says nothing about authoring tool schemas, and we have three surfaces that *are* tool schemas: `epsilon-tools`, `epsilon-mcp`, and the skill loader. |
| **`one-three-one-rule`** | A decision-brief format: one-sentence problem, three genuinely different options, one pick | **[Hermes Agent] Verbatim copy.** ~70 lines of prose. `AGENTS.md` §14 lets engineering judgment override the file but gives no format for *surfacing* that judgment. |
| **`rtl-aware-development`** | Text-direction correctness: logical CSS, mixed-script isolation, RTL scroll, title bars | **[opencode] Verbatim copy.** `AGENTS.md` §8 holds the product to WCAG 2.2 AA and names keyboard, contrast and focus — but says **nothing about direction**, and we already ship an Arabic reading guide. |
| **`grill-me`** | Interrogate a plan in frontier rounds — ask everything answerable now, recompute, repeat | **[Hermes Agent] Adapted** from `grill-me`. The design-tree mechanic kept; every Hermes tool name replaced with this repo's. 83 lines. |
| **`isolate-the-layer`** | A seven-step ladder that finds the failing layer before anyone changes code | **[Hermes Agent] Adapted** from `rest-graphql-debug`. Every `curl` and Python block deleted; retargeted at `epsilon-llm` and `epsilon-server`. |
| **`ux-triage`** | Hostile persona, then a mandatory pragmatism filter into RED / YELLOW / WHITE / GREEN | **[Hermes Agent] Adapted** from `adversarial-ux-test`. Browser tour, screenshots and ticket creation removed — none of them are available here. |
| **`evidence-guardrails`** | Evidence-id discipline for any report someone will act on | **Written here**, distilling the guardrail block of Hermes' `oss-forensics`. 59 lines. |
| **`two-stage-review`** | Spec compliance strictly before code quality, and never the author's own self-review | **Written here**, distilling the ordering in Hermes' `subagent-driven-development`. 68 lines. |

The three verbatim copies have a `PROVENANCE.md` beside them recording the upstream path, the
licence, what was changed (nothing, in all three), and **when an agent should reach for it**.
The three adaptations have one too, recording what was stripped and why. The two written
here have none, because they are our own text — see below.

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

## Additions of 2026-10-03

The six reference harnesses vendor **258 `SKILL.md` files**: DeepSeek Harness 27, Hermes 209,
opencode 4, pi 17, OpenTUI 1. Four read-only investigators read them under one standard —
**instructions only, actually relevant to a Rust harness, earns its place rather than restating
`AGENTS.md`, no conflict, licence-compatible** — and three files were installed.

**Most of that corpus is a Python/web agent ecosystem's toolbox, not a Rust harness's.** The
Hermes set alone spans 23 domains including blockchain, gaming, smart-home, email and finance, and
the majority instruct the agent to install and run third-party CLIs (`npx`, `pip install`,
`git clone`, `curl | bash`) or to call vendor APIs. That alone disqualifies them.

### Three rejected outright, recorded so they are not re-litigated

| Skill | Why not |
|---|---|
| `1password` | **Conflict.** Writes `OP_SERVICE_ACCOUNT_TOKEN` into a plaintext `.env`. `AGENTS.md` §6: secrets belong in the OS credential store, never plaintext on disk. |
| `setup-wizard-generator` | **Conflict.** Its `write_env` helper persists secrets to a plaintext `.env`. Same rule. |
| `pr-lens` | **Conflict.** An unpinned `npx` CLI with an opt-in path that **publishes the diff to a third-party host**. `AGENTS.md` §11 forbids pushing or publishing; §6 forbids that supply-chain posture. |

Also rejected as irrelevant rather than harmful: DSH's Cordis composition skills (we compose
through Cargo crates and `epsilon-runtime`), Hermes' MCP-server-authoring skills (we *consume* MCP;
`crates/epsilon-mcp` has zero OAuth, so `mcp-oauth-remote-gateway`'s 17 pitfalls describe a
subsystem we do not have), and Hermes' 36 mlops skills (we consume models through a provider
protocol; we do not train, fine-tune or serve them).

### Recommended on 2026-10-03 — and now installed

Recorded here so the next session does not re-derive them. The survey recommended five; a
later pass installed them, three by adaptation and two written from scratch.

- **`grill-me`** (Hermes) — stress-test a plan by frontier rounds: ask every question whose answer
  is *not* blocked by another open question, recompute the frontier, repeat. Its Hermes tool
  names are stripped, and the repository rule — a fact discoverable locally is looked up,
  never asked — is promoted from a pitfall to step 2 of the procedure. `AGENTS.md` §12 covers
  briefing a subagent, not interrogating a plan before writing it.
- **`rest-graphql-debug`** (Hermes) → **`isolate-the-layer`** — a seven-step "isolate the
  layer, then fix" ladder plus the gotcha that a GraphQL response can be `200` with an
  `errors` array. Every `curl` and Python block is deleted, and the ladder now names
  `crates/epsilon-llm` and `crates/epsilon-server`. The gotcha is restated as a rule — *a
  successful exchange can carry a failure in its payload* — and tied to the two places this
  repository has that shape: the never-fails provider stream, where an error arrives as a
  `StopReason`, and message-text retry classification, where an unrecognised error is
  deliberately never retried.
- **`adversarial-ux-test`** (Hermes) → **`ux-triage`** — persona, then a mandatory pragmatism filter
  into RED/YELLOW/WHITE/GREEN, and the click-count heuristic. Browser, screenshot and ticket
  machinery removed, because none of them are available here.
- **Evidence-ID guardrails** (from Hermes `oss-forensics`) and **two-stage review ordering** (from
  Hermes `subagent-driven-development`) — the investigator's judgement was that these are worth
  ~30–50 lines **of our own text** rather than a 426-line forensics framework and a 352-line
  Python-flavoured workflow, because they are strict additions to `AGENTS.md` §5's evidence
  discipline. Writing them ourselves also avoids the `THIRD_PARTY_NOTICES` cost that adapting the
  parents would carry.

All five use one body shape — normative checklist, **Avoid / Prefer** block, **Test Matrix**,
**When NOT to use it** — because that shape is what makes a rule into a procedure. Each stays
under ~90 lines so it can be loaded whole.

## Note on `ratatui-tui`

It has uncommitted modifications inherited from a previous session (6 files). Those were
**not** reviewed before that session's work began and are **not** covered by the audit.
Decide whether to keep them on their own merits before committing.