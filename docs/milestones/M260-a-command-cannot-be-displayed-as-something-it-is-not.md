# M260 — a command cannot be displayed as something it is not

The new reference's most transferable idea turned out to be one we **already had**, and its
second most useful one turned out to be a **hole in ours**. Both are recorded, because
"we checked" and "we were missing it" are the two halves of a useful reading.

## What we already had

The reference warns that a key arriving before a dialog's first paint must not answer it.
**We have that, and a stricter version.** `handle_approval_key` compares the *specific ask
id* against `approval_painted_id` and refuses otherwise — not merely "has this dialog been
drawn once", but "is the dialog on screen the same ask these keys may decide". Its comment
explains why, and it is right: a FIFO channel parks parallel tool calls, so a key handled
after a new ask is seated must not answer what no frame has shown.

We also have `app/golden.rs`, which drives the real `draw` through a `TestBackend` and
asserts on the **assembled screen**. That covers the reference's "golden frames" idea, with
a deliberate difference it teaches too: those tests assert on **structure, not pixel-exact
text**, because "a golden test that pins every character fails on every deliberate copy
change and gets rewritten without being read".

So the reference's top two recommendations were **already implemented**. Porting them would
have been duplication — and the note that would have prevented it already says to check
before adopting.

## What we were missing

A grep for `202E`, `2066`, `200E`, `bidi`, `display_safe`, `is_control` across the whole
workspace returned **nothing**.

ratatui 0.30 drops control characters, so an escape sequence in tool output cannot drive
the terminal. **Bidirectional overrides are not dropped.** `U+202E` (RIGHT-TO-LEFT
OVERRIDE) is eleven characters that reverse the *visual* order of everything after them.

This harness shows a reader the line it is about to run, and then asks whether to run it. So
`ls<U+202E> -R /tmp` can be **read** as one command and **be** another. That is not a
rendering artefact. It is the approval prompt lying, in a product whose entire premise is
that you read what you approve.

`crates/epsilon-tui/src/text_safety.rs` strips the embedding/override family
(`U+202A`–`U+202E`), the isolates (`U+2066`–`U+2069`), the directional marks
(`U+200E`/`U+200F`/`U+061C`) and every other non-layout control character, and reports how
many of each it removed.

**It is applied in one place.** `self.lines.push` appears exactly once in `state.rs`, inside
`push_line` — which its own comment already called "a choke point for future hooks". So a
line cannot enter the transcript any other way, **including by replaying a session from
disk**, which is the path where hostile text is most likely to arrive. This was the first of
those hooks, and it took the promise the comment had already made.

### Three decisions inside it

**The common path borrows.** A transcript of two thousand rows must not allocate two
thousand `String`s per frame to learn that nothing needed removing, so `display_safe`
returns `Cow::Borrowed` when there is nothing to do — and a test asserts the *borrow*,
rather than trusting it.

**It does not change what the tool receives.** This is display-only. The session log keeps
the exact bytes; no command, argument or file is altered. Stripping the transcript does not
change what runs.

**Removal is recorded, not silent.** `TranscriptLine.hidden` carries the counts. A silent
strip would be this repository's own recurring failure wearing a security hat — and the
count is the useful part: a reader told *"2 hidden characters"* can conclude the text was
not what it claimed, which is the entire point of showing commands before approving them.

## Verification

Battery `m260` — **GATE PASS on attempt 1: 1,855 tests across 76 binaries, 0 failed**
(`audit/evidence/gate-m260.log`). Up from 1,847. `epsilon-tui` is 486 unit tests.

**Negative control:** with the stripping skipped in `push_line`, the two override tests fail
and the third — `ordinary_text_is_stored_exactly_and_the_common_path_borrows` — still
passes. That is correct rather than a weak control: that test guards the **common** path,
which stripping deliberately leaves untouched, and it asserts the borrow directly against
`display_safe`.

**Reinstall:** yes, and not done. This changes what `epsilon-tui` **displays**, which is the
half of the product the owner reads. The release install is a separate ~28-minute build and
is left to a round with time for it; it is recorded here rather than skipped silently.

## What nothing exercises

- **The approval dialog is not covered.** `handle_approval_key` is guarded, but the text it
  *renders* comes from `pending_ask`, not from the transcript, so it does not pass through
  `push_line` and is **not stripped**. A permission dialog is the single place where
  showing a command wrongly is worst, so this is the open half of the milestone and the
  first thing to do next.
- **Composer input is not stripped** — it is the reader's own typing and must not be
  modified; nothing else reaches it.
- **Other text paths**: the picker, the session list and the search box render names and
  paths that reach the screen without going through a transcript line. Files and directories
  can carry a bidi character in their names.
- **Grapheme-safe truncation is still absent.** `.chars().take(n)` at four sites can split a
  combining sequence, and `frame_dump.rs` cuts rendered rows — which is where A23's CP437
  trouble would come from. Measured as present, not fixed.
- **The reference's `FrameBudget` is not adopted.** It is the most interesting idea in that
  repository — `next_frame_in() -> Option<Duration>` where `None` sleeps until an event — and
  it is exactly the knob that separates *slow* from *lost input* on our unreproduced freeze.
  Adopting it needs a redraw-cadence measurement first, and there is none.