# M264 — `/compact` worked once, and the window never showed what a tool printed

Two findings from the queue the M262 review sweep wrote down, both about **a capability that
exists and never completes**.

## `/compact` could run exactly once per session

`compact_running` was set when the work was spawned and **never cleared**. Five mentions in the
crate: one writer, one reader, no reset.

```rust
self.compact_running = true;
self.ui.push_notice("compacting context…");
tokio::spawn(async move {
    let mut produced = Vec::new();
    let _ = a.compact_now(&mut produced).await;   // …and never clears it
});
```

Every later `/compact` answered **"compact already running — wait for it to finish"** — for a
task that finished long ago, for the rest of the session.

**Why it survived 263 milestones.** The comment above the spawn described it as intended:

> *we spawn a fire-and-forget that resets the flag after a short grace; … For now the spinner
> stays until the next successful `/compact`.*

A comment that names the defect and calls it a design decision is the hardest kind of stale
evidence to spot, because it reads as deliberate. The refusal is also *plausible* — it names a
real state — so nobody suspects it and there is no way for a user to discover it is wrong. That
is the wrong-but-plausible state `AGENTS.md` exists to forbid, wearing the costume of a
feature.

The field is now an `Arc<AtomicBool>` and the task clears it when it finishes. That is the
whole fix, and it is the honest shape: the flag already means *"a compaction is in flight"*,
and this is how a detached task says it is no longer.

**The error was discarded too.** `compact_now` returns `Result`; `let _ =` threw it away, and a
failure reaches the frontend as a generic notice that cannot be told apart from any other — so
a failed compaction ended the spinner and said nothing about why the context did not shrink. It
is named now, through the same `notices_tx` channel the shell-escape path already uses.

## The desktop window never showed what a tool printed

`AgentEvent::ToolExecutionUpdate` reached the reducer's `_ => {}` arm and was dropped. So the
window showed

```
▸ bash {"command": "cargo test …"}
✓
```

and never a line of what happened in between. **For `cargo test` that is the difference between
"it ran" and "it passed"** — and the terminal has shown it since M63.

Worse, the **replay path showed results** (`from_messages`), so the same session showed two
different transcripts before and after a session switch: live showed the *call*, replay showed
the *result*. One field now means "what this tool produced", on both paths.

### The single-slot bug this uncovered

`live_tool` was one `Option<usize>` — "the row for the call currently running". Two tools in
flight is not exotic: a `bash` parked on a permission ask is exactly when a second call
arrives, and the FIFO approval channel permits it. Two interleaved calls ended out of order and
the first result was marked onto the second row.

Rows now carry the agent's own `tool_call_id`, which was in hand the whole time
(`ToolCall.id`, `ToolResultMessage.tool_call_id`) and simply was not kept. **The field is
deleted** rather than left with no reader — the sixth instance of "faithful port, no production
caller", and the first this round that was *created* and then removed inside one change.

### Output is a bounded tail, not the whole stream

`partial` is a **delta**, so a long build arrives as hundreds of updates and the row would grow
without bound in a view already capped at `MAX_ROWS` that cannot scroll to them all. 40 lines,
the same bound and the same reason as the terminal's `CARD_STREAM_TAIL_LINES` — one number, two
frontends, rather than two that drift.

## The finding worth more than the feature

The output **drew correctly on the first run and the frame test still failed.** The reducer
tests passed; the row carried the right text; the frame showed

```
[agent] ✓ {"command":"cargo test"}
```

and no output. The cause was not the drawing — it was that the drawing never reached
`FrameSummary.rows`, which is the **only** window anyone has into what this window shows. A
line the summary does not carry is a line no frame test can ever see.

> Painting without recording is how a feature ships unverified. `FrameSummary` is the
> instrument; a thing the instrument cannot see did not happen as far as the suite is concerned.

This is the sixth time this repository has been bitten by it (M196: a search field drawn inside
another feature's renderer; M248: panes correct, contents painted on top of each other), and
the third time it has turned up **in this one round's own work**.

## Verification

Battery `m264final` — **GATE PASS: 1,876 tests across 76 binaries, 0 failed**
(`audit/evidence/gate-m264final.log`). Up from 1,872 at M263. All seven gates green.

**Two failures on the way there, and both logs were kept rather than overwritten** — the rule
this repository states twice, which I broke in round 1 by re-using a base label and did not
break this time:

- `gate-m264` — **Avira `LNK1104`** on `epsilon_core-4a8081d0361ce74c.exe`, the same pinned
  path as the round before. A machine fault, not a code failure; the documented remedy is
  fresh test-profile metadata. Kept at
  `audit/evidence/gate-m264-FAILED-LNK1104.log`.
- `gate-m264r` — the retry was **killed by the harness's job runner** mid-`test`
  (`3221225786`), not by a failing test. Kept at
  `audit/evidence/gate-m264r-interrupted.log`, because "the harness was killed" and "the tests
  failed" are different facts and only one of them is true.

The knob that clears the first one costs a full test-profile rebuild: the test leg ran
**1,489 s** against ~675 s warm.

The four new tests, and what each is for:

- `a_session_can_compact_more_than_once` — polls the observable condition rather than sleeping
  and hoping, and asserts the flag was genuinely *set* first, or it would pass against a build
  that never started a compaction at all. **Negative control: with the clear removed it fails
  with "the flag must clear when the compaction finishes".**
- `streamed_tool_output_lands_on_the_row_that_produced_it` — two calls **interleaved**
  (`c1, c2, c1`), because the single-slot bug cannot be expressed with one call.
- `streamed_tool_output_keeps_a_bounded_tail` — the bound is the property, not a detail.
- `a_tools_output_is_painted_under_its_row` — **on the frame**, because the three unit tests
  above were green while the window showed nothing.

## What nothing exercises

- **A tool that streams nothing still looks frozen.** This fixes the shape of the symptom, not
  every cause: a long `cargo test` that emits no intermediate update shows a static row until
  it ends. The terminal has the same limit.
- **The window still has no slash commands**, so `/compact` — which this milestone makes work
  twice — remains reachable only from the terminal. That is row A38 of the defect ledger and
  it is the largest parity gap left.
- **Desktop per-frame cost is untouched and unmeasured.** The reliability review found
  `Agent::goal()` deep-cloning every session entry twice per frame and the sessions pane
  re-summarising every file on every paint (202 ms at 500 sessions, by the project's own
  recorded number). **No desktop frame cost has ever been measured in this repository**, so
  sizing that fix needs a measurement before it needs a patch.