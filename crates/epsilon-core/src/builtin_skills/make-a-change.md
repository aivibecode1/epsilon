# Make a change

Use this skill for any change that will be kept: a fix, a feature, a refactor.
The shape is the same at every size, and it is deliberately boring.

## Before editing

1. **State the change in one sentence.** If you cannot, it is two changes.
2. **Read the whole file you are about to edit**, not the matched fragment. The
   surrounding twenty lines are where the invariants live.
3. **Find the callers.** A function with one test caller is not a feature. Know
   which production path your edit reaches.
4. **Decide the layer.** If two frontends need the decision, it belongs in the
   shared core. A second copy in one frontend is the defect you will fix next.
5. **Check the test that should fail.** For a bug, reproduce it first. For new
   behaviour, write the failing assertion first.

## The edit

- Make the **smallest correct change**. Deletion beats addition. Reuse a helper
  that already exists three files over rather than writing a fourth.
- Do not reformat, rename, or move unrelated code in the same change. A diff
  that mixes a move with a behaviour change cannot be reviewed.
- Keep refusal paths intact: return an error, leave the file untouched, and say
  why. Never turn a refusal into a silent success.
- Comments explain **why**, especially when the code looks odd. Cite the
  reference implementation or the incident when the semantics come from one.

## Prove it

1. Run the narrowest test that covers the change. Watch it fail for the reason
   you named before you make it pass.
2. Run the neighbouring tests, then the project's real verification command.
3. For anything user-visible, render the real surface and read it. State
   assertions are not frame assertions.
4. Look at the diff. Delete debug prints, dead branches, and any file you
   created for a probe.

## Report

- what changed, in behaviour terms;
- which test proves it, with the command;
- what you did **not** verify;
- any deliberate shortcut, named as one.

If the requirements are ambiguous, ask before editing. If the change could lose
user data, confirm before writing. Those are the two places where speed is a
false economy.
