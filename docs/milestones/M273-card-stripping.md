# M273 — the guard test that could not see the thing it was guarding

The rest of **A35**'s terminal half: **tool cards bypassed display stripping entirely**.

## The hole

`push_line` is the single point where a line enters the transcript, and its doc says so:

> `self.lines.push` appears exactly once in this file, so a line cannot enter the transcript
> any other way.

It sanitised `text`, `full` and `md`. It **never looked at `line.card`.** So:

* `CardBody::Prompt` — the command the model wrote;
* `CardBody::Text` — **the tool's own output**, which for `read` is the file's contents;
* `CardBody::Diff` — a diff, also file contents;
* `args` and `head` — the model's arguments and the headline derived from them;

all reached the screen with a `U+202E` intact, **while the permission dialog two panes away
refused to approve a request carrying one.** That is the M272 defect's mirror image: a rule in
one place and its absence in the other.

## The test that could not see it

Two tests sit directly above this one and both pass:

```rust
app.ui.push_external_text("ls\u{202E} -R /tmp");
assert!(!line.text.contains('\u{202E}'));
```

**They push no card.** `push_external_text` has no `card` field to sanitise, so the assertion is
about a field the function never touched, and it has been green through every version of this
hole.

That is worth stating plainly rather than quietly replacing:

> **A test that cannot fail on the defect it names is not evidence.** The existing tests were
> not lying — they were answering a different question, and the gap was the absence of a card.

`a_card_is_stripped_where_its_text_enters` drives the **real** route — an assistant message
asking for the tool, then a streamed `ToolExecutionUpdate` — and checks both the prompt and
the body. Its first version used `ToolExecutionStart` and found no card at all, because a card
exists only once a model has asked for a tool: the same mistake in a new place.

**Negative control:** with the call-site strip replaced by a default, it fails with

```
the model's own command reached the card verbatim: "ls\u{202e} -R /tmp"
```

## One method, because there are three sites

Stripping at some of the three places a card body is set leaves the others reaching the screen.
`Card::sanitize()` covers **every** text-bearing field — all three `CardBody` variants, `args`
and `head` — and returns the count, which the call sites fold into `TranscriptLine::hidden`.

Three sites call it: the pending call, the settled result, and the streaming tail. The last is
the one a two-site fix would miss, and it is the *running* card — so without it the tool's
output was stripped once it finished and shown verbatim the whole time it was running.

**The count is not optional.** M260's rule is that a silent strip is the same failure wearing a
security hat, so the reader is told; the test asserts `line.hidden.total() > 0` rather than only
that the character is gone.

## Verification

Battery `m273` — **GATE PASS: 1,906 tests across 77 binaries, 0 failed**, all seven gates,
**attempt 1** (`audit/evidence/gate-m273.log`). Up from 1,905 at M272.

`a_card_is_stripped_where_its_text_enters` drives the real route — an assistant message asking
for the tool, then a streamed update — and checks the prompt, the body, **and** that
`line.hidden.total() > 0`. **Negative control:** with the call-site strip replaced by a default
it fails with `the model's own command reached the card verbatim: "ls\u{202e} -R /tmp"`.

## What nothing exercises

- **The `@`-popup, model-picker and session-browser rows are still unsanitised** — the rest of
  A35's terminal half, untouched here. A directory name containing an override reaches those
  three screens as written.
- **The desktop strips nothing but the permission line** (M272 closed only that).
- **Stripping is presentation.** The session log keeps the exact bytes — `text_safety`'s own
  doc is explicit that it does not delete what the tool received — so a hostile command is still
  what runs, which is the permission dialog's job rather than this one's.