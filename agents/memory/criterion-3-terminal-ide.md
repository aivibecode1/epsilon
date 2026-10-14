# Criterion 3 — the terminal IDE experience

Audited 2026-10-14 by reading the draw path, **not** by designing a replacement.
Do not build before reading this.

## What the layout is today

`crates/epsilon-tui/src/ui/shell.rs:67` `paint()` composes exactly three regions,
top to bottom:

```
Regions::split(area, draft_lines)
  ├── header      — status line: ctx, tokens, cost, model, thinking, mode
  ├── transcript  — ONE column
  └── composer    — the input
```

The transcript is a single column. Everything the agent does — conversation turns,
tool calls, execution output — is interleaved in it, with tool calls drawn as
*cards* inside the same scrollback (`ui/card.rs`). There is no second pane.

## The gap against criterion 3

The criterion asks for "clear separation of conversations, tool calls, execution
output, agent activity, and task progress" and "real-time visibility into everything
the agent is doing, rather than a basic chat interface."

Measured against the current frame: **the separation does not exist as layout.** It
exists as *inline structure* — cards, glyphs, labels — inside one column. Whether
that is a defect or a deliberate constraint is a design question, and it is the
owner's, not mine:

- **For** restructuring: a single column means a long tool output pushes the
  conversation off screen, and the user scrolls to find the turn they were reading.
  Real-time visibility of "what is the agent doing right now" competes with
  "what did it just say."
- **Against**: ratatui panes cost width, and a terminal is 80 columns by default.
  A two-column split at 80×24 makes both columns unusable. The current design is
  the one that fits the smallest supported terminal, and the repository's own floor
  is enforced by `Regions::too_small`.

## What already exists and should NOT be rebuilt

- tool cards with own header, body, status (`ui/card.rs`) — already a component
- header metrics, one place, held to a test by a legend (`ui/metrics.rs`)
- tabs in the file viewer (`tree.rs`) — a real tab strip, the only one in the app
- panels/sessions browser, palette, composer with undo and kill-ring

**The question to answer first:** is criterion 3 asking for a *layout* change
(side-by-side panes) or for the *visibility* the criterion names (activity, task
progress, current tool) delivered inside the existing column? The second is cheap,
fits 80×24, and is achievable in slices. The first is a re-architecture that will
touch every frame test in the crate.

## MEASURED DEFECT (2026-10-14) — the header says `idle` while a tool is in flight

Frame `audit/evidence/frames/streaming-tool-running.100x30.txt`:

```
|idle           ?/1.0M · ↑0 ↓0 · R0 W0 CH n/a · $0 · Faux (offline scripted) ...|
|▸ bash $ cargo build · pending                                                |
```

The transcript card says a tool call is `pending`; the header says **`idle`**. The
header is the one row a user reads to learn whether work is happening, and it is the
row that lies. This is criterion 3's "real-time visibility into everything the agent
is doing" failing in the exact place it matters.

**The machinery already exists and is unused**: `RunState::Running { frame, phase,
elapsed_secs }` (`ui/header.rs:22`) with `label()` producing `running 12s · running
bash`. This is not a missing feature, it is a missing *feed* — the header's `RunState`
is not derived from the in-flight card.

**This retires my own recommendation.** I wrote above that the fix was to build a
"live activity strip" in the header. There is one, and the region already carries the
state; the defect is that nothing tells it a tool started. Do not build a second one.

## Status

**WITHDRAWN — it was the fixture, not the product.** `RunState` is driven by
`AgentEvent::AgentStart` (`app.rs:1767`, `run_started_at = Some(now)`), and the fixture
at `frame_dump.rs:347` emits `MessageStart`/`MessageEnd`/`ToolExecutionStart` and never
`AgentStart`. So the header correctly reported `idle` for a state where no run had
started; the card was pending but nothing was running.

**So this is not a defect.** A frame fixture that does not drive the event the header
reads is a fixture problem, and I nearly "fixed" a working feed because I read the
frame instead of asking which event drives it. That is the sixth withdrawal this
session, and the fifth where the frame or a read, rather than a control, was what I
reasoned from.

**Status: no defect found in criterion 3's visibility half.** The header does carry the
running tool's phase and elapsed time whenever a run is actually in flight — verified
by `golden.rs:3605`, which sets `run_started_at` and asserts the header.

The layout-restructuring question remains open and is the owner's. The honest state of
criterion 3 is: **the visibility requirement is met; the layout-separation requirement
is not, and building it is a design decision I should not make alone.**
