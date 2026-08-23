# Design Q&A — The Lego-Brick Build Methodology

> Captured from a project conversation (2026-08-23). Answers how Epsilon is built from
> independent parts ("bricks") and why that maximizes build speed AND debuggability.

## The question

If we build the harness from scratch as lego bricks — building parts, then joining them — which
approach is easiest and most efficient to build and debug?

## Core principle: contracts are the studs

Each brick is a crate; the stud-shape is a **trait + types**, defined *before* either side exists:

```
brick 0: epsilon-contracts   (types only: Message, AgentEvent, StreamEvent, SessionEntry,
                              Tool trait — zero logic, zero internal dependencies)
bricks 1..n: everything else snaps onto those shapes
```

This single decision makes independent building AND debugging possible: any brick can be tested
against a **fake counterpart** instead of the real one.

## Build order (dependency-sorted; each brick shippable alone)

| Order | Brick | Why this position | Its debug tool |
|---|---|---|---|
| 1 | `contracts` | Everything compiles against it | serde roundtrip tests |
| 2 | `session-store` | Pure logic, fs-only deps | `replay` bin: dump/validate/fork any session file |
| 3 | `llm` + faux provider | pi's key trick: a scripted deterministic provider runs the whole loop offline, free, reproducibly | record/replay HTTP cassettes |
| 4 | `core` loop vs faux provider | Loop is small once provider+store exist | golden-file tests: scripted responses -> exact expected event stream |
| 5 | `tools` (fs first, bash last) | Each tool unit-testable in a tempdir | per-tool CLI bins |
| 6 | `safety` permission eval | Wildcard rulesets are pure functions — the most testable security code you will ever write | `eval-rule` debug bin |
| 7 | compaction + prompt builder | Needs 2-6 | prompt snapshot hash tests |
| 8 | `ext-host`, `mcp` | Riskiest I/O surfaces, isolated late | echo/mock extension + mock MCP server fixtures |
| 9 | frontends (`tui`, `server`, `cli`) | Pure consumers of events — no agent logic left to debug | server mode IS the debug UI (SSE + curl) |

## Why this ordering maximizes debugging efficiency

1. **Event-sourcing = time-travel debugging.** "Model-visible means logged" means any bug
   reproduces by replaying the session JSONL through any brick, deterministically.
2. **Faux provider kills the two worst debugging categories**: flaky network tests and paid API
   calls in CI. Full-loop integration tests become millisecond-fast and free.
3. **Pure-function seams** (permission eval, prompt assembly) get property-based tests — bugs
   surface as generated counterexamples, not manual repros.
4. **Frontends last** so TUI glitches can never masquerade as agent bugs.

## Two warnings taken from the reference codebases

- **Do not over-brick.** DSH's ~150 packages was an anti-pattern (harness-analysis §2 W1).
  Rule of thumb: *a brick earns existence when you have written its third consumer.* Start at ~10
  crates; split later only on proven need.
- **Contracts freeze slowly.** Early on, keep the vocabulary crate unstable-but-versioned;
  churning the stud-shape forces rebuilds of every attached brick. Freeze hard at v1.

## Related

- ADR-017 (build order decision), ADR-014 (why contracts became its own crate),
  docs/milestones/* (the methodology applied for real)
