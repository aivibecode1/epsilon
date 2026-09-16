# Plan 007: Extension event forwarding — wire the real filter or remove the dead contract

> **Drift check**: `git diff --stat 64e927f..HEAD -- crates/epsilon-ext/ crates/epsilon-cli/src/runtime.rs`

## Status

- **Priority**: P2 · **Effort**: S-M · **Risk**: MED (behavior change when activated)
- **Depends on**: none · **Category**: security / tech-debt
- **Planned at**: commit `64e927f`, 2026-08-24

## Why this matters

SECURITY-06: `host.send_event` starts with `if filtered { return; }` and its only
caller hardcodes `true` (`runtime.rs:328`) — so M9a's shipped "event forwarding" has
NEVER delivered an event. Extensions built to observe runs silently receive nothing
(false assurance), and a future naive fix would suddenly stream full transcripts
(file contents, tool args) to third-party processes whose manifests asked for `["*"]`
— a one-character change nobody is watching. Decide deliberately: wire it with
end-to-end tests, or remove the manifest contract until supported.

## Current state

- `crates/epsilon-ext/src/host.rs:128-131` early-return; `manifest.rs:33-37` accepts
  `events`; `event_forwarding.rs:99-110` tests `wants_event` in isolation.
- `crates/epsilon-cli/src/runtime.rs:328` hardcoded `send_event(true, …)`; doc
  comment at `:312` claims filtering happens inside the host.

## Scope

**In scope**: host.rs send path, runtime.rs pump call, integration test.
**Out of scope**: new event taxonomy, extension API growth.

## Steps

1. Wire it properly: caller passes the per-host filter result:
   `let wants = manifest.wants_event(tag)` computed at spawn and stored on the host;
   `send_event` drops its bool param and uses stored state.
2. Integration test (extend `event_forwarding.rs`): fixture ext with `events:["*"]`
   receives a tool_execution_end frame; second fixture with no events field receives
   NOTHING while other hosts still work. Assert payload shape round-trips.
3. Correct both doc comments to describe actual behavior.

## Done criteria

- [ ] At least one real child process provably receives a forwarded event AND one
      provably receives none, in the same suite run.

## STOP conditions

- Fixture protocol cannot carry events without version negotiation changes.
