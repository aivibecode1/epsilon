# M268 — one `/goal`, two frontends

M267 gave the window a command parser and had to refuse `/goal edit`, `/goal complete` and
`/goal blocked` **by name**, because the rules were ninety lines inside the terminal's command
handler. This milestone moves them out, so the whole goal feature is agent-level rather than
terminal-level — and a test caught me misreading a rule along the way.

## The parser was a frontend's private property

`run_goal_command` in `epsilon-tui` parsed `/goal …` into a `GoalOp` in about ninety lines: the
verb table, the `rounds <n>` sub-form, the replace-don't-append gate list, and the usage
messages for the malformed shapes. All of that is **rules**, and none of it is about pixels.

So it now lives in `epsilon-runtime::goal_command`, and both frontends call it:

```rust
pub enum GoalCommand {
    Show,                                  // no argument
    Help(&'static [&'static str]),         // /goal help
    Apply(GoalOp),                         // do this to the goal
    Invalid(String),                       // not a line this understands — say why
}
```

Text in, enum out. No agent, no window, no runtime — which is exactly what was missing when
only the terminal had a copy, and what made its coverage "whatever that handler's tests
happened to press".

**What stayed in each frontend is the application, and the two are genuinely different.** The
terminal starts the first round on a create or a resume, through `continue_goal`. The window
arms the goal and lets the round driver notice. Applying an operation is a frontend's business;
deciding which operation a sentence means is not.

The window therefore gains `/goal edit`, `/goal complete`, `/goal blocked` and `/goal help` for
free — and a gate declared by typing it now lands on the goal band, where `1 gate(s)` is the
fact that distinguishes *declared* from *the word appeared somewhere*.

## The mistake this milestone made, and the test that caught it

While moving the parser I read this in the old handler:

```rust
if head.eq_ignore_ascii_case("pause") && tail.is_empty() { … }
```

and concluded that `/goal pause whatever` — falling through to `Create { objective: arg }` —
**silently replaced your goal with one called "pause whatever"**. I called it a defect, wrote
the rule out as a guard, and wrote a milestone section about it.

**It is a deliberate rule, and the terminal has a test named after it:**
`an_objective_that_starts_with_a_verb_is_still_an_objective`, whose comment reads *"only a verb
that is the **whole** argument is a verb, so this creates a goal rather than pausing one"* —
with the fixture `/goal clear the failing gates and pause nothing`.

That test failed. It did exactly what it was written to do, which is the whole argument for
keeping behaviour-locking tests next to the code they lock: I read the code, formed a confident
opinion, and was wrong about a decision somebody had already made deliberately and written
down.

**A verb is a verb only when it is the whole argument**, and the cost of that is real — a stray
word after a verb produces an objective containing it. The trade is deliberate: an objective is
a sentence, and a harness that guesses which sentences were meant to be commands destroys
goals. The rule now lives in `epsilon_runtime::goal_command` **with the reasoning attached**,
and `an_objective_that_starts_with_a_verb_is_still_an_objective` is now asserted there as well
as in the terminal.

`/goal blocked <code>` with **no reason** is still refused, because the `blocked` arm requires a
non-empty tail and that shape is a genuine malformed command rather than an objective.

## A quiet path now says something

`Handle::try_current()` failing meant both the goal round driver and `/compact` **did nothing**
— the round driver put its pending flag back and returned. In the product eframe runs inside a
runtime, so this is the test-only path, and rare is exactly when a silent failure costs most:
nobody looks for the one path nobody exercises. That silence is how the channel defect stayed
invisible for six milestones. It now says *"cannot continue the goal: this window is not
running on an agent runtime"*.

## Verification

Battery `m268` — **GATE PASS: 1,897 tests across 77 binaries, 0 failed**, all seven gates,
**attempt 1** (`audit/evidence/gate-m268.log`). Up from 1,889 at M267.

- `every_verb_reaches_its_operation` — the eight verbs, as an exhaustive table against the
  operations they must produce.
- `an_objective_arrives_whole_even_with_spaces` — the sentence rule, which is the reason the
  parser was written by hand in the first place.
- `the_gate_list_is_replaced_not_appended` — the replace rule, including that a bare
  `edit gate` means "no gates" rather than "a gate whose command is the empty string".
- `a_malformed_line_is_refused_rather_than_guessed_at` — four malformed shapes, each refused
  **with a reason**.
- `an_objective_that_starts_with_a_verb_is_still_an_objective` — the rule above, asserted at
  its new home as well as in the terminal.
- `a_bare_goal_shows_and_help_lists_the_verbs`.
- `a_gate_declared_by_typing_it_lands_on_the_goal_band` — through the window, asserting on the
  band, because "declared" and "printed somewhere" are different claims. **It creates the goal
  first**: editing the gate of a goal that does not exist is refused, and should be.
- `a_malformed_goal_verb_says_why_instead_of_doing_something` — the same rule from the other
  frontend.

## What nothing exercises

- **The terminal's `/goal` tests were written against the handler, not the parser**, so this
  move changes what they cover. They pass, which is the evidence that the behaviour is
  unchanged — and one of them **failed this milestone**, which is the better evidence: it caught
  a change I had believed was a fix.
- **`/new` is still refused in the window.** The terminal's starts a new session *file*
  (M221) and this window has no such action; mapping it onto the nearest thing that exists
  would be a different feature, not a smaller one.
- **The window still does not start the first round itself** when a goal is created — it arms
  the goal and the round driver picks it up on the next pump. That is the terminal's
  behaviour with the `continue_goal` call removed, which is correct for a window that pumps
  continuously, and is not yet checked by a test.
- **`GoalCommand` carries no session id**, so a round in flight when the reader switches
  sessions still lands on the new one. Recorded, not fixed.