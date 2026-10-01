# M235 — the agent is in the window

M233 gave the project a desktop IDE that opens. M234 gave the second frontend the ability to
decide *what* to run. This wires them: `epsilon-desktop` composes a real agent through the
same two functions the CLI uses, streams its events into the frame, and takes an answer back.

## What landed

| Piece | Where | What it does |
|---|---|---|
| `IdeSession` | `src/session.rs` | The harness: one `Arc<Agent>`, the approvals channel, the launch |
| `Transcript` | `src/session.rs` | A **pure** reducer from `AgentEvent` to rows |
| The bridge | `src/session.rs` | A task that owns the broadcast, forwards, and asks for coalesced repaints |
| The agent pane | `src/app.rs` | Status line, transcript, permission prompts, composer |

The loop is unchanged. `epsilon_runtime::build_runtime` builds the agent,
`epsilon_runtime::resolve_launch` decides which provider, and the approvals go through the
same `ChannelApprovalUi` the terminal uses — so a permission behaves identically in both
windows because it *is* the same machinery.

## The test that mattered, and the four bugs it found

`crates/epsilon-desktop/tests/window.rs` composes a real agent in a throwaway workspace, sends
a turn to the offline provider, and asserts on what is **on screen**. The offline script calls
`bash`, so the run parks on a permission gate: the test waits for the ask to reach the screen,
answers it through the approvals channel, and waits for the turn to finish. That is the whole
loop, and it is what "the window is wired to the harness" has to mean.

It found four real defects, none of which any other test could have:

1. **`Arc::get_mut` returned `None` the moment a test held a handle**, so the frame silently
   stopped draining. The status bar knew a run was in flight and the transcript stayed empty
   forever — a window that reports the run and shows none of it. `drain` now takes `&self`,
   with the queue behind a mutex and the flag behind an atomic, and the doc says why.
2. **The reducer depended on `MessageStart`, which the loop does not always emit.** The
   offline provider goes straight to `MessageUpdate`, so every update had nowhere to land and
   the transcript stayed at zero rows. Depending on a "start" event a producer may omit is the
   same class of assumption as depending on a key nothing binds.
3. **The composer never took the keyboard.** With nothing focused, every keystroke goes
   nowhere and the window looks exactly like a frozen application — the failure this
   repository has chased across four audits in the terminal frontend. It requests focus when
   it does not have it, and the end-to-end test found it by synthesizing characters.
4. **Enter was consumed before the text edit ingested the frame's characters**, so pressing it
   submitted an empty prompt. Consuming first is what stops Enter becoming a newline;
   *acting* on it has to happen after. Both halves are now commented where they are written,
   because the order is invisible until someone types and presses Enter.

Two more came from the same file's neighbourhood: a **current-thread test runtime silently
swallows `spawn`** unless something is polling it, so a fixture that does not behave like the
product tests a situation the product cannot be in; and **approvals drawn only from the
transcript** meant a run that parked at its first gate showed an empty pane, because the
channel is the authority on what is waiting.

## What the loop looks like in the frame

```
agent
  provider: faux (offline scripted)
› say hello
! allow bash echo ?   [enter] allow   [esc] refuse
▸ {"command": "echo epsilon-ready"}
✓ {"stdout": "epsilon-ready\n"}
  Run complete.
[ ask anything · enter sends · shift+enter for a new line ]
```

Approvals sit after the rows rather than in a modal, which is where opencode and DSH both
converged, and for the same reason: **a prompt that can scroll out of reach behind a long
answer is not a prompt.** They come from the approvals channel, not the transcript, so a run
that blocks at its very first gate still tells the reader what is waiting.

## The input path, and the measurement that settled it

The test above **types.** Not `set_draft` — `egui::Event::Text` and `Event::Key{Enter}`
through `RawInput` with `focused: true`, which is what egui requires before it will deliver
a key at all.

Getting there took a probe rather than a guess, and the probe is the useful part. Four
attempts to make the composer accept a character all failed, and each was a plausible cause:
focus not requested, an unstable widget id, `RawInput::focused` defaulting to false, the wrong
`Event::Text` shape. Changing each in turn changed nothing.

So the next step was to stop asking and **measure**: render one bare `TextEdit` into an
off-screen context, feed it the same events, and see whether *it* received them. It did —
**including unfocused**, which ruled out all four candidates at once. The real cause was a
fact about immediate-mode drawing: an egui frame paints top to bottom, the transcript is
painted before the composer, so a row the composer adds during a frame cannot appear in that
frame's output. The prompt is on screen on the next repaint, which is well inside a display
cycle.

Four rounds were spent looking for a product bug that did not exist, and the thing that ended
it was a twenty-line probe rather than a fourth theory. `audit/04-tui.md` records what four
unreproduced theories cost this project; the shape of the mistake is the same one.

## Verification

**Battery `m236-r2` — GATE PASS on attempt 2, 1,704 tests across 72 binaries, 0 failed.**
Attempt 1 (`gate-m236.log`) failed `the_history_timeline_is_structurally_sound` — the
repository's own guard catching a milestone row I had inserted out of order. It is recorded
because it is the guard working as designed, on the same kind of mistake it was written for.

clippy clean across 13 crates; check-links and check-line-endings clean. The built binary was
launched against this repository: it composes an agent, opens, and stays up with empty stderr.

## What nothing exercises

- **A real window.** Every frame here is painted into an off-screen `egui::Context`.
  Keystrokes go in through `RawInput`; a real window receives them from winit. The paths
  meet at `egui::InputState` but the two have never been run together in a test.
- **A window the user has resized.** The layout is a pure function with its own tests, but
  the session has never repainted at a size other than those.
- **Shift+Enter for a newline.** `Enter` and `Escape` are covered; the modified variant is
  handled by the same `consume_key` with different modifiers and is not driven.
- **Two windows at once.** `IdeSession` was made `Sync` (interior mutability) precisely so a
  second window *could* hold one; nothing opens one.
- **The sidebar and editor panes** still draw placeholder rows. They are labelled, and the
  status line does not pretend otherwise, but they are not features yet.