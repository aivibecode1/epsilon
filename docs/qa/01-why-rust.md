# Design Q&A — Why Rust?

> Captured from a project conversation (2026-08-23). Question asked before implementation began;
> answer preserved as a standalone note for future contributors and agents.

## The question

Is Rust a good choice for building the Epsilon agent harness? Why?

## Short answer

Yes — but specifically because of *what Epsilon is*, not because Rust is fashionable.

## Where Rust pays off (evidence from the reference audit)

The four reference harnesses were audited first (`docs/research/harness-analysis.md`); their weaknesses
show exactly where Rust helps:

| Need | Evidence from references | Rust advantage |
|---|---|---|
| Streaming UX | pi folds SSE deltas live into its TUI | No GC pauses mid-stream, deterministic frame timing |
| Long sessions | pi holds the whole transcript in RAM; Hermes hit a synchronous-loop ceiling | Cheap tasks/threads; bounded tool pools without GIL or event-loop limits |
| Distribution | pi needed Bun-compile workarounds; Hermes needs an exact-pinned Python env | One static binary, no runtime install |
| Safety layering | DSH needed native addons (koffi FFI, custom Landlock addon) for confinement primitives | Landlock/bwrap/job-objects are first-class crates; process spawning is near-free |
| Ecosystem | — | tokio (LTS channel), rmcp official MCP SDK, direct per-provider HTTP via reqwest (pi-style, M17), ratatui, schemars — all verified current |

## Honest costs (so this is not cheerleading)

1. **Iteration speed.** TypeScript ships experiments roughly 2-3x faster: no compile wait,
   dynamic loading, instant reflection-style debugging.
2. **Async Rust learning curve.** Lifetimes x async traits take design effort; patterns trivial
   in JS (closures capturing state) need real structure here.
3. **Ecosystem breadth.** Fewer batteries than npm/PyPI. Mitigated by keeping provider
   transport behind our own `StreamFn` trait (ADR-003) — first over genai, and since M17 as
   pi-style direct per-provider HTTP.

## Verdict

Epsilon's value proposition IS what Rust is best at: a reliable, fast, safely-confined,
single-binary harness. The trade is worth it for this product. If the goal were rapid prototyping
of novel agent *behaviors*, TypeScript or Python would win instead.

Supporting existence proof: `hermes-agent-rs` (Lumio Research's Rust port of Hermes) runs
~110k lines of Rust in production-scale agent work.

## Related

- ADR-001 (language/runtime decision), ADR-014/017 (workspace shape and build order)
