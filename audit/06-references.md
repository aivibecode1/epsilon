# 06 — What to take from the references

Four reference harnesses sit in `references/`: `pi-0.85.1` (the port target),
`hermes-agent-2026.9.14`, `opencode-1.18.31`, and
`deepseek-harness-dsh-v0.1.6-alpha.2`. A comparison that lists everything they do and
admits nothing is not useful. This document takes the opposite stance: **a short list of
things genuinely worth taking, and a longer list of things that look attractive and are
not.**

The test for "worth taking" is the same one the rest of this audit uses: it must be
something Epsilon demonstrably lacks, it must not compromise the safety layer, and it
must be implementable without a rewrite.

## Worth taking

### R1 — pi: a *real* branch navigator. *The cheapest parity win available.*

pi's `navigateTree` (`coding-agent/core/agent-session.ts:3116`) moves the session leaf
to any ancestor and re-enters from there, abandoning the abandoned tail with a summary.
Epsilon has the store half of this and nothing wires it up:

```
$ grep -rn "branch_with_summary" --include=*.rs crates/
  → every hit is in tests/ or is the definition itself
```

`branch_with_summary` (`store.rs:1372`) mirrors pi's shape exactly and
`fork_branch_at` (`store.rs:1486`) writes only the selected ancestry with fresh
identity. Both are tested. `/tree` today is a viewer
(`tui/src/app/commands.rs:401`).

Everything hard is done and proven. What is missing is a picker that moves the leaf
instead of describing the tree. **This is the first implementation task after the TUI
harness**, because the ratio of user-visible capability to work is unlike anything else
on the list.

### R2 — pi: a default tool set proportionate to the context budget.

pi activates **4** tools by default (`agent-session.ts:2810`). Epsilon hands the model
the full **11-tool registry** (`core/src/lib.rs:2612`) on every request.

This is the most consequential divergence in the port, and it is invisible in any
capability table because both projects "have tools". The cost is paid on *every single
request*, in a product whose entire premise is managing a finite context window: 11 tool
schemas plus descriptions is several thousand tokens of system prompt before the user has
typed anything.

The fix is not "remove tools". It is a per-agent-type tool set — `default` gets a small
set, `full` gets everything — which also makes the existing agent-type machinery
(`lib.rs:2612`) do something it currently does not.

### R3 — opencode: make the permission default proportionate too.

The `bash` default asks (`safety/src/engine.rs:123`), and that is defensible in
isolation. But it means the first tool call of a new session opens a modal, and the
modal is the first thing a new user meets. opencode's model — a workspace-scoped trust
boundary, with escalation rather than a blanket ask — matches what Epsilon's own
`external_dir` rule already implies: allow inside the workspace, ask outside it.

Whatever is chosen here, the rule from [04](04-tui.md) applies: the composer must be
visibly inert while a modal owns the keyboard, and **no key may ever be discarded without
feedback**. That second half is not a feature request; it is an invariant.

### R4 — hermes: an account-usage surface as a first-class diagnostic.

`references/hermes-agent-2026.9.14/agent/account_usage.py` and `aux_accounting.py`
treat per-account rate-limit and quota state as something the agent reports about
itself. Epsilon's `cache_report()` (`core/src/lib.rs:1954`) already does the
equivalent for the prompt cache — one line per fact, telling the user *why* a hit rate
is low rather than just what it is.

That instinct is right and should be extended to the two numbers most likely to be
misleading: per-model cost and context-window size. [05](05-metrics.md) finds both can
be quietly wrong. An `epsilon doctor`-style command that says "this model's context
window is a 2026-09-21 snapshot and the vendor says 1M, not 164k" is exactly the
diagnostic that turns a wrong number into a known-unknown.

## Attractive, and not worth taking

### Not: pi's extension SDK, in full.

`coding-agent/core/extensions/types.ts:1198-1346` exports 157 symbols — an event bus,
`addCommand`, `registerProvider`, `setStatus`, `newSession`. Epsilon's `epsilon-ext`
(`host.rs:62`) supports process tools and declared events, which is a real subset.

Taking the rest is not a port, it is building a plugin system, and a plugin system in a
product with a permission gate is a security surface. The subprocess model Epsilon
already has is the right boundary: an extension is a process, it gets tools through the
same gate as everything else, and it cannot corrupt the TUI. **Keep the boundary, skip
the surface area** until there is a user asking for it.

### Not: pi's package manager.

`package-manager.ts` is 2,699 lines plus a 1,102-line CLI, for installing extensions and
resources from npm and git. Epsilon has root discovery (`cli/src/runtime.rs:564-608`)
and no installer. Building an installer means a dependency-resolution story, a
supply-chain story, and a trust story — and the repository already has a supply-chain
ADR with no advisory scanner. Wrong order.

### Not: opencode's or Hermes' telemetry.

Both collect it. Epsilon should not until it can do so without a consent story, and a
local-first coding agent has no use for a metrics backend it cannot justify to the
person running it.

### Not: SQLite session backends.

JSONL is the right default for a local tool: it is greppable, diffable, and survives a
crash mid-write. SQLite is the right answer for a server with many concurrent sessions,
which Epsilon is not yet.

### Not: Kitty/iTerm2 terminal graphics.

`state.rs:41-42` says Kitty parity is "honest-parity deferred (needs detectCapabilities +
Kitty negotiation)". That label is correct and should stay. Half the terminals in use do
not speak the protocol, and a graceful text fallback is worth more than fidelity to a
protocol most users have never heard of.

### Not: 25 more providers.

pi supports 38; Epsilon supports 13 with 557 model rows. The long tail is mostly
OpenAI-compatible endpoints, and Epsilon already has an OpenAI-compatible wire
(`providers/mod.rs`). A provider whose only difference is a base URL is a
configuration, not a port. The catalog should grow by *rows*, not by code.

## The one thing none of the four does better

Worth stating, because an audit that only finds deficits is not a fair reading. Epsilon
has a **permission and safety layer that all four references lack**: `epsilon-safety/`
with a real policy engine (`engine.rs`), escalation (`escalation.rs`), and OS-level
confinement via bubblewrap and Windows Job Objects (`sandbox/jobobject.rs:514`). pi's
`effect-gate.ts` is a seam, not a policy. Hermes and opencode gate tools, but neither
ships a sandbox.

That is the thing to protect while taking everything else from these references. It is
also the thing most likely to be damaged by a careless port — which is why R3 is worded
as "make the default proportionate" and not "remove the gate".

## Honest limits

- This is a static reading of four codebases. Nothing was executed from any reference.
- Hermes is a Python agent and is architecturally unlike a Rust TUI; the comparison is
  about ideas, not structure, and only two of its ideas are proposed here.
- The DSH reference was not studied in depth. It is the newest and the smallest, and a
  fresh pass over it specifically is worth doing before the restructure lands.
- Line-count comparisons are gross and include tests and comments.
