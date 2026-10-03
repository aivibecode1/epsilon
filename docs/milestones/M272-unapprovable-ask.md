# M272 — the window could be asked to approve a command it could not show

The desktop half of **A35**, and the last gap M271 left open.

## The defect

`U+202E` (RIGHT-TO-LEFT OVERRIDE) reverses the visual order of everything after it, so a
command can be *displayed* as one thing and *be* another. The terminal has refused `y`/`a` on
such an ask since M261, leaving it parked with `n` still working.

The desktop window had **no check at all**. The same bytes were safe in one frontend and not
the other — which is the whole failure, and it is the third time this project has shipped one
frontend's rule to the exclusion of the other's (`/goal`, the permission line, and this).

## What moved, and what deliberately did not

`text_safety` is a pure function over text with no terminal and no app state in it, so it moved
from `epsilon-tui` to **`epsilon-runtime`**, beside `summarize_request` and `goal_command`. The
terminal's `text_safety` module is now a **re-export**, so its ten call sites did not churn and
there is still exactly one implementation.

`ask_is_display_honest` — *whether an ask can be shown and therefore approved* — is new and
shared. The terminal's `ask_display_is_honest` now delegates to it, which is what keeps the two
dialogs from drifting: the footer decides what to **advertise**, the handler decides what to
**honour**, and two computations of one question is exactly how this repository's own M261
arrived at a footer promising `y` while the handler refused it.

**The window now says what the terminal does, in the same words:**

```
allow bash ls -R /?   this request carries 1 display-steering character(s),
it cannot be approved as shown   [esc] refuse
```

and **does not draw `Allow` at all.** Stripping is not enough on this screen: a cleaned line
still means agreeing to something a reader did not fully see. And refusing is not a wedge —
`Refuse` is always drawn, because the direction that cannot run the wrong command has to stay
available.

## A test that could not be written the obvious way

The first two attempts drove a **painted frame**, and both failed the same way: the prompt is
drawn from `pending_approvals()`, so a seeded transcript row has nothing to draw against —
the trap M270 hit as well. Then the real obstacle: **`ChannelApprovalUi` has no way to park an
ask.** Only the agent can produce one, and the agent runs whatever the model wrote, which the
offline provider does not put a `U+202E` in.

So the rule is a pure function, `IdeApp::ask_line(request, budget) -> (String, bool)`, and is
tested where it is written. **What is lost is the drawn assertion; what is kept is the part that
decides whether `Allow` exists** — and the milestone says so rather than claiming a frame
proof it does not have.

## The guard that caught the move, correctly

`protocol_doc_tests::no_source_file_lost_a_test_since_head` failed on the move — the tests went
from `epsilon-tui/src/text_safety.rs` to a **new, untracked** file, and the guard diffs
`git diff HEAD -- crates/`, which cannot see an untracked file. The TUI's removed tests read as
lost.

Staging the new file made them visible and the guard passed. **The gate was right and the
workaround was to let it see the file, not to relax it** — worth recording because the tempting
move here is to add the lost names to an exception list, which is how a guard stops guarding.

## Verification

Battery `m272final` — **GATE PASS: 1,905 tests across 77 binaries, 0 failed**, all seven
gates, **attempt 1** (`audit/evidence/gate-m272.log`). Up from 1,903 at M271.

One attempt before it failed on `check-line-endings` and nothing else — a PowerShell
`WriteAllLines` wrote CRLF into `audit/03-defects.md`. **That is the third time this trap has
cost a round on this machine**, which is why it now has its own measured note in `05-machine.md`.

- `a_command_with_a_bidi_override_cannot_be_approved_in_the_window` — the override never
  reaches the text, `honest` is false, the line says it cannot be approved, and `[esc] refuse`
  is still offered.
- `an_ordinary_command_is_still_approvable_and_shows_whole` — **a guard that fired on every ask
  would be worse than none**, since the button would simply always be missing.

## What nothing exercises

- **Still no frame-level proof that the `Allow` button disappears.** The unit test pins the
  decision; the drawing of it is one `if honest && ui.button(…)`, which no test reaches
  because the ask cannot be produced. Recorded rather than claimed.
- **`text_safety` covers the permission screen only.** Tool output, file contents and model
  prose in this window still render unstripped — the rest of A35's desktop half.
- **The `@`-popup, model-picker and session-browser rows in the terminal are still
  unsanitised** (A35's other half), and this milestone did not touch the terminal's own
  non-permission surfaces.