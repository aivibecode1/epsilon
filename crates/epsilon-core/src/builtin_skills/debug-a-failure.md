# Debug a failure

Use this skill when something fails and the cause is not already proven. The
order matters more than the cleverness.

## 1. Reproduce the smallest failure

- Capture the exact command, the exact error, and the version or commit.
- Reduce the input until the failure is one assertion or one call.
- If it only fails in one environment, record what differs: OS, shell, terminal,
  locale, environment variables, installed tools.

A failure you cannot reproduce is not yet a bug report; write down what you know
and what you need.

## 2. Name the layer before changing code

Most wasted debugging is a fix in the wrong layer. Ask which of these can
produce the symptom:

- the front end (input, focus, rendering, a frame that never drew);
- dispatch and composition (the wrong handler, a config not read);
- the core loop (scheduling, state, cancellation);
- a tool or its sandbox (a command that ran somewhere else);
- the transport or provider (request shape, timeout, error mapping);
- the filesystem or OS (permissions, locks, path spelling, line endings).

Then make one observation that distinguishes the candidates. A log line, a
frame, a request captured at the boundary. "It is probably X" is a hypothesis,
not a finding.

## 3. Prove causation

- Form one hypothesis at a time. Change one variable.
- Confirm that the broken version fails and the fixed version passes, for the
  same reason. If both pass, the test proves nothing.
- Tell the difference between correlation and cause: a cache hit can make a fix
  look real. Clear the cache or rebuild under fresh metadata when in doubt.
- When the cause is a race, run the reproduction repeatedly and state how many
  runs you observed.

## 4. Fix and lock it down

- Fix the cause, not the symptom. A retry around a corrupt value hides the bug.
- Add the test that fails for the reason you named. Run it red, then green.
- Re-run the neighbouring tests and the real verification command.
- Record the incident in the status or ADR log when the behaviour was
  deliberate and moved.

## Report

State the layer, the evidence that identified it, the fix, and the test. If the
defect is still open, say exactly what is unknown. "Intermittent, three of
twenty runs, only under the pseudo-console" is a useful report. "Flaky" is not.
