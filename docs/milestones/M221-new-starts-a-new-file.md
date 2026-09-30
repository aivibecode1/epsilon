# M221 — `/new` starts a new session file, and the primitive it needed is deleted

**Plan 020 step 3, and the last step of A16.** `/new` used to clear the visible rows and keep
appending to the same file — so a second conversation was a stowaway inside the first one's file, and
the header's `id` named only the first. With one file per launch (M210) and a directory per working
copy (M217), that behaviour was the last place the old model survived.

## What changed

```text
/new   before:  clear the rows, keep appending to this session's file
       after:   name a file in this workspace's session directory and attach it
```

The implementation is small because the *mechanism* already existed: `switch_session_file` — the one
path shared by `/resume <path>`, `/import` and the browser's Enter — attaches a store and rehydrates
the transcript from its history. `/new` now does what a launch does (name a file where the others
are) and then takes that path. No new seam, no second switch implementation, and no way for the
browser and the command to disagree about what "attached" means.

Three details that are load-bearing rather than incidental:

- **The file is created before the switch, with a handle dropped immediately.** `SessionStore::open_in`
  writes the header and takes a cross-process **lease**; `switch_session_file` opens the same file
  again. Holding both would have this process wait on its own lease — the same class of mistake A19
  was, in the same file, two hundred lines of code away.
- **A run in flight refuses the swap**, with the reason on screen. Swapping the store under a running
  turn would leave that turn appending to a file the transcript no longer shows: the TUI invariant
  ("nothing happened must never be a state the UI can be in") applied to a swap rather than to a key.
- **The notice names the file it made**, relative to the workspace, and says the previous session is
  kept. The old notice said "transcript cleared (session file continues; use /fork for a branch
  file)" — advice that was correct then and is misleading now, since `/fork` is for branching and
  `/new` no longer continues anything.

## `reset_leaf` is gone, and that is the point of the slice

`SessionStore::reset_leaf` (pi `resetLeaf`) made **the next append start a new root inside the same
file**. That is the one-file-many-conversations primitive this model deleted, and it had **no
production caller** — the sixth instance of this repository's signature class, and the one the ledger
row for A16 was opened about. The plan's decision was "delete it unless step 2 finds a caller"; step 2
found none, and a measurement before deleting it confirmed none is pending: `jump_to_prompt` and
`jump_to_char` move the *viewport*, not the store's leaf, so no navigation path can want "continue
from before the first entry".

Deleted with it: the two test calls, and the test they were in — **rewritten, not deleted**. What that
test asserted was two things at once: the fact underneath (`/tree` navigation to the first message is
a *position*, not an id) and the reset itself. The first is still true and still worth a test, so it
kept its place under a name that says what it checks:

```text
navigating_to_the_first_message_is_a_position_not_an_id
```

The second is gone, and its coverage was **not** re-pointed at another primitive: a test that keeps
its name while checking something else is how a suite starts lying. Four doc comments in `store.rs`
referenced the deleted item — two of them as rustdoc links, which `missing_docs`/clippy would have
turned into a broken intra-doc link — and now say what replaced it.

## Verification

**Battery `m221` GATE PASS** — `audit/evidence/gate-m221.log`: **1,644 tests across 65 binaries**
against M220's 1,642: **+2**, the two new `/new` tests, with the reset test rewritten in place (one
out, two in). Gate shape: check-links 0 s, check-lint-tables 0 s, fmt 2 s, clippy 33 s, test 365 s,
demo 17 s, real-check 7 s. Run with `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0`
(M208's disk knob).

**The new tests**, in `crates/epsilon-tui/src/app/tests.rs`:

- `new_command_starts_a_new_session_file_and_keeps_the_old_one` — after `/new` the agent is attached
  to a *different* file, **inside this workspace's session directory**, that **exists already**; the
  previous conversation does not come along; the directory holds **two** files; and both are browsable.
- `new_command_refuses_while_a_run_is_in_flight` — the refusal says *why*, entered through
  `AgentEvent::AgentStart` because `running` is state the run loop owns and there is no setter to
  reach behind its back.

**On the controls, honestly.** Two were attempted and neither is the clean thing this repository asks
for. The first — skipping the swap — failed the test, but inside `epsilon-core` rather than at the
assertion, so it demonstrates *that* the test fails and not *why* it is the right test. The second —
pointing the swap back at the file the app already holds — **passed**, because
`agent.session_path()` is `None` at that point and the fallback silently restored the real path: a
control that did nothing, which is the failure mode this repository warns about most often.

So the claim rests on something simpler and checkable by arithmetic: **`/new` used to write no file at
all**, and the test asserts the directory holds two after the command. No rewrite of the old code can
satisfy that, which is why the count assertion is in the test rather than in a control script — and
why the second assertion (the file exists, not merely that the store moved) is there beside it.

## Limits

- **No frame test for `/new`.** The tests assert the store, the directory and the notices; nothing
  draws the new session's first screen. What a user sees is an empty transcript and one line of
  notice, and that line's wording is only checked by `contains("busy")` in its sibling.
- **The refused path is not the only unhandled one.** A `create_dir_all` failure and an `open_in`
  failure each push a notice and stop, which is right, and neither has a test — the second needs a
  read-only workspace to provoke.
- **`/new` does not ask.** pi's `/new` is immediate too, but a harness with one file per conversation
  accumulates files faster than one with one, and nothing prunes them (plan 020's open question, still
  open). A user who types `/new` forty times gets forty files and a browser with forty rows; that is
  the accepted price of the model, and it is not addressed here.
