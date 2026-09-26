# M154 — the probe could type but not submit, so the post-turn header was unreachable

## The gap this closes

The owner asked for accurate, real-time context usage, cache hit rate and estimated cost.
M151 fixed the cache row. But every measurement I had taken of the header was of an **idle**
session:

```
idle   ?/164k · ↑0 ↓0 · R0 W0 CH — · DeepSeek V3.1
```

The `?` is not a bug — `context_used` is `None` before the first reply, and printing a number
there would be a fabrication. But it means **I had never once looked at the header in the state
a user actually spends their time in.** Every claim that context, cache and cost are "exposed"
rested on the idle screen and on golden frames I wrote myself.

## Why I could not measure it

`scripts/console-probe.py` could type into a real console but had no way to press Enter, and
`type_text` is the only input path. So the probe could photograph the composer holding text and
nothing else. The state with a real context percentage, a real cache rate and a real cost on
screen was *unreachable by the instrument built to measure it*.

The mechanism already existed and was simply not exposed: `VK_BY_CHAR` maps `\r` to
`0x0D`, which is the keystroke a TUI's key handler recognises as Enter. A probe writing only
characters could type `/help` forever and never run it.

## What changed

- `--enter` — submit what was typed, then read the screen *again* into `turn_screen`.
- `--settle SECONDS` — how long to wait after submitting before reading. The default (9 s) is
  generous for the faux provider; a real provider needs it, and without the knob the
  measurement silently reports a model that had not finished rather than a header.

## What the measurement found

`--size 100x30 --enter --settle 12 --keys y`, real conhost window, mid-turn then settled:

```
⠧ running 13s · streaming…   ctx <1%/164k · ↑100 ↓50 · R0 W0 CH — · $0.0001 · DeepSeek V3.1
```

```
idle                          ctx <1%/164k · ↑200 ↓100 · R0 W0 CH — · $0.0001 · DeepSeek V3.1
▸ ✓ bash $ echo epsilon-ready · ok · 15s
• permission allowed
✓ ✓ bash $ echo epsilon-ready · ok · 15s
• Run complete.
```

All three of the owner's requirements are met and measured: context is live and is the *session*
context rather than the last prompt, the cache row is honestly empty rather than falsely zeroed,
and cost is present and non-zero. The window never moved off 100×30.

**And it found a defect the idle probe could never have found:** the tool card is drawn twice,
once as the call and once as the result, with the same headline and body. That is the
long-standing design `a_tool_call_and_its_result_are_two_cards_but_one_call` pins — a pending
call genuinely is a different fact from a settled one, since a permission prompt can leave a
call pending indefinitely. But that test asserts the event-stream shape, and nobody had looked
at the pair *painted*, where it reads as a duplicate. Deliberately **not** fixed here: it is a
design decision with an argument on both sides and deserves its own slice with a control.

## What this milestone changed

Only `scripts/console-probe.py` — no Rust, so the cargo gates have nothing to say about it. The
evidence is the probe's own JSON and the two screens above; see
`audit/evidence/post-turn-header-probe.md`.
