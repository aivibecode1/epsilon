# 04 — The TUI

The reported defect, in the owner's words: *it freezes and shows no input or output the
moment you start typing, in both CMD and PowerShell, even though tokens are clearly being
consumed underneath.*

**This document retracts the previous explanation.** The earlier assessment, now kept at
[`history/13-tui-superseded.md`](../history/13-tui-superseded.md), concluded the cause was
a `bash` permission dialog that silently swallowed keystrokes and presented three console
readbacks as decisive evidence. I re-ran those measurements. **The conclusion does not
survive.** Below is what I measured, what that rules out, what remains, and the honest
state of the question.

## What I measured

`scripts/console-probe.py` opens a genuine conhost window, attaches to it, launches the
TUI from the shell inside that console, injects real `KEY_EVENT` records, and reads the
screen back. I ran it against **two** binaries:

- a fresh **debug** build (`D:/eps-target/debug/epsilon.exe`), and
- the **installed release binary** (`~/.cargo/bin/epsilon.exe`, 20 MB) — the one the
  owner actually runs.

Both behave identically.

**Typing works.** After `type_text("say hi")` the composer read `• say hi` and the cursor
had advanced to column 8 — exactly six columns for six characters. The keystrokes were
received, routed and rendered.

**Typing during startup works.** `--early` mode types while the program is still
initialising. The composer read `• hello second` — both words, in order.

**Submitting works, and the run completes.** `turn_screen` showed the run streaming with
live header figures (`↑100 ↓50`, `ctx <1%/164k`, `$0.0001`), then the permission dialog.
Pressing `y` produced `settled_screen` with `ok · 6293ms` and `Run complete.`

**The draw path is fast.** `--tui-bench` reports ~2.9 ms/frame at 20 messages, ~3.1 ms at
100, ~3.7 ms at 300. A frame is not the freeze.

## What this rules out

**The previous root cause is wrong.** The claim was that the dialog gives the user no
feedback, so keystrokes vanish and the user concludes the program has hung. But:

- `app/panels.rs:637-647` paints `y allow once`, `a always (session)`, `n deny`,
  `e deny + guidance` in the dialog body;
- the footer repeats them — `turn_screen`'s last line reads
  `approval · y allow once · a always · n deny · e deny + guidance`;
- and pressing `y` demonstrably works.

The dialog is not silent and the keys are not swallowed. A user who reads the screen is
not misled. That claim is retracted, and the retraction matters more than the finding it
replaces, because the previous document's readbacks were presented as decisive.

**The frame-ceiling starvation hypothesis is refuted.** The loop drains input first
(`event.rs:123-134`) and its `select!` is `biased` with `term_events.next()` as branch
one (`event.rs:190-192`), above `events_rx.recv()` (`:215`). A saturated event stream
loses to input on every pass. The loop cannot spin without drawing either, because the
frame decision at `:163` is reached on every iteration. This is a correct design and it
should not be "fixed".

**The git-branch read is irrelevant.** `cached_branch` (`app/session.rs:117-124`) has a
2-second TTL, so at 30 fps it is 0.5 reads/second of a ~30-byte file, and `git.rs` reads
the file directly rather than spawning a process — deliberately.

## What is real

### T1 — the per-frame context mirror deep-clones the whole transcript. *Measured.*

`app/event.rs:169` calls `App::mirror_context_size` on **every drawn frame** — up to 30
times a second, on the event-loop thread, which is the same thread that must read the
keyboard. It reaches `Agent::estimated_context_tokens` (`core/src/lib.rs:1934-1937`),
which calls `self.history()`:

```rust
pub fn history(&self) -> Vec<Message> {
    self.history.lock().map(|h| h.clone()).unwrap_or_default()
}
```

A full deep clone of every message — every `String`, every `ContentBlock`, every tool
result — computed to produce one integer, and then walked again by `anchored_tokens`
(`compact.rs:46-53`).

`--tui-bench` **structurally cannot see this**: it calls `draw_frame` directly, and its
`bench_app` holds no `Agent`, so the mirror never runs. Its 2.9 ms is an understatement
of a real frame by exactly this cost.

I added `crates/epsilon-core/tests/context_mirror_cost.rs` to measure it (commit
`47ca904`). Result on this machine:

| Transcript | Cost per mirror |
|---|---|
| 20 messages | 16.9 µs |
| 200 messages | 216.7 µs |

**12.9× cost for a 10× larger transcript.** That is the signature of a whole-transcript
scan rather than the anchored walk it intends to be. The test's second assertion — that
the cost must not *track* transcript length — fails today, deliberately. It is the
tripwire for the fix.

**And the honest part: at 216 µs this is ~0.7% of a 33 ms frame.** It is a real defect
and it is O(session) on the input thread, so it gets worse exactly as a session gets
longer — which is the *shape* of the complaint. But it does not by itself explain a total
freeze at the sizes I measured. I am not going to ship it as "the" cause.

### T2 — the harness gap that let this class of bug survive. *This is the real finding.*

**No test or probe has ever run this TUI while a turn is genuinely in flight.**

The reason is mechanical. `crates/epsilon-llm/src/faux.rs:145-148`:

```rust
async fn next(&mut self) -> Option<StreamEvent> {
    self.pending.pop_front()
}
```

No delay. The faux provider emits an entire turn's events instantly, so every probe run
to date — including the three above — has photographed a turn that was over in
microseconds. The state the owner is describing, a run streaming over seconds while they
type, has never been executed by anything.

`SlowProvider` (`core/src/testing.rs:71-92`) does not close the gap: it sleeps *before*
`stream` opens, so it can delay the start of a turn but not its duration. And it is
reachable from `rpc_mode.rs:412` and `epsilon-server/src/lib.rs:492,639` — but **nothing
in the CLI wires it to the TUI**.

So: every green result this repository holds about the TUI was produced by a provider
that cannot reproduce the bug. That is the finding I would put first in any handoff.

### T3 — per-event O(session) work in the apply path. *Read, not measured.*

`restale_finished_cards` (`app/view.rs:117-125`) scans every transcript line, and it is
called from `apply_with_scroll` (`app.rs:938-948`) on **every agent event** — not per
frame. A live provider emits hundreds of events per second. `card_restale` is a
`Vec<u64>` with linear `contains` (`app.rs:233`), so this is O(session) per event with
an O(n) scan inside. Same class of defect as T1, different multiplier.

### T4 — no panic hook and no restore guard.

`app.rs:906-926` calls `ratatui::try_init()` and `try_restore()` with no `Drop` guard and
no panic hook. A panic leaves raw mode, the alternate screen and a hidden cursor — which
a user experiences as *exactly* a freeze, and which produces no output because the
terminal is in a state nobody is drawing to. Cheap to fix, and it converts an
inexplicable failure into a readable one.

## The one real user-facing bug I did confirm

Independent of the freeze, and inherited from the previous audit but verified by me: a
killed TUI leaves a stale `.epsilon/sessions/tui.lock`, and the next launch blocks for
30 seconds on `STALE_AFTER`/`ACQUIRE_WAIT` (`lease.rs:50,59`) showing a truncated
"waiting for this session to be released" frame with no working input. The recorded pid
is deliberately never probed for liveness. I hit this myself — every probe run needs
`rm .epsilon/sessions/tui.lock` first, because `console-probe.py` ends with `taskkill /F`.

A crashed TUI bricks the workspace for 30 seconds per launch. That is a real defect and
it is not the one that was reported, which is exactly why it went unnoticed.

## What to do, in order

**H1 — build the harness first.** Without it, no TUI claim afterwards is verifiable, and
this audit would be repeating the mistake it is correcting. Specifically: a provider that
streams over a controllable duration, reachable from the TUI, plus a conhost test that
types *while a turn is in flight* and asserts that either the character is visible
somewhere or the screen says why it was not.

> **"Nothing happened" must never be a state the UI can be in.** That is the assertion
> worth having, and it is the one no existing test makes.

**H2 — fix T1.** Hoist or cache the mirror so a frame does not clone the transcript.
`anchored_tokens` already takes `&[Message]`; the clone is the caller's choice, not the
function's. The tripwire test already fails and will pass when this is done.

**H3 — fix T4.** A panic hook plus a `Drop` guard. Small, and it makes the next
occurrence of this bug diagnosable instead of mysterious.

**H4 — fix the stale lock.** Probe the recorded pid, or make the lease stealable after a
shorter window.

**H5 — then re-measure with H1 in place** and let the measurement, not this document,
say what the freeze is. I expect T1 + T3 to be a large part of it once a real streaming
turn is actually being driven. I am not willing to claim that from inference when the
harness to prove it is a day's work.

## Honest limits

- **I did not reproduce the reported freeze.** Typing, submitting, streaming and
  completion all worked in every configuration I could construct with the faux provider.
  The most likely explanation is T2 — the bug lives in a state the repo cannot reach —
  but "most likely" is not "reproduced", and this document does not claim otherwise.
- The 0.5–3 MB transcript figure in circulation is an estimate. The actual size of a
  real session's `.epsilon/sessions/*.jsonl` was not measured.
- Only `cmd` was measured directly, though M177 measured PowerShell. Windows Terminal
  was not measured at all.
- The `?` characters in probe readbacks are an artifact of reading a non-UTF-8 console
  buffer. That is unconfirmed, and it is the kind of thing that hides a real encoding
  bug.
