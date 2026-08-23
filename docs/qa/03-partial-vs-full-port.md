# Design Q&A — What Exists After the First Milestones (Partial vs Full Port)

> Captured from a project conversation (2026-08-23). Question: after executing milestones M0-M5,
> do we have a full port of pi and a working harness, or a partial port?

## Short answer

A **partial port** — but the most important partial one. M5 delivers the complete architectural
spine (~80% of the risk retired) with roughly 60% of pi's features still missing.

## What works at end of M5

- Complete type vocabulary ported from pi (`epsilon-contracts`)
- Working agent loop: pi's two-level loop, steering/follow-up queues with drain modes, stop hooks
- Tool pipeline with all three improvements: bounded parallel pool, fail-all-on-truncation,
  doom-loop brake
- Session persistence: JSONL tree, forks/navigation, crash repair, multi-process leases
- LLM layer: faux provider (deterministic) + real Anthropic/OpenAI via genai + retry
- Proven by integration tests and a runnable demo loop

You can hold a real conversation with a real model calling tools through the full pipeline,
steer mid-run, fork sessions, kill and resume.

## What does NOT exist yet

| Missing | Milestone | Consequence until then |
|---|---|---|
| Real built-in tools (read/write/edit/bash/grep) | M6 | Model talks to stub tools only — not usable on real code |
| Permission rulesets + approval UX | M6 | Gates exist as a callback seam, no policy engine |
| Compaction engine + three-tier prompt builder | M7 | Long sessions hit context limits unmanaged |
| Skills, extension host, MCP client | M8 | No extensibility beyond compiled-in tools |
| TUI / server / print modes | M9 | Output is an event-printing demo bin |

## Trajectory

```
M5 (first plan):  harness kernel - architecturally complete, functionally a demo
M6:               FIRST GENUINELY USABLE agent (real files/commands, gated)
M7-M8:            long-session capability + extensibility = functional parity with pi
M9+:              product polish + Epsilon-specific advantages (safety layers)
```

The classic mistake this sequencing avoids: building the TUI first and retrofitting the spine —
the refactor both opencode and pi eventually paid for.

## Related

- docs/qa/04-finished-harness-vision.md (the full destination)
- design/epsilon-architecture.md section 14 (v1 exclusions)
