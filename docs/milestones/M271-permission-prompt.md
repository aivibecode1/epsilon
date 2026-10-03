# M271 — the permission prompt was not the command

**A48**, opened by M270's measurement and closed here.

## What the reader was asked to approve

`PermissionRequest::pattern` is the **first whitespace token** of the subject
(`epsilon-core/src/exec.rs`). That is exactly right for what it is: the string a *rule*
matches on, so `Remove-Item` is the pattern for `Remove-Item -Recurse C:\Users\me`.

It is wrong for what the window used it for. The permission screen rendered the pattern
verbatim, so:

| the command the shell would receive | what the window asked |
|---|---|
| `Remove-Item -Recurse C:\Users\me` | `allow bash Remove-Item ?` |
| `echo epsilon-ready` | `allow bash echo ?` |

**No ellipsis, nothing marking it partial, and the words that decide what happens were simply
absent.** The string on the permission screen was not the string that would run — on the one
screen whose entire job is to let a reader stop that.

## One rule, two frontends — and one thing that is *not* shared

The terminal had been right all along: it reads `detail["command"]` through
`summarize_detail`. The window had no copy of that rule, so the two frontends disagreed about
what the reader was being asked to approve — which is the same shape as M267's `/goal`, and the
same resolution.

`epsilon_runtime::summarize_request` now holds **which field answers the question**. The
terminal delegates to it, and keeps its 96-column fit.

**The budget is deliberately *not* shared**, and that split was forced by a test:
`a_detail_object_prints_the_field_that_decides_the_question` asserts that an unrecognised
field's detail is truncated with an ellipsis, and it failed the moment the whole function moved.
It was right to: a 96-column terminal row and a 520-point window pane are different
measurements, and a shared one would be wrong in at least one of them. So the rule is shared and
each frontend fits its own.

> A shared constant is not automatically a shared rule. Splitting this one is what kept
> `a_detail_object_prints_the_field_that_decides_the_question` passing.

## Why this is worth a milestone rather than a one-line change

The obvious fix is to render `detail["command"]` in the window and stop. That would have left
the terminal and the window holding two copies of the same rule again — which is the defect
that produced this one, and the reason the window disagreed in the first place.

## Verification

Battery `m271` — **GATE PASS: 1,903 tests across 77 binaries, 0 failed**, all seven gates,
**attempt 1** (`audit/evidence/gate-m271.log`). Up from 1,900 at M270.

- `a_pending_permission_ask_is_drawn_once` — now also asserts the prompt carries the whole
  command. The scripted turn runs `echo epsilon-ready`, so the **second word** is the proof:
  the old rendering showed `echo` and could never show it. **Negative control: with
  `summarize_request` swapped back for `ask.pattern`, it fails with "the prompt must carry the
  whole command, not its first word."**
- `the_command_field_answers_the_question_and_the_whole_thing_is_the_answer` — the rule at its
  new home, including the `Remove-Item` case that started this.
- `an_unknown_shape_falls_back_to_the_request_itself`,
  `the_preference_order_is_stable` — the two behaviours that moving the rule could have broken.

## What nothing exercises

- **The desktop has no `text_safety`.** A `U+202E` inside a command still reaches the window's
  approval screen unstripped — the desktop half of A35, and the more serious of the two,
  because this is the screen a reader decides on. The terminal refuses to approve an ask
  carrying display-steering characters at all (M261); the window has no such check.
- **The window still shows only the capability and the request**, not the rule that matched.
  The terminal's dialog can say *why* an ask was allowed; this one cannot.
- **`fit` is an approximation in ems**, so a long command is still shortened by an estimate
  rather than measured. The marker (`?`) makes that visible, which is the rule this repository
  keeps: a partial value says it is partial.