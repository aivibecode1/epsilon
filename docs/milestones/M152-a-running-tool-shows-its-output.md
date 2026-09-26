# M152 — a running tool shows its output while it runs

## What

The owner said the TUI was far below the references. A ranked, cited gap analysis against
`references/pi-0.85.1` put "live tool output inside the card" second, and reading the code
confirmed it exactly:

`AgentEvent::ToolExecutionUpdate` already received every streaming chunk. It reduced the whole
thing to the **last non-empty line**, put that in the one-line status header, and discarded the
rest. The card body was never touched.

So a three-minute `cargo build` drew a static card for three minutes, with one truncated line
of its own output crawling across the header. From the user's seat that is not a missing
feature — it is indistinguishable from a hang, which is the complaint that started this.

## What changed

`ToolExecutionUpdate` now also writes into the card. Two details that are easy to get wrong:

- **`partial` is a delta, not a cumulative buffer.** The card accumulates, keeping
  `CARD_STREAM_TAIL_LINES` (40) recent lines. A long build emits tens of thousands of lines;
  keeping them all turns the transcript into a memory leak with a scrollbar. The result
  overwrites the tail wholesale on settle, so nothing streamed is ever the record.
- **Only while `Running`.** A settled card's body belongs to its result, so accumulation resets
  rather than prepending — otherwise every line of a long command appears twice.

## Negative control

Reverted only the body write, leaving the accumulation computed and dropped — the pre-M152
shape:

```
test result: FAILED. 1 passed; 3 failed
  a_running_tool_card_keeps_only_a_bounded_tail                    FAILED
  a_running_tool_card_shows_the_output_it_has_produced_so_far      FAILED
```

**The one test that passed is the one that should have.** `the_result_replaces_the_streamed_tail`
covers the settle path, which the control does not touch — so the control isolates exactly the
line under test rather than failing everything at once.

## What this did not do, and why

The same analysis proposed a **live elapsed timer** on the running card. This milestone leaves
it alone, because the code already argues against it in a comment worth reading
(`state.rs:174-177`): a `now - started` duration freezes at whatever value the frame that built
the row happened to see, because rows are cached until something invalidates them — and a number
that is sometimes live and sometimes stale is worse than no number.

That objection is correct, and it is not resolved by adding a clock. It is resolved by making
the frame repaint and the row invalidate together, which is a render-cache change with its own
risk. The field's own doc comment claims `None` "makes the duration live rather than absent",
which contradicts the method two lines below. **That contradiction is left in place and recorded
here rather than quietly resolved**: one of the two comments is wrong, and which one should
change is a decision for whoever does the render-cache work, with a measurement behind it.

**What nothing exercises:** nothing renders a running card in a real console yet — the probe
cannot drive a live tool, and there is no test that paints a `Running` card to screen and reads
the pixels back. The state assertions prove the card holds the text; they do not prove a user
sees it. That is the honest gap, and it is the same class as M149's: a field exists, the
transcript consumes it, and the paint path is assumed.

## Verification

- `cargo test -p epsilon-tui --lib` — **334 passed, 0 failed** (3 new).
- `cargo clippy -p epsilon-tui --all-targets --features real -- -D warnings` — exit 0.
- Full battery: see `docs/history.md`.
