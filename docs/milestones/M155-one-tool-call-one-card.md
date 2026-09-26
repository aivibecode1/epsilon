# M155 — one tool call, one card

## The defect

M154 measured a settled turn on a real Windows console and found the tool call drawn **twice**:

```
▸ ✓ bash $ echo epsilon-ready · ok · 15s
▸ epsilon-ready
• permission allowed
✓ ✓ bash $ echo epsilon-ready · ok · 15s      ← the same call, again
✓ epsilon-ready
```

Same headline, same body, twice. On **every** tool call — so it is not an edge case, it is what
every tool result in this harness has always looked like.

## What I found when I read the code

`push_tool_result` called `settle_call`, which updates the call's own card's status and end
time — and then, separately, pushed a **new** card carrying the result. Both existed. The
`settle_call` doc comment had already stated the intent:

> the result settles the call's own row rather than leaving a permanently pending one to
> scroll past

So the author wanted what I have now. The twin was an artifact of implementing half the
sentence. And a test named `a_tool_call_and_its_result_are_two_cards_but_one_call` asserted
`cards.len() == 2` — the test's **name** had become the behaviour, and the assertion froze the
oversight in place.

## The fix

When a result settles a call card, *finish that card* — body, head, args, and the line's
`kind` so a finished call is drawn with the result's colour and glyph rather than still looking
like a request that never came back. No second row.

**A result with no call to settle still gets its own row.** A replay that lost the call, or a
result carried over from another branch, has nothing to settle, and dropping it would lose the
tool's output entirely. This is why the fix is not "one card per `tool_call_id`" — that would
be a simpler sentence and a lossy one. `a_result_with_no_call_in_the_transcript_still_gets_its_own_row`
pins that half.

## Two tests rewritten, and why that is the point

- `a_tool_call_and_its_result_are_two_cards_but_one_call` →
  `a_tool_result_settles_the_calls_own_card_rather_than_doubling_it`, now asserting one card,
  a settled status, a duration, the result's body, and the re-kinded row.
- `run_lifecycle_renders_in_order` expected three rows and now expects two, and its comment
  ("assistant prose, the tool call, its result") was rewritten to say the call *is* the result
  row.

Neither rewrite is cosmetic. Both tests were pinning the duplication as correct, and the second
one now also pins that a bash row's line text is the *command* (`$ echo hi`) while the tool
name lives in the card title — M153's change, which the old assertion would have hidden.

## Negative control

Disabled only the in-place branch (`if false && settled.is_some()`), leaving the twin push:

```
test result: FAILED. 347 passed; 2 failed
  assertion failed: one call, one card
    left: 2
   right: 1
```

## Painted proof

The same probe that found the defect, re-run against this build (`--enter --settle 12 --keys y`,
real conhost, 100×30 — the window did not move):

```
idle      ctx <1%/164k · ↑200 ↓100 · R0 W0 CH — · $0.0001 · DeepSeek V3.1
▎ ❯ run the check
✓ ✓ bash $ echo epsilon-ready · ok · 12s
✓ epsilon-ready
• • permission allowed
• Run complete.
```

Against M154's capture, which showed the same call twice. The card appears **once**, carrying
the command, the status and the duration, with the result's body under it.

## What still nothing exercises

No golden frame pins the transcript of a *completed* tool call end to end — the painted
check above is a probe run, not a test, so it cannot fail in CI. The state-level tests prove
the row is settled once; only a human or a scheduled probe would notice a regression that
reintroduced the twin.

That is the third milestone running in a row to end on this note (M152, M153, M155). The
pattern is the real finding: this crate asserts *state* far more readily than it asserts
*what a user sees*, and the three defects it shipped in this sequence — a running card
that looked hung, a header made of the body's first 400 characters, and every tool call drawn
twice — were all invisible to the state tests and all obvious on a screen. The cheap fix
is a golden frame for a completed tool call. It has not been written yet.
