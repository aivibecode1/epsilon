# The port audit's own recheck, and the extension seam that is built and uninhabited

**Date: 2026-09-28. Source: delegation `deleg_bf96cf7a`, two read-only tasks (424 s, 66
tool calls). Neither subagent modified a file; both claims below were re-verified by me
against the source before acting.**

Two findings. One corrects `02-port-fidelity.md` in the direction of *less* missing, which
is the direction an audit is least likely to get wrong by accident. One enlarges the gap,
and is the more useful of the two.

## 1. "Branch navigation: **Missing**" was wrong. It is wired, and the audit misread itself

`audit/02-port-fidelity.md:49` listed branch navigation as **Missing**, on the evidence that
`/tree` is "a *viewer*". The subagent found the production caller:

- `SessionStore::navigate_to` — `crates/epsilon-session/src/store.rs:1329`
- Called from the live TUI key handler — `crates/epsilon-tui/src/app/keys.rs:495`, on Enter
- Re-entry is real, not cosmetic: request messages are built from the store rather than from
  memory, so the next append parents to the moved leaf

**The audit made the exact mistake its own repository memory warns about.** It searched for
pi's identifier shape (`navigateTree`/`navigateTo`), found no `navigate_to`, and concluded
absence. `agents/memory/next-hardening.md:216` says it: *a grep that finds nothing proves
nothing if the identifier is wrong*.

Worth recording that this is **not the first time**. Two files in this repository already
said navigation was wired — `audit/evidence/port-gap-recheck-2026-09-21.md:82-86` and
`audit/evidence/F22-compaction-usage-never-read.md:149-153` — and the audit that superseded
them did not notice. The defect was not the missing caller; it was that nobody reconciled
contradictory files before publishing a verdict.

**What the audit got right by accident, and why it matters.** Its *conclusion* — that `/tree`
does not navigate the conversation — was true, and it is what I fixed in `7489c00`. The
store's leaf moved but the on-screen transcript did not follow. So the audit described a real
user-visible defect correctly while attributing it to the wrong mechanism, which is the worst
of both: right symptom, wrong cause, and a "Missing" label that would have justified building
something that already existed.

**The half that is genuinely missing is unchanged:** `branch_with_summary`
(`store.rs:1372`) still has no production caller. Confirmed by the subagent across all file
types, not just `*.rs`: 7 call sites, all tests.

### Correction applied

`audit/02-port-fidelity.md:49` now reads **Ported and wired** rather than **Missing**, cites
`keys.rs:495`, and states the real defect — the re-projection, since fixed. The
`branch_with_summary` row is untouched, because it was right.

## 2. The extension seam is correctly shaped and has no production caller

This is the larger finding, and the most actionable thing either subagent produced.

`ToolHooks` and `TurnHooks` are the right shape for pi's mutating extension events:
`BeforeToolCallResult { block, reason, terminate }`, result override, context transform,
system-prompt replacement, compaction cancellation. The agent loop calls them correctly
(`exec.rs:654,782`).

**I verified the gap directly: `set_tool_hooks` and `set_turn_hooks` occur 10 times in the
repository and every one is in `crates/epsilon-core/tests/loop_scenarios.rs`.** No production
caller, and no bridge from `ExtHost` to either trait.

This is the same shape as the defect I closed in `7489c00` — *a primitive ported faithfully,
tested, and never wired* — and it is the failure mode this project keeps producing. It is
worth naming as a class rather than a bug: **green tests on an unwired seam read as
progress, and they are the reason the gap is invisible.** `branch_with_summary` and
`ToolHooks` both pass every test they have.

### Scale of the remaining extension gap

pi's extension SDK declares 153 exports / 144 unique names across 26 capability areas
(`types.ts`, 4,125 LOC). `epsilon-ext` is 979 LOC and depends only on `epsilon-contracts` —
not on core, TUI, or LLM — so an extension child has no path to the agent at all.

| Area | pi | Epsilon |
|---|---|---|
| Events | 36, 15 with mutating results | 15, **all observational** — `send_event` returns `()`, no reply (`ext/src/host.rs:156-175`) |
| UI methods | 27 on `ExtensionUIContext` | 0 — no extension-facing UI path exists |
| Registration | `registerCommand/Shortcut/Flag/Provider`, 3 renderers | 0 — CLI is hand-rolled `parse_args`; the slash list is a static table |
| Message injection | `sendMessage`/`sendUserMessage` | 0 — no path from a child process to the agent queue |
| Provider registration | `registerProvider`, OAuth, custom transport | 0 — no registry to add to |

The port ships pi's **isolation** model well — process boundary, timeouts, crash budget,
frame caps, 668 lines of adversarial protocol tests — without its **capability** model. An
Epsilon extension can contribute a tool and watch the transcript. It cannot participate in
the run, appear in the UI, or outlive the process.

## What I did about it

Recorded, not built. Closing gap #1 means bridging `ExtHost` to `ToolHooks` — a real
behavioural change with a mutating protocol (`before_tool_call` must return a decision to a
child that is currently fire-and-forget), and it needs its own milestone and ADR per
AGENTS.md. Doing it as a drive-by alongside a TUI fix would be the "quick patch" the
brief rules out.

The concrete next step is in `audit/07-plan.md`: make the *event frame* bidirectional
first (id + reply, so a child can answer), because every other mutating API depends on that
one primitive. Wiring `ToolHooks` before the protocol can carry a reply would mean
implementing a seam that still cannot reach a real extension.
