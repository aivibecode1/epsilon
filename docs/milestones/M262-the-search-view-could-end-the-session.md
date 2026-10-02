# M262 — the search view could end the session

Found by a parallel read-only review of the terminal's key path, verified here by a test that
failed before the fix and passes after it.

## What happened

`handle_key` returns *true when the app should quit* (`app/keys.rs:419`), and
`handle_term_event` hands that straight to the run loop (`:1713-1718`). `handle_search_key`
returned `true` from **every arm** — including its catch-all.

So:

```
/search needle        ← search opens, panel draws
<any key>             ← the harness exits
```

`Esc` closed the search **and** quit in the same keystroke. A stray letter quit without
closing anything at all. The panel drew, the footer drew, and the session ended.

**Why no test caught it.** All four tests that cover search call `search_own_transcript`,
`search_step` or `search_close` directly and never press a key. A view whose entire key
surface is untested is how a return value written in the *other* vocabulary survives: the two
conventions are `true = done` and `true = quit`, and one `return` line separates them.

## The fix, and the half of it that mattered more

Every arm now returns `false`, and the catch-all says so:

```text
x is not bound in the search view — n/p step, esc or q close
```

That second part is not decoration. A key the search view does not bind is exactly the
"nothing happened" state `AGENTS.md` names as the defect this repository keeps rediscovering,
so it is not allowed to be silent.

**The part I nearly shipped wrong.** Withdrawing the search's accidental claim on `true` also
withdrew the only way to quit — the branch sits above everything and `Ctrl+Q` was falling into
its catch-all. `app.exit` now keeps its precedence over the branch, exactly as the approval
branch has it (`:446`), and `the_exit_chord_still_works_while_a_search_is_open` holds that
line. Quitting drops what the view was holding; refusing to quit is strictly worse than a
search that ignored a stray key.

## The comment that described a different program

`keys.rs` explains, correctly and at length, that the search branch is placed **after** the
approval branch:

> A pending permission ask is a question about *doing work*, and a search is a question about
> *reading*; answering the first with `n` should deny the tool, not silently move the viewport.

The two guards were in the **opposite** order for as long as that paragraph existed. With a
tool waiting for permission and a search open behind it, `n` stepped the viewport, the dialog
stayed up, and the tool stayed parked — on the one screen where the reader is deciding whether
work runs.

The code now matches the comment. `a_pending_ask_outranks_an_open_search` is what makes it
stay matched, and it is the second instance in this file of a rule
`the_backward_jump_chord_arms_backward_not_forward` already states: **a comment about ordering
is a promise, and only a test makes it one.**

## Verification

Battery `m262` — **GATE PASS: 1,872 tests across 76 binaries, 0 failed**
(`audit/evidence/gate-m262.log`). Up from 1,859 at M261; the thirteen new tests are this
milestone's four plus M263's, since one battery covered both trees. `epsilon-tui` is 502 unit
tests.

Tests added, all on the key path rather than on the helper:

- `no_key_inside_the_search_view_can_end_the_session` — sweeps ten keys, reopening the search
  per key so a loop cannot silently test the composer for the keys after the first.
- `the_exit_chord_still_works_while_a_search_is_open` — the regression the fix above would
  otherwise have shipped.
- `a_pending_ask_outranks_an_open_search` — holds the ordering comment to its promise.

**Negative control:** with `handle_search_key` returning `true` again on `Esc`, the first test
fails with `Esc ended the session from inside the search view`. That is the RED, taken before
the fix rather than argued for.

## What nothing exercises

- **The "not bound in the search view" notice is a transcript row, not panel text.**
  `App::notice` goes through a channel to `ui.push_notice` (`event.rs:291-296`), and the search
  panel is a **centred five-row overlay drawn last over the transcript** (`app.rs:1625-1630`).
  A new row lands at the bottom of the body, so the notice is visible at ordinary sizes — but
  that is geometry, not a guarantee, and at a short terminal the panel can sit over it. The
  right home is the panel's own key row, which is where a reader is already looking. Not done
  here because it is a rendering change and this milestone is a contract one.
- **The other seven modals still swallow unbound keys.** The "not bound here" notice lives in
  the last arm of the top-level `match`, and every modal returns earlier than it — approval,
  guidance, label prompt, timeline, palette, session browser, picker, auth picker, auth key.
  The invariant is currently true in the composer and in the search view, and nowhere else.
  (Recorded in `audit/03-defects.md` this round.)
- **`search.is_some()` is not in `overlay_open()`**, so the composer is still drawn focused
  behind an open search while the keystrokes go to the panel.
- **`app.interrupt` is in the binding table and printed by the footer, but Esc is handled as a
  bare `KeyCode::Esc` literal** — so rebinding it in `keybindings.json` changes the printed
  chord and nothing else. This is the closed `app.clear`/G1 defect shape.