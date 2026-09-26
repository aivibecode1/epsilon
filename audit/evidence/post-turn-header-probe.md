# M154 — the post-turn header, measured in a real Windows console

## Why this measurement exists

Every previous claim that context usage, cache hit rate and estimated cost are "exposed" rested
on an **idle** screen:

```
idle   ?/164k · ↑0 ↓0 · R0 W0 CH — · DeepSeek V3.1
```

The `?` is not a bug — `context_used` is `None` before the first reply, and a number there
would be a fabrication. But it means the state a user actually spends their time in had never
been photographed. Worse, it could not be: `scripts/console-probe.py` could type but had no
way to press Enter, so the instrument built to measure the TUI could not reach the only state
worth measuring.

`type_text` already mapped `\r` to `VK 0x0D` (`VK_BY_CHAR`) — the mechanism existed and was
simply not exposed. Added `--enter` and `--settle`; then `--keys`, after the first run showed
a `bash` turn parks on a permission dialog, so a header read at that moment describes a *waiting*
turn, not a *finished* one.

## What the real screen says

`--size 100x30 --enter --settle 12 --keys y`, against a real conhost window
(`probe163.json`). Mid-turn, and then settled:

```
⠧ running 13s · streaming…   ctx <1%/164k · ↑100 ↓50 · R0 W0 CH — · $0.0001 · DeepSeek V3.1
```

```
idle                          ctx <1%/164k · ↑200 ↓100 · R0 W0 CH — · $0.0001 · DeepSeek V3.1
▎ ❯ how many tokens so far
▸ ✓ bash $ echo epsilon-ready · ok · 15s
▸ epsilon-ready
• permission allowed
✓ ✓ bash $ echo epsilon-ready · ok · 15s
✓ epsilon-ready
• Run complete.
```

Against the owner's three requirements:

| Asked for | Measured | Verdict |
|---|---|---|
| accurate, real-time context usage | `ctx <1%/164k`, live during and after | met — and it is the *session* context, not the last prompt, so it cannot flatter |
| cache hit rate | `R0 W0 CH —` | met — correct, and honestly empty: the faux provider has no cache. A `0%` here would have been a lie |
| estimated cost | `$0.0001` | met — non-zero and present |

Also confirmed in the same run: the window stayed 100×30 (no freeze, no resize), and M153's
renderer is visible in a real console — `bash $ echo epsilon-ready · ok · 15s` carries the
command, the status and the duration on one line.

## The defect this measurement found

**The tool card is drawn twice.**

```
▸ ✓ bash $ echo epsilon-ready · ok · 15s     ← the call card
▸ epsilon-ready
✓ ✓ bash $ echo epsilon-ready · ok · 15s     ← the result card
✓ epsilon-ready
```

Same headline, same body, twice. This is not a regression — it is the long-standing design
that `a_tool_call_and_its_result_are_two_cards_but_one_call` pins, and it predates M153. But
that test asserts the *event-stream* shape, and nobody had looked at what the pair looks like
*painted*, where it reads as a duplicate. The rationale is legitimate (a pending call is a
different fact from a settled one, and a permission prompt can leave a call pending
indefinitely), so the fix is not "delete the call card" — it is for the result to not redraw
everything the call already said.

**Not fixed here.** It is a design decision with a real argument on both sides, and it deserves
its own slice with a control, not a drive-by edit inside a probe milestone.

## What this milestone changed

Only `scripts/console-probe.py`. No Rust changed, so the cargo gates have nothing to say about
it; the evidence is the probe's own JSON and the two screens quoted above.
