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

## Verification

**Battery `m235` — GATE PASS on attempt 1, 1,703 tests across 72 binaries, 0 failed**
(`audit/evidence/gate-m235.log`). Up from 1,687: sixteen desktop tests, of which four are the
end-to-end ones. clippy clean across 13 crates; check-links and check-line-endings clean.

The built binary was launched against this repository: it composes an agent, opens, and stays
up with empty stderr.

## What nothing exercises

- **Keystrokes.** The draft is set and sent through `set_draft`/`send_draft` — the same
  functions the composer calls — but the route from an egui `Event::Text` into the draft is
  **not** driven. Four attempts through `RawInput.events` failed to move a character and the
  cause is not isolated; the file says so and lists what was already ruled out. Nothing is
  known to be broken (a real window gets focus and events from winit), and nothing is known
  to work.
- **The keyboard shortcuts.** `Enter`, `Shift+Enter` and `Escape` are handled in
  `draw_composer`, and the *rules* behind them are tested through `send_draft`; the key
  consumption itself is not.
- **A window the user has resized.** The layout is a pure function with its own tests, but
  `IdeSession` has never repainted at a size other than the ones the layout tests cover.
- **Two windows at once.** `IdeSession` was made `Sync` (interior mutability) precisely so a
  second window *could* hold one; nothing opens one.
- **The sidebar and editor panes** still draw placeholder rows. They are labelled, and the
  status line does not pretend otherwise, but they are not features yet.