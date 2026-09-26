# M156 — the frame test that would have caught M155

## Why this milestone exists

M155's last section said, in its own words, that the cheap fix "has not been written yet".
This is it. Three milestones running (M152, M153, M155) ended by recording the same gap: the
state tests cannot see what a user sees, and every one of those three defects — a running
card that looked hung, a header built from the body's first 400 characters, every tool call
drawn twice — was invisible to them and obvious on a screen.

M155 is the proof of the pattern rather than an instance of it. Its state tests were *green*:
`cards[0].status == Ok`, `ended_at_ms.is_some()`, a duration, a body. A duplicate card and a
single card satisfy all of it. The thing that distinguishes them is only ever visible as a
**count of rows on a screen**.

## What the tests assert

`a_completed_tool_call_is_drawn_once_on_a_real_frame` drives the production event stream —
assistant message committing a call, `ToolExecutionStart`, then the result — through
`render_rows`, which is the same `draw` the event loop calls. Then it counts:

```rust
let command_rows = rows.iter().filter(|r| r.contains("echo epsilon-ready")).count();
assert_eq!(command_rows, 1, "the tool call is drawn once; found it on {command_rows} rows");
```

and, because `epsilon-ready` is also the command's own argument, checks the total is two rows
— one title, one body — rather than two of each.

`an_orphaned_tool_result_still_reaches_the_frame` is the counterpart: M155's fix must not
become "drop results that have no call", because a replay or a branch switch would then lose
a tool's output silently.

## The control is the point

Reverting M155's in-place branch — nothing else — and re-running **only these two tests**:

```
test result: FAILED. 1 passed; 1 failed
  a_completed_tool_call_is_drawn_once_on_a_real_frame
    assertion failed: the tool call is drawn once; found it on 3 rows
```

The frame test fails on the exact defect, and `an_orphaned_tool_result_still_reaches_the_frame`
correctly still passes, because the orphan path is untouched by the control. That asymmetry
is the evidence that each test is aimed at its own property rather than failing together.

## Two fixtures, and why they had to be new

`assistant` hardcodes `{"command": "echo hi"}`, and `agent_event_tool_result` derives the
`tool_call_id` from the tool name. Both are fine for state assertions and useless here: a
card's headline is rendered from the *arguments*, so a frame test that wants
`$ echo epsilon-ready` on screen needs a fixture that can say that, and a test of a
*completed* call needs to address a call id that is not `tc_bash`. Both fixtures now live at
module scope rather than inside `mod tests`, because `app/golden.rs` needs them.

## What I got wrong doing this

Twice, and both cost real time:

- I put `assistant_calling` inside `mod tests`, where `golden.rs` cannot reach it, and then
  "fixed" it by deleting a range from the first helper to the first `// ---- M142` marker.
  That range contained the `assistant` fixture that **thirty-odd tests depend on** — 8,716
  characters, not the ~1,300 I expected. Recovery was `git checkout HEAD -- state.rs` (M155
  was committed, so nothing was lost) and re-applying the two helpers by anchor.

The lesson is one I have already written down and then broke: *never reassemble a Rust file
with ad-hoc line-range surgery.* The previous instance damaged `state.rs` in the same way.
Recorded again because repetition is the evidence that the rule was not specific enough.

## The other half: a test that was never a flake

Running the battery for this milestone, `pi_import_carries_every_source_entry` failed **again** —
the third battery in a row, which I had twice written off as load. This time I read the test
instead of dismissing it, and the cause was in its own doc comment:

> the threads inside one process share the fixtures they are meant to share and are
> serialized by the test harness anyway

Both halves are false. Rust's harness runs tests in a binary **in parallel threads by
default**, and these tests share no fixtures — each calls `work_dir()` and builds its own
source. Two of them build the **same filename**, `large-session.v2.jsonl`, and one does
`remove_file(&imported)` on a path another is renaming into.

The previous fix had keyed the path on the **process id**, which fixed a cross-process race
and left the in-process one untouched. The symptom was identical, which is exactly why it read
as load flakiness. The tell that it was never load: **it always passes in isolation** — a
filtered binary has no second thread to collide with — and fails only in a full run. Load
changes the timing; it does not cause the race.

**The control could not reproduce it on demand.** With the per-process path restored, four
consecutive runs of that binary in isolation all passed. So the honest control is not a failing
run; it is an invariant test:

```rust
#[test]
fn two_threads_never_share_a_work_directory() {
    let mine = work_dir();
    let theirs = std::thread::spawn(work_dir).join().unwrap();
    assert_ne!(mine, theirs, "two tests in one process must not write the same fixtures");
}
```

Against the per-process path it fails deterministically, naming the collision. `work_dir` is
now keyed on the thread as well as the process, and the round-two reasoning in the doc comment
is corrected rather than left standing as the explanation for the next occurrence.

## What still nothing exercises

`render_rows` is a frame at a fixed 80×24 with a pinned theme. It does not catch anything
that depends on terminal width, and the console probe remains the only thing that measures a
real conhost window. A defect that only appears at 40 columns, or only under a real
`conhost`, is still outside CI.
