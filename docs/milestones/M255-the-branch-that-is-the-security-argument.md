# M255 — the branch that is the security argument, finally executed

`agents/memory/goal-gates.md` has carried this line since the gates landed:

> **No gate has ever run confined in a test.** Every gate test runs with `Confinement::Off`,
> because that is the only backend the harness sets up. The production branch is unexercised.

Every gate test constructed `GoalGateRunner::new(None)` — the *unconfined* path. So the two
decisions that make a gate a gate rather than a shell command had **never executed in a
test**: run it through the backend when one exists, and **refuse** when one does not. The
second is the fail-closed rule a confused deputy would rely on.

## What runs now

Four tests, against `epsilon_safety::best_available()` — the same call the product's
private `resolve_sandbox` makes, so a test that skips is skipping for the same reason a
session would refuse:

- **a gate runs through the backend** — reached the sandbox, ran there, reported the result.
- **a failing gate reports identically there** — the failure path is what a model acts on, so
  it must not depend on which backend ran it.
- **an unavailable backend refuses rather than running unconfined**, and says why.
- **a backend declares what it cannot confine** — see below, because this one is the point.

The first two skip with a printed message where no backend exists. On this machine they do
**not** skip: `best_available` is the real Windows Job Object backend, and running with
`--nocapture` prints no skip line. A test that quietly skips proves nothing, so that was
checked rather than assumed.

## The honest part, which is the part worth keeping

The first test's name, in the version I wrote it, said a gate "runs confined". That
**overclaims**, and the fix is the finding:

> A job object bounds memory, process count and CPU, and it kills the whole tree on timeout
> or on drop — but nothing in it restricts what those processes may read or write, so a caller
> that needs a filesystem boundary must reject this backend rather than trust the word
> "confined."
> — `crates/epsilon-safety/src/sandbox/jobobject.rs:233-241`

`JobObjectSandbox::capabilities()` returns
`Enforcement::from_missing(vec!["filesystem", "network"])`, and **`epsilon-cli`'s doctor
prints exactly that**: `windows-job-object (available) — cannot confine: filesystem,
network`. So the honesty chain exists and is accurate.

It is now **also a test**. `a_backend_declares_what_it_cannot_confine` asserts, on Windows,
that the backend does **not** report full enforcement and that `filesystem` is named among
what it cannot confine. That is the assertion that matters most here: a future change making
the job object claim full enforcement would pass every gate test in the file and leave the
doctor telling users their writes are contained when they are not.

The gate test was also renamed — `a_gate_runs_through_the_backend_when_the_platform_has_one`
— because a name that overclaims is a small lie that survives review.

## Verification

Battery `m255b` — **GATE PASS on attempt 1: 1,830 tests across 75 binaries, 0 failed** (`audit/evidence/gate-m255.log`). Up from 1,826.

Attempt 1 failed on `fmt --check` because this milestone's own edits landed after the last
format; fixed and re-run. A gate script that refuses a battery because a doc comment moved a
space is a gate script doing its job, and this cost two minutes rather than a hundred.

**Negative control:** with the fail-closed branch disarmed,
`an_unavailable_backend_refuses_rather_than_running_unconfined` fails and the other eleven
pass. The branch that decides *whether to run unconfined* is now pinned by the test named
for it.

**Reinstall:** none. `epsilon-cli` and `epsilon-tui` are untouched; this is tests and one
memory note.

## What nothing exercises

- **A gate that actually escapes its workspace, on Windows, is not testable** because nothing
  stops it — that is the finding above, not an oversight in the test. On Linux the bubblewrap
  backend does give a filesystem boundary, and this suite's non-Windows branch asserts the
  weaker "either full, or it names what it does not".
- **The confined branch has never run on Linux CI from this file.** It would, but the
  backend there is bubblewrap and the assertions here are backend-agnostic — which is correct
  and also means they do not check that bubblewrap confines anything.
- **`workspace_fingerprint` is still never acted on.** With the gate now on the round path
  (M254), an unchanged workspace re-running a slow gate every round is a cost that has
  started to matter.
- **Still no end-to-end gated-goal test** through `UpdateGoalTool`.