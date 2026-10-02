# M259 — the window can control a goal, and a fifth reference arrived

Two things: a feature, and a correction to what we thought we were given.

## The command table

M258 made the desktop **watch** a goal. It still could not **control** one: the terminal has
had `/goal` with create, pause, resume and clear since M240, and the window had no verb for
any of them. For a product whose thesis is one application serving both frontends, that is
the asymmetry.

The palette's four hardcoded entries became a table:

```rust
struct Command { id: &'static str, label: &'static str, hint: &'static str, action: PaletteAction }
```

**The `id` is the point.** Without a stable name every consumer — the palette, a future key
binding, a `/help` — has to match on the *label*, and a label is what a reader sees and
therefore what gets reworded. `opencode` builds its palette from its command ids for exactly
this reason.

It had a real consumer from the first commit, which took a deliberate choice: a palette row
records `command:<id>` in its hotspots, so a test presses `command:goal.pause` rather than
the words on screen. Before that the field was dead — clippy said so — and the alternative
was to delete it.

`goal: create` takes **the palette's own query as the objective**: type the objective, choose
the verb. One gesture, and the same one `/goal <objective>` is in the terminal. With nothing
typed it **refuses and says why**, because a goal with no objective runs to its round cap and
gets nowhere.

Every verb's outcome lands in the transcript. A verb that does nothing looks exactly like a
verb that worked, which is the state this window has produced five other ways and has now
been tested against five times.

## Two tests that were wrong before they were written

**A refusal test that passed on a build where the command never ran.** `goal_verbs_without_an_agent_say_so`
asserted the frame contains *"no agent attached"* — which the agent-less panel prints as its
**empty state**. It passed under the negative control, which is exactly how a control tells
you it is not measuring what its name says. It now asserts the window's own refusal sentence,
which the empty state does not also use. The same trap `goal-gates.md` records about a
`FakeHost` test, one layer up.

**A wording assumption.** `a_goal_verb_with_nothing_to_change_says_so` expected the window's
`Ok(None)` sentence and failed: `goal_apply` answers a pause with no goal as an **`Err`
carrying the contract's own words**. Both are correct; the assertion now checks the *shape* of
the refusal — it says it refused, and it names the reason — rather than one wording, because
a test keyed on wording fails for a reason unrelated to what it was testing.

## A fifth reference, and what it actually is

`references/codewhale-ratatui-main` is **not a ratatui application**. It has no `src/main.rs`,
no `[[bin]]`, zero async and zero threads; `DESIGN.md:81-86` states that the *host* owns
events, permissions, queue mutation, Markdown, clipboard, persistence **and the event loop**.
It is a component library, and the app-shaped code is in `examples/`.

That does not make it useless — it makes it useful for **mechanisms** rather than shape.
The highest-value ones for our terminal frontend:

- **`src/testing.rs` is shipped in `src/`** and records **the role each run was painted with,
  not hex**, across 9 terminal profiles × `[40, 80, 120]` columns. *"A token value change
  does not rewrite every snapshot, but painting `Muted` where `Danger` belongs does."* Our
  rule — *assert on the rendered frame, not on state* — as a one-liner per component.
- **`rule_violations(frame)`**: machine-checked invariants over the rendered buffer (hairline
  count, banned glyphs, colour depth the profile can show), shipped **with a control for the
  control**.
- **`every_paint_impl_has_a_gallery_entry`**: greps every `Paint` impl and fails if no test
  drives it. *"A component added without a gallery entry would slip past it: fail instead."*
  That is our M196 defect — a feature drawn inside another feature's renderer, fully tested,
   never rendered — made structural.
- **`ApprovalState::unarmed()`/`.arm()`**: a key arriving before a dialog's first paint must
  not answer it. **Worth checking against ours.**
- **`FrameBudget`**: components request when they next need a frame; `None` sleeps until an
  event. Idle costs zero redraws — and `None` vs `Some(166ms)` is precisely the knob that
  separates *slow* from *lost input*, which is our unreproduced freeze.

And one thing **not** to take: its `Keymap::lookup` **has no production caller** — grep finds
hits only in its own tests and in a gallery that uses it for hints. The *design* is the best
idea in that repository (one table that is simultaneously the dispatch table and the
rendered footer, so `/hotkeys` cannot drift), but the mechanism is a port of the upstream
engine's dispatch and only the rendering half came along. That is the "faithful port, no
production caller" class — in a reference we are reading for ideas.

All of it is recorded in [`agents/memory/references-five.md`](../../agents/memory/references-five.md).

## Verification

Battery `m259` — **GATE PASS on attempt 1: 1,847 tests across 76 binaries, 0 failed** (`audit/evidence/gate-m259.log`). Up from 1,842.

**Negative control:** with the command actions disarmed, three goal-verb tests fail and the
band tests still pass. `goal_verbs_without_an_agent_say_so` **passed under the control** — that
is how the wording bug above was found, and it is the reason the control was run against the
new tests specifically rather than as a formality.

**Reinstall:** none. `epsilon-cli` and `epsilon-tui` are untouched.

## What nothing exercises

- **The palette's command rows are still not clickable.** Rows carry ids and hotspots now, but
  the click path is resolved against last frame's rectangles for *entries*, and the new tests
  drive verbs through the seam rather than through a pointer. A reader who sees
  `goal: pause` and reaches for it may not get it.
- **`/hotkeys` parity is still a test, not a type.** The command table has ids; nothing
  generates a help row from them yet, so the table cannot drift but nothing would notice.
- **The reference findings are recorded, not acted on.** Every item in the
  `references-five` note is a plan; none of it is in the code, and none of it has been tried.
- **The approval arm-after-paint guard is unchecked in our TUI**, and our TUI has approval
  prompts.