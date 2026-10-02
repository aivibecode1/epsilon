# M261 — the permission dialog cannot be lied to, and the footer stopped agreeing with it

M260 closed the transcript half of the bidi defect and named the open half: **the approval
dialog**. It renders the *agent's* own `capability` and `detail`, and it is the one screen
where a reader decides whether work runs. This milestone closes it.

## What a request carrying a `U+202E` now gets

```
+ permission required -------------------------------------+
| cap  bash                                                |
| what ls -R /                                             |
| this request carries 1 display-steering character(s), re |
| it cannot be approved as shown                           |
| n  deny          e  deny + guidance                      |
+----------------------------------------------------------+
approval — cannot approve as shown · n deny · e deny + guidance
```

Three properties, each of which had to be decided:

**The text is sanitised, like the transcript.** Same helper, same characters. The command
shown is the command, minus the eleven bytes that would have reordered its display.

**The approve keys are withdrawn, not merely the text.** This is the part that differs from
M260's transcript handling, and it is deliberate. Stripping the override makes the dialog
honest about most of the text and still leaves the reader approving something they did not
fully see. On a transcript row that is tolerable — it is a report. On the screen that decides
whether a command runs, it is not. So `y` and `a` are refused, the ask stays parked, and
`n` still works, because the direction that cannot run the wrong thing is the one that must
always be available. **Refusing is not a wedge.**

**Both callers share one predicate.** `ask_display_is_honest` answers the question once;
the footer decides which keys to advertise and the key handler decides which to honour. Two
computations of one question is precisely how the next paragraph happened.

## The footer was advertising a key the window refused

This is the finding worth the milestone. The first version of the fix did the right thing
in the dialog and left the footer printing:

```
approval · y allow once · a always · n deny · e deny + guidance
```

while `y` did nothing. **A key the window does not act on, printed as though it would** —
the defect this repository tracks by name, and it arrived *from the direction of a security
fix*. The test caught it by asserting on the whole frame rather than on the dialog.

Two decisions follow:

- `Hints::approval_deny_only()` exists, and the mode word says **why** the decision surface
  is smaller.
- The predicate is shared rather than computed twice, because that duplication is the
  cause and not a coincidence.

## Two mistakes the tests caught, both mine

**I deleted the `y` arm.** Inserting the guard by replacing the `match` arm removed the
`KeyCode::Char('y' | 'Y')` arm entirely, so `y` did nothing for *every* ask. Two
pre-existing tests failed immediately —
`a_pending_approval_preempts_the_auth_key_prompt` with *"'y' must answer the approval"*.
Restored. This is what those tests are for, and the argument for keeping a behaviour-locking
test for something you are editing right next to.

**An assertion backwards.** `handle_key` returns true when the app should **quit**; I
asserted it returned true on a refusal. The neighbouring tests had it right in front of me.

## Verification

Battery `m261` — **GATE PASS on attempt 1: 1,859 tests across 76 binaries, 0 failed**
(`audit/evidence/gate-m261.log`). Up from 1,847. `epsilon-tui` is 490 unit tests.

Five new tests: the approve keys are refused and the ask stays parked; deny still works;
the dialog names what it removed and withdraws the approve keys **in both the dialog and the
footer**; and an ordinary ask is untouched — because a guard that fired on every ask would
be useless.

**Negative control:** with the footer forced back to `approval()` regardless, the
footer-honesty assertion fails.

**A gate caught my own tooling.** `WriteAllLines` writes `Environment.NewLine`, so
normalising three lines of a 4,800-line test file with it reintroduced CRLF across the whole
file and `check-line-endings` failed. Rewritten with LF. The gate is not a chore; it is the
third time this season it has caught something that reading would not have.

**Reinstall:** done this round. `cargo install --path crates/epsilon-cli --features real
--force` into `%TEMP%\epsilon-install`; the demo runs clean from the installed binary. It
goes to a temporary root rather than over the owner's global `PATH` entry, because replacing
a binary another toolchain may own is the owner's decision.

## What nothing exercises

- **Below 60×12 the fallback path still renders the ask unstripped.** `draw`'s
  too-small branch prints `capability · pattern` directly. It cannot be *approved* there —
  the keys are refused by the painted-id check — so it cannot cause the harm this milestone
  closes, but it is still text on screen that was not sanitised.
- **Composer input is deliberately untouched**: it is the reader's typing.
- **Picker, session list and search render names and paths that never pass through a
  transcript line**, so a directory whose name contains a bidi character is still displayed
  as something else.
- **Grapheme-safe truncation is still absent.** `.chars().take(n)` at four sites, and
  `frame_dump.rs` cuts rendered rows — the likely source of A23's CP437 trouble.
- **The reference's `FrameBudget` is still not adopted**, and still needs a redraw-cadence
  measurement before it can be.