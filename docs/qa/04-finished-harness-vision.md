# Design Q&A — The Finished Harness: Look, Feel, and Inheritance Map

> Captured from a project conversation (2026-08-23). Question: when the project is finished,
> what will the harness look like, and what good things does it take from the other harnesses?

## Day-to-day experience

```
$ epsilon

  Epsilon v1.0 - claude-sonnet-4 - ~/projects/myapp - main
  context 23% - $0.41 session cost - 4 extensions active

> fix the flaky auth tests, then commit if green

  reading tests/auth_test.rs ...
  ok   read   tests/auth_test.rs            (142 lines)
  ok   bash   cargo test auth               [permission: allowed]
  DENY bash   git push                      [sandbox denied -> escalate? y/N]
```

- **Terminal-first TUI**: streaming markdown, collapsible thinking blocks, one-line tool cards
  expandable on demand; status footer with model/tokens/cost/context %
- **Permission prompts** answered `once / always / reject-with-correction`; sandbox denials
  become escalation dialogs, never dead ends
- **Four modes**: interactive TUI; `-p "task"` print mode for scripting; `serve` (SSE/REST) so
  editors or web UIs attach later; `rpc` for embedding over stdio
- **Sessions you fork** like branches, resume across processes, replay deterministically,
  inspect as plain JSONL under `.epsilon/sessions/`
- **Skills** (`SKILL.md` folders) and **MCP tools** (`mcp__server__tool`) plug in without
  touching core; third-party extensions run as isolated processes that can crash harmlessly
- **Single static binary** (~10MB), no runtime install, first-class on Windows/macOS/Linux

## Inheritance map — what came from each reference

### From pi — the skeleton (port target)

| Taken | Why |
|---|---|
| Two-level agent loop + steering/follow-up queues with drain modes | Cleanest loop of all four; precise queue semantics nobody else specified |
| StreamFn never-throws failure contract | One channel for retries/errors/aborts instead of exception plumbing |
| Typed event stream consumed by reducers | Every frontend becomes trivial |
| JSONL tree sessions with typed metadata entries | Forks/navigation as data operations |
| Fail-all-tool-calls-on-truncation | Prevents silent corruption from cut-off JSON args |
| Cache-isolated auxiliary calls; skills spec; supply-chain pinning | Each proven in production |

### From DSH — the safety spine

| Taken | Why |
|---|---|
| OS confinement seam: fail-closed, per-call policy (landlock/bwrap/seatbelt) | Only real sandboxing among the four |
| In-band escalation protocol | Denial becomes a justified retry and approval prompt, not a dead end |
| Monotonic deny-only guard stage | Security decisions immune to listener ordering |
| Surface-shadowing compaction | Model view rewritten while human transcript stays intact |
| Bounded parallel pool + concurrency classification | Fixes pi's unbounded fan-out |
| Spill locators; durable retry ledger; sticky turn-end reasons; repeat-call advisories | Crash-safe resume and loop-breaking |

### From Hermes — cache discipline and operational craft

| Taken | Why |
|---|---|
| Three-tier byte-stable prompt assembly | Deepest prompt-cache discipline observed |
| api_content sidecar | Byte-identical session replay across restarts |
| Turn leases | Multi-process safety over one state store |
| Amend-not-regenerate rolling summaries | Cheaper, more stable compaction |
| TTL + failure-grace tool availability probes | Transient flakes do not strip toolsets mid-session |

### From opencode — permission UX and product patterns

| Taken | Why |
|---|---|
| Last-match-wins wildcard rulesets; once/always; reject-with-correction | Most usable permission system of the four |
| Parser-based shell analysis feeding permissions | Patterns match command semantics, not regex guesses |
| Strict compaction template + protected-tail clamp + overflow replay | Compaction as designed path |
| Doom-loop brake tied into permissions | Stops runaway agents gracefully |
| .txt tool descriptions; OpenAPI-from-types server | Prose/code separation; multi-frontend reuse |

## What Epsilon has that none of them have

1. **Both safety layers combined by default**: intent gates AND OS confinement, wired together.
2. **Process-isolated extensions as default distribution**: everyone else runs extensions
   same-process; only DSH sandboxes them, experimentally.
3. **Single static binary** with this feature set.
4. **Windows-first-class development**: the references treat win32 as second-class.
5. **pi-grade architectural simplicity WITH DSH-grade safety**: no existing harness offers both.

One-line summary: pi's loop and structure, DSH's safety spine, Hermes' cache discipline,
opencode's permission UX.
