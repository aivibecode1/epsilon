# M171 — turning "sixteen uncovered commands" into a test that runs forever

## What this does

M167 measured that **18 of the 30 user-typable slash commands were never executed by any
test**, and M168 found a real defect hiding in that gap: nine of them called an accessor
that panics on a state the code above it calls normal. M170 gave the probe a `--submit`
flag so they could be checked in a real console — but that is one serialised run per
command, roughly 40 minutes of wall clock per batch, and the evidence does not survive the
next change.

A test persists. This drives **every** command once, on a fresh `App`, and asserts that
none of them panics and none of them does nothing at all.

## The assertion, and how it got sharper twice

The first version failed to compile (`no field session_picker` — the field is
`session_browser`), and the second failed on `/model`:

```
`/model` produced no notice and opened no picker - it may have done nothing at all,
which is how the M168 defect hid: after.len()=0 before.len()=0
```

That was **my assertion being wrong, not a defect**: `/model` opens the model picker
rather than printing anything. The fix is to count every overlay `App` can open, which I
took from the struct rather than from memory:

```rust
let opened_something = app.timeline_open
    || app.session_browser.is_some()
    || app.picker_open
    || app.auth_pick_open
    || app.auth_prompt_open
    || app.palette_open
    || app.trust_review_pending;
```

`trust_review_pending` is not an `*open` flag and was not in the first list I guessed
from a regex. Both failures are the same mistake in different clothes: I described a
condition from memory instead of reading the field list, and the test told me so. That is
what a test that runs every command is for.

## `/exit` is excluded on purpose

It is the one command whose job is to end the session. Running it here would prove nothing
about the other 29 and would leave the loop with nothing to loop over.

## The control

Restoring the panicking accessor on **one** command — `/timeline`, via
`open_timeline_picker` — reproduces the production failure exactly:

```
test result: FAILED. 0 passed; 1 failed
panicked at crates\epsilon-core\src\lib.rs:1002:22:
no session store attached
```

Not a near-miss and not a different message: the same panic `Agent` documents as a
programming error, reached through the real command path, by a test that types the command
the way a user does. Restored, byte-identical, and re-verified.

## Verification

- `epsilon-tui --lib`: **372 passed** (371 + the new sweep), 0 failed.
- Control: 371 filtered, 1 failed, panic at `lib.rs:1002`.
- Workspace battery `s285`: FMT=0, CLIPPY=0, TEST=0, DEMO=0; **62 binaries, 1,296 passed,
  0 failed** - exactly +1 for this test.

## Limits

- **The sweep asserts a weak property on purpose**: "did not panic, and did something
  visible". It is not per-command coverage — `/model` and `/model` produce different
  output, and only the weaker claim is checked for all 29. Stronger assertions belong
  beside each command, where the expectation is knowable.
- **The command list is a hand-maintained copy** of the `picker` mapping. A new command
  added without updating it is a silent gap. Making the test enumerate the parser's own
  table would remove the copy, but `Command` is not `Debug` and comparing needs a
  different design; recorded as open work rather than pretended away.
- **29 of 30, in a fresh `App` with the faux provider.** Nothing here proves a command
  behaves correctly against a real session file, a real lease, or a real provider.
