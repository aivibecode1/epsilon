# M158 — the contradiction a comment could not settle

## What this is

A milestone that changes **one sentence of a doc comment**, and earns its number by proving
that sentence was wrong rather than by replacing it with another plausible-sounding one.

## The debt

M152 proposed a live elapsed timer on a running tool card, then declined to build it, and in
doing so recorded this in the milestone log:

> The field's own doc comment claims `None` "makes the duration live rather than absent", which
> contradicts the method two lines below. **That contradiction is left in place and recorded
> here rather than quietly resolved**: one of the two comments is wrong, and which one should
> change is a decision for whoever does the render-cache work, with a measurement behind it.

That was the right call at the time. M152 had no way to observe a running tool in a console —
its own *what nothing exercises* section says so — so it had two comments in conflict and no
evidence to break the tie. Picking one would have been a coin toss dressed as a decision.

## The measurement that settled it

M154 made the console probe able to submit a turn, and captured a live frame:

```text
⠧ running 13s · streaming…   ctx <1%/164k · ↑100 ↓50 · R0 W0 CH — · $0.0001 · DeepSeek V3.1
```

The header carries a live figure, repainted every frame. So both statements were true about
different surfaces:

- the **header** shows a running call's elapsed time, and can keep it live;
- the **card** deliberately shows none, because a transcript row is cached until something
  invalidates it, and a frozen number that looks measured is worse than no number.

Only the field's comment was wrong. It is now corrected, and — the part that matters — the
architecture is **asserted** rather than left to a comment to carry:

`a_running_call_shows_its_time_in_the_header_not_on_the_card` renders a running call on a real
frame and checks both halves: the header says `running 12s`, and the card row carries no
duration at all.

## The state-level half was already proven

`only_a_finished_card_carries_a_duration` (`state.rs`) already asserted `elapsed_ms() == None`
while pending. The new test deliberately does **not** repeat it. Re-asserting a proven fact
would have made the milestone look busier while proving less, and the only thing M158 adds is
the claim that was actually unproven: what the reader sees.

## Getting the test right took four attempts

Each one was a guess about an API that did not exist, and the compiler was right every time:

1. Invented `agent_event_tool_call_named` and `begin_turn`/`on_tool_start`/`cards` — none exist.
   The real path is `UiState::apply(&AgentEvent::…)`, which is what the M156 golden test uses.
2. Drove `UiState` directly, then asserted the **header** said "running". It said `idle`. The
   header's live figure comes from `App::run_started_at`, stamped by the event loop — a test
   that reached the header by a different route would have been testing the route, not the
   behaviour.
3. Rewrote the doc comment and **deleted the `pub ended_at_ms: Option<u64>` field** along with
   the two old comment lines. The compiler found it as two unrelated `no field` errors in
   `app.rs` and `renderers/tests.rs`, neither of which pointed at the edit.
4. Finally green.

Step 3 is the fourth time this session that an edit anchored on comment text removed code it
did not own. The mitigation is unchanged and still insufficient: anchor on code, not prose.

## Control

`elapsed_ms` forced to `Some(now - started)` with `unwrap_or(0)` — the exact stale-live
behaviour — fails **360 passed / 2 failed**: the new frame test and the existing state test
together. Restored byte-identically; the `.ctrl` copy is gone.

## What nothing exercises

No console probe has rendered this specific state, though M154's captured frame shows the
header half live. The card half rests on an 80×24 frame with a pinned theme, so a card at 40
columns is still unmeasured — the same width gap M156 and M157 both recorded.

## Verification

- `cargo test -p epsilon-tui --lib` — **362 passed, 0 failed** (1 new).
- Control: 360 passed, 2 failed.
- Full battery: see `docs/history.md`.
