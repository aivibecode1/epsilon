# M267 — the window had no slash commands, and the goal round channel was two channels

## What the window did with `/compact`

Nothing. `send_draft` handed its text to `session.submit` unexamined, so a reader who knew
the terminal typed `/compact` and the model was asked to compact **as a prompt**. It answered
something plausible, the transcript filled with prose, and nothing anywhere said the window had
never heard of a slash command. The same class this repository tracks by name — *it worked, in
the pane that says it worked*.

The answer is three outcomes, and the third is the point:

| typed | what the window does |
|---|---|
| a command it can do | runs it, and the transcript says what happened |
| **not** a command | sends it — the reader's words are the reader's words |
| a command the terminal has and this does not | **refused, by name** |

A window with three commands is a smaller product. A window that quietly forwards the other
thirty-six to the model is **a window that lies**, and that is the more expensive failure.

**Two refusals, because they are two different facts.** `/timeline` is a gap in *this*
product and works in the terminal, so the window says so. `/timelin` is nobody's command, and
telling the reader to go and look in the terminal would send them there for something that is
not there either.

### Why the match is exhaustive, and why that is the design

`Command` lives in the terminal crate and the window already depends on it (ADR-020, for the
palette). Matching it exhaustively means **adding a command to the terminal breaks this
module's build** until someone decides what the window does with it — which is the decision
that was being skipped, invisibly, one command at a time.

It worked immediately. The first compile of `head_of` failed with:

```
error[E0004]: non-exhaustive patterns: `Command::Help` and `Command::Quit` not covered
```

Two commands the table had forgotten, caught by the compiler rather than by a reader. A `_ =>`
arm would have compiled fine and swallowed both.

## The bigger find: an unattended goal never continued itself

`/compact` needed to report back when it finished, over a channel. It reported nothing. One
`eprintln` in the drain — the move this repository's own notes prescribe — showed the task
running, finishing, and sending, with the drain returning `None` every time.

```rust
goal_round_tx: tokio::sync::mpsc::unbounded_channel().0,   // the .0 of ONE call
goal_round_rx: tokio::sync::mpsc::unbounded_channel().1,   // the .1 of ANOTHER
```

**Two different channels.** The round driver spawned a task that sent its answer where nothing
was listening; `take_goal_round` read from a channel nothing wrote to. It could only ever
return `None`.

So: **an armed goal in the desktop window has never continued itself.** M258 built the driver,
M259 gave the window the goal band, and the feature has been inert since — a goal set in the
terminal or the window would run its rounds in the terminal and go quiet in the window.

Both constructors had it, and it survived because **every goal test drives
`start_goal_round_for_test`, which bypasses the channel by design.** The one test that could
have shown it was the one test built not to. This is the "drive the round driver end to end"
gap that `agents/memory/desktop-ide.md` had listed as a known next slice — building it found
the reason nobody had.

### The control, and a test that had to be rewritten to be worth anything

`a_goal_round_spawned_by_the_driver_really_arrives` holds a runtime context, creates an armed
goal, pumps frames, and waits for the notice `start_goal_round` pushes.

**Its first version waited for the goal's `rounds_started` to move — and passed against the
broken channel.** `begin_goal_round` records the round *before* it sends anything, so the count
moves whether or not the answer ever crosses. It proved the spawn happened and said nothing
about the channel, which is the half that was broken. Asserting on the **notice** is what makes
it a test of the channel.

**Negative control**, with the two-channel form restored:

```
a_goal_round_spawned_by_the_driver_really_arrives ... FAILED
the round the driver spawned must arrive and be applied.
```

## `/compact` in the window

Off the paint thread, because the summariser resends the whole history and awaiting it in a
frame would freeze the window — the failure it has already produced five other ways. The answer
comes back over a channel the frame drains, and **a failure is named**: `compact_now` reporting
an error into `let _ =` is how a reader ends up with a compaction that silently did nothing.

It cannot come back as an event: the window's reducer folds neither `CompactionStarted` nor
`CompactionFinished`, so that is the channel's whole job.

## Verification

Battery `m267b` — **GATE PASS: 1,889 tests across 77 binaries, 0 failed**, all seven gates,
**attempt 1** (`audit/evidence/gate-m267b.log`). Up from 1,883 at M266: six new tests.

The first battery (`gate-m267`) failed on `check-line-endings` and nothing else — my own
PowerShell `WriteAllLines` had rewritten `desktop/src/app.rs` with CRLF, which is the trap this
repository's Windows notes name twice. Kept at `audit/evidence/gate-m267-FAILED-crlf.log`,
because "the gate caught my tooling" is a fact about the round and not only about the code.

- `a_command_the_window_lacks_is_refused_and_never_sent_to_the_model` — the defect itself,
  through the route a reader uses: the refusal is on screen, naming the command, and
  `send_draft` submitted nothing.
- `a_line_that_is_not_a_command_is_still_sent` — the over-correction. The parser's rule is
  kept rather than a looser one invented here, because two frontends disagreeing about what a
  line **means** is worse than either of them being wrong.
- `an_unknown_slash_word_says_unknown_rather_than_not_in_this_window` — the two refusals stay
  apart.
- `every_command_head_the_window_lacks_is_refused_by_name` — the refusal table is **derived
  from the parser**, the same discipline M171/M172 applied to the terminal's own sweep: a
  hand-written list is a gap waiting for a new command.
- `compact_from_the_typed_line_compacts_and_reports` — the spawn and the report back.
- `a_goal_round_spawned_by_the_driver_really_arrives` — the channel, with the control above.

## What nothing exercises

- **Only three commands.** `/new` is refused with its name rather than mapped onto the nearest
  thing that exists — the terminal's starts a new session *file* (M221) and the window has no
  such action.
- **`/goal edit`, `complete` and `blocked` are refused** although `GoalOp` carries all three.
  The window builds create/pause/resume/clear; guessing at the rest would be a second
  implementation of the goal rule, which is the thing ADR-021 exists to prevent.
- **`wire()` in the test file does not return its runtime**, so every test that needs a frame
  to be able to `spawn` must use the new `wire_with_runtime`. Two do. A third that needs it
  will hit the same wall, and the failure is a silent no-op rather than an error.
- **`Handle::try_current()` failing is handled by doing nothing** in both the round driver and
  `/compact`. In the product eframe is inside a runtime, so that is the test-only path — but it
  degrades to "the feature does not happen, with no notice", which is how this defect's test
  gap happened in the first place.