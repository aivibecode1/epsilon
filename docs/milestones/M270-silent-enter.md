# M270 — Enter did nothing, and two predicted defects were not real

Three claims from the desktop review went into this round. **One was true, two were false**,
and the false ones are the part worth keeping.

## The real one: pressing Enter mid-turn was silent

The review reported a *sticky* refusal — `self.refusal` is set and never cleared, so the status
bar claims a run is in flight for the rest of the session.

**The field was never reachable.** `send_draft` checked `session.running()` and returned
`None`; `IdeSession::submit` checks the **same condition** one line later and returns exactly
the right sentence, which the window stores in `self.refusal` for the status bar. The guard ran
first, so the branch that produces the message was never reached — **two checks of one question,
and the second one was dead code.**

So no stale message was ever drawn. The actual defect is the opposite complaint, and it is
worse:

> **Pressing Enter while a turn was running did nothing and said nothing.**

The project's standing invariant is that *"nothing happened" must never be a state the UI can be
in*. This was it, in the window whose whole purpose is a conversation.

The guard now answers the question it was already asking:

```
a run is already in flight — wait for it, or press Esc to stop it
```

in the status bar **and** as a transcript row. And `pump` clears the refusal once the window is
idle, because a refusal describes a condition and conditions end — so the field is now both
reachable and self-expiring.

**Negative control:** with the message removed, the test fails with *"Enter did nothing and said
nothing, which is the state this repository forbids."*

## The two false ones

**"The permission ask is printed twice."** Driving a *real* run until it parks on a gate, the
agent region is:

```
"Agent", "provider: faux (offline scripted)", "› say hello",
"allow bash echo ?   [enter] allow   [esc] refuse"
```

**One** mention. `PermissionRequested` folds into a transcript row in the reducer, but it does
not reach the window's transcript on this path, so there is no second copy to draw.

The claim's own evidence said so as well — *"`tests/window.rs:925` asserts
`contains("allow bash")`, which the **prompt** satisfies, so the test cannot see the
duplicate"* — which is equally true of a duplicate that does not exist. The test that seemed to
contradict it was not looking where the defect would have been.

**The first version of my test passed for the wrong reason**, and the reason is the lesson:
it seeded a `PermissionRequested` straight into the transcript, so the approvals **channel** was
empty and there was nothing to duplicate. **A test aimed at a duplication must build both
copies**, or it measures its own fixture. It now drives a real run to a real gate.

## Verification

Battery `m270` — **GATE PASS: 1,900 tests across 77 binaries, 0 failed**, all seven gates,
**attempt 1** (`audit/evidence/gate-m270.log`). Up from 1,898 at M269.

- `pressing_enter_while_a_turn_runs_explains_itself` — presses Enter through `send_draft` while
  a real turn is in flight and asserts the frame explains itself. It deliberately goes through
  the **reader's** path: the existing
  `a_second_submit_while_running_is_refused_rather_than_queued_silently` calls `session.submit`
  directly and writes `if let Err(…) = …`, so it passes whether or not the refusal ever
  happens — it could not have found this.
- `a_pending_permission_ask_is_drawn_once` — a real run, a real gate, and an assertion on the
  drawn region.

## What that same frame did confirm

The ask reads **`bash echo`** — not the command. `request.pattern` is
`cmd.split_whitespace().next()` (`epsilon-core/src/exec.rs`), and the window renders it
verbatim, so a reader asked to approve `Remove-Item -Recurse C:\Users\me` is shown
`Remove-Item`, with no ellipsis and nothing marking it as partial.

**The string on the permission screen is not the string that will run**, and the terminal
avoids exactly this by rendering `detail["command"]` through `summarize_detail`. That is now
row **A48**, and the fixture is already a regression guard on the current behaviour — so
changing it is a deliberate act rather than an accident.

## Verification

Battery `m270` — **GATE PASS: 1,900 tests across 77 binaries, 0 failed**, all seven gates,
**attempt 1** (`audit/evidence/gate-m270.log`). Up from 1,898 at M269.

- `pressing_enter_while_a_turn_runs_explains_itself` — presses Enter through `send_draft` while
  a real turn is in flight and asserts the frame explains itself. It deliberately goes through
  the **reader's** path: the existing
  `a_second_submit_while_running_is_refused_rather_than_queued_silently` calls `session.submit`
  directly and writes `if let Err(…) = …`, so it passes whether or not the refusal ever
  happens — it could not have found this.
- `a_pending_permission_ask_is_drawn_once` — a real run, a real gate, and an assertion on the
  drawn region.

## What nothing exercises

- **A48 is not fixed here.** The window still shows the first word of a command on its approval
  prompt, and that is a security-shaped defect, not a cosmetic one. It is the next slice.
- **The status bar's refusal is now reachable**, and its lifetime is bounded by `pump`; no test
  asserts that it *clears*, only that it is set.
- **The other seven modals in the terminal still swallow unbound keys** — unchanged this round.