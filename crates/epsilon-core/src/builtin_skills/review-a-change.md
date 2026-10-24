# Review a change

Use this skill when someone else's change — a pull request, a delegated
implementation, a patch you did not write — has to be judged. Review it in two
stages, in this order. Do not mix them.

## Stage 1: does it do what was asked?

Read the request first, then the diff. Do not read the author's summary first;
it is a claim, and it will anchor you.

- The requested behaviour: present, absent, or present with a different meaning?
- What did the request explicitly exclude? Is any of it in the diff?
- Are there user-visible consequences the author did not mention — a changed
  default, a new file on disk, a permission that widened?
- Is anything advertised but not reachable? A command with no handler, a config
  key nothing reads, a test as the only caller.

If the change fails this stage, stop. Implementation quality is irrelevant when
the change solves a different problem. Say so plainly and name the gap.

## Stage 2: is it built correctly?

Only now read the implementation closely.

- **Correctness.** Off-by-one, empty input, unicode, Windows paths, concurrent
  writes, cancellation, retry after partial failure.
- **Refusals.** Does an error path leave the user's work intact? Does a dropped
  responder fail closed?
- **Bounds.** Payloads, output, recursion, time. An unbounded loop that "will not
  happen" is a denial of service.
- **Security.** Untrusted text stripped of control and bidi characters? Paths
  normalised and checked for escape? Secrets kept off command lines and out of
  logs?
- **Tests.** Do they fail for the reason they name? Is the failure path tested,
  not only the happy path?
- **Waste.** Dead code, a second copy of an existing helper, a dependency that
  the standard library already replaces.

Run the tests yourself. A summary is not evidence.

## Verdict

Use three labels, each with a `file:line`:

- **Blocker** — must change before merge.
- **Follow-up** — real, but safe to land after.
- **Nit** — style or wording, explicitly optional.

Do not rewrite the author's change while reviewing it unless you were asked to.
The deliverable is a judgement with evidence, not a competing patch.
