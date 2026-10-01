# Design Q&A — Who owns what, when there are two frontends

> Captured 2026-10-01, at M233, when the project grew a second frontend. The question:
> a terminal UI and a desktop IDE now both drive one agent. Which code goes where, and
> what stops them drifting apart?

## The rule

> **A frontend owns pixels. It does not own agent behaviour.**

Every turn, tool call, permission decision, compaction and session write in both windows is
the same `epsilon_core::Agent`, built by the same function, over the same session files, behind
the same `ApprovalUi` seam. There is no second agent loop, no second permission engine, and
no desktop-only shortcut around a gate.

## Why it needed writing down

Because the alternative is not "a bit of duplication" — it is a *silent* duplication. The
first version of `build_runtime` was a private `async fn` in `crates/epsilon-cli/src/main.rs`,
beside 3,199 lines of helpers in `cli/src/runtime.rs`. There was exactly one of it, and it
was unreachable from any crate that was not the `epsilon` binary. A second frontend could
only have reimplemented it.

Everything a reimplementation would have copied, by name:

| What | Where it lived before M233 |
|---|---|
| Tool registry + MCP attachment | `main.rs:1747-1756` |
| Skill discovery and the `skill` tool | `main.rs:1760-1785` |
| Extension roots, trust gating, manifests | `main.rs:1791-1800` |
| The permission layers and `--plan` | `main.rs:1806-1818` |
| Resume resolution and `session_path_for` | `main.rs:1851-1865` |
| Lock reaping, store open, migration, backup notice | `main.rs:1873-1910` |
| System-prompt overlay, agent definitions | `main.rs:1919-1939` |
| Repair, unknown-entry notice, model view | `main.rs:1972-1995` |

Fourteen steps, each with a rule that has a defect behind it. A second copy of that list is
fourteen opportunities to disagree — and, critically, **the two copies would disagree
silently**: a skill root that moves in one is still right in the other, and nothing errors.

This is the same class as ADR-012's `global_dir` (one rule written twice, byte-for-byte) and
as the `fork_branch_at` defect ADR-019 records: a correct primitive wired to the wrong caller,
with green tests because the tests call it directly.

## Where things live

```
epsilon-contracts     types and traits
epsilon-llm           providers, catalog, auth
epsilon-session       the durable store
epsilon-core          the agent loop
epsilon-tools         the tools
epsilon-safety        the permission engine and the ApprovalUi seam
epsilon-runtime       COMPOSITION: workspace + configuration  ->  a running agent
epsilon-tui           a frontend
epsilon-desktop       a frontend
epsilon-server        a frontend
epsilon-cli           the binary: argument parsing and mode dispatch
```

`epsilon-runtime` exists because **frontends are peers, not clients of the CLI**. Depending
on `epsilon-cli` as a library would work today and would be wrong: it drags `epsilon-tui`,
`epsilon-server` and `epsilon-mcp` into a native GUI that wants none of them, and it makes
the second frontend a consumer of the first rather than a peer of it.

## The three questions to ask

**"Does this need to exist?"** Most frontend code does not. Read state, format it, draw it.
Everything below the line belongs upstream.

**"Is this behaviour, or presentation?"** Behaviour is anything that changes what the agent
does or what a user is *told* the agent did. A header that names a model is behaviour: M233
in the TUI was a header that said `DeepSeek V3.1` over a faux turn, and the fix was to carry
the CLI's *decision* rather than re-derive it from the model id. The desktop's status line
says "no agent attached" rather than inventing a cost, for the same reason.

**"Would a second frontend need this too?"** If yes, it is not in the frontend.

## What each frontend is allowed to add

| Allowed | Not allowed |
|---|---|
| Its own layout, theme, widgets | A second `AgentEvent` reducer |
| Its own keybindings | A second permission decision |
| Its own session *picker* (it reads `epsilon_session::list_sessions`) | Its own session *writer* |
| Its own `ApprovalUi` implementation | Its own `PermissionEngine` |
| Formatting that matches what the terminal shows | A different number |

That last row is the one people get wrong. Both frontends must print **the same number** for
the same session, from the same source. A desktop that shows a nicer cost figure is a desktop
that is lying.

## The rule that survives a rewrite

A claim about what a user sees is verified by painting a frame and looking at it — in both
frontends. `IdeLayout` is a pure function so its rules are tests; `App::paint` returns a
`FrameSummary` so a test can assert on what was drawn rather than on state that ought to
produce a draw. That shape is not new — `epsilon_tui`'s `render_rows` is the same idea — and
it is the reason three layout bugs were caught in an afternoon that reading the arithmetic had
missed.
