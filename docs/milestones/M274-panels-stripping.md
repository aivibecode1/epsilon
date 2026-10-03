# M274 — three panels drew unstripped text, and the frame cannot see why

The last of **A35**'s open surfaces: the `@`-popup, the session browser, and the model
picker's stage 2.

## What was drawn as written

* the **`@` popup** — workspace paths;
* the **session browser** — a conversation's **title**, which is its first user message, read
  from a file on disk, so a session imported from elsewhere can contain anything;
* the **model picker, stage 2** — ids from a **live listing**, i.e. text that came from a
  third-party endpoint. That is the only place in this crate where the bytes are not ours.

## Why strip rather than refuse

M261's distinction decides it. The permission dialog **refuses** an ask carrying a
display-steering character, because a reader there is *making a decision* and a cleaned line
still means agreeing to something they did not fully see.

These three only **display**. What they hand back comes from the row's own field — the picker
applies the model's id, the browser resumes the path beside the title, the popup inserts the
path it listed. **None decides anything from the string it draws**, so removing the character is
the whole correct response; refusing would break a legitimate row over bytes that were never
going to be executed.

## The finding, and it is about the instrument rather than the code

The frame test for this was written first, and **its negative control passed**.

`safe_to_show` was stubbed to `text.to_string()` — a complete no-op — and
`no_panel_draws_a_display_steering_character` **still went green**. That is not a weak control;
it is an instrument that cannot see the thing under test. `row_text` reads ratatui's **cell
symbols**, and `U+202E` never becomes one:

```
a_frame_cannot_see_a_display_steering_character
```

draws a hostile string straight through ratatui with nothing of ours in between and finds the
character is **not in the buffer**. It is written to *document that limit*, and to say what
would invalidate it: if it ever starts failing, ratatui began passing the character through and
every frame-level assertion about it becomes meaningful again.

Two consequences:

* **The strip is still right** — defence in depth, and a different backend, width or a future
  ratatui need not behave the same. But the reason to keep it is not *"the bytes reach the
  screen"*; it is **"we do not depend on that being true."**
* **`text_safety`'s own doc overstates its premise.** It says ratatui "passes it through". On
  this version it evidently does not, at least not into the buffer. That is worth correcting,
  because the module has been arguing from that premise since M260.

So the load-bearing assertions are on **text**: `an_override_never_survives_into_a_row` checks
the helper directly, and it is the one that fails when the strip is removed —
`left: "claude\u{202e}-3-opus"` against `right: "claude-3-opus"`.

## The count is not reported here, and that is a limitation

M260's rule is that the transcript must say what it removed. **An overlay has nowhere to put
it** — no field, no footer, no line — and inventing one for a value that cannot cause harm is
the noise this repository keeps arguing against. The honest statement is that this is a gap in
the rule's reach, not a decision to skip it.

## Verification

Battery `m274` — **GATE PASS: 1,909 tests across 77 binaries, 0 failed**, all seven gates,
**attempt 1** (`audit/evidence/gate-m274.log`). Up from 1,906 at M273.

- `an_override_never_survives_into_a_row` — the helper, on text. **Negative control verified**:
  with `safe_to_show` stubbed to a no-op it fails on
  `left: "claude\u{202e}-3-opus"` vs `right: "claude-3-opus"`.
- `a_panel_row_is_stripped_before_it_drawn` — the frame, asserting **the row survives first**
  and the character is absent second, in that order, because a row that vanished would satisfy
  a "no override" assertion for entirely the wrong reason. That mistake is what the first
  version made.
- `a_frame_cannot_see_a_display_steering_character` — documents the instrument's limit.

## What nothing exercises

- **The session-browser row was not separately driven.** It uses the same `safe_to_show` call
  as the other two and is covered by the helper's test, but no frame test opens that panel —
  building the fixture was not worth the round against a shared helper already proven.
- **Stripping is presentation.** The session log keeps the exact bytes, so a hostile path is
  still the path that is opened — which is the permission dialog's job, not this one's.