# M241 — you can now declare a goal's gates

M240 shipped a goal's completion rule that only a hand-edit of a session file could reach. That
is not a feature; it is a type. This is the syntax.

```
/goal fix the auth tests, and don't call it done until the suite is green
/goal edit gate cargo test              # set the rule
/goal edit gate cargo clippy             # replace it — not append
/goal edit gate clear                   # take the rule off
```

## Replace, not append — and empty means clear

Both are the same field, and both would be bugs if they were anything else.

**Append** would mean a user who re-reads their own goal ends up with three tests and a
completion rule that silently got stricter every time they corrected a word. "Add a gate" is
then `read the current list, push, send it` — which means every caller has to read before it
can write, and cannot lose one by racing itself. So `GoalOp::Edit` carries the **whole** list.

**An empty list is a request, not an omission.** Omitting the field means "leave it alone";
sending `[]` means "take the rule off". They have to stay different, because otherwise "clear
the gates" is unreachable from any client that omits empty values — and **a rule a user cannot
remove is a rule the user is stuck with.**

## One decision that is smaller than it looks

`UpdateGoalArgs::gates` is a list of **command strings**, not a list of gate objects.

`GoalGate` has a second field, `timeout_seconds`, and the natural thing is to expose both — which
needs `schemars::JsonSchema` on the type, and `epsilon-contracts` has **no `schemars`
dependency and deliberately so**. Adding one for a single field would be a real change to a
crate that is meant to stay dependency-light, to carry a knob whose default (300 seconds) is
right essentially always. Commands go in; the timeout is the type's business.

## A test in the wrong place, and what it cost

The first attempt put "an empty gate is refused at declaration" beside the tool tests. It
**passed against a build where the rule did not exist** — because the tool tests drive a
`FakeHost`, and the refusal lives in `Agent::goal_apply`, which a fake host never calls.

That is the repository's own recurring lesson in a new costume: a test that exercises a stand-in
is a test of the stand-in. The declaration tests now live in
`crates/epsilon-core/tests/goal_state.rs`, where the real `goal_apply` runs, and the tool keeps
the two tests that are genuinely about the tool — that a **failing** gate refuses completion and
that a **passing** one lets it through. The pair matters: without the second, the first would
also pass if the tool refused everything.

The three declaration tests: an empty command is refused; a gate can be set, replaced and
cleared; and a gate list that survives each of those says what it should.

## Verification

**Battery `m241` — GATE PASS on attempt 1, 1,779 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m241.log`). Up from 1,774: five goal tests.

## What nothing exercises

- **The `/goal edit gate` surface end to end.** The TUI parses it and `goal_apply` honours it,
  but no test drives the parser with `gate clear` versus `gate cargo test`.
- **A gate under real confinement.** Still unexercised — every gate test runs
  `Confinement::Off`, because that is the only backend this harness sets up.
- **The round driver still ignores gates.** They gate completion, not continuation, and
  Hermes's second use — a failing gate's output as the continuation prompt — is still not built.