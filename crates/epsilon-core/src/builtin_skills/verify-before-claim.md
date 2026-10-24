# Verify before you claim

Use this skill when you are about to tell someone what the code does, what a
command returned, or what they will see. It turns "I believe" into "I checked".

## The rule

Every factual sentence about this repository must have one of these behind it:

- a file and line you opened (`path/to/file.rs:42`);
- a command you ran and its actual output;
- a frame you rendered and read;
- a URL you fetched, quoted as a URL.

If none of those is true, label the sentence **inference** or **unknown** and
say what would settle it. An honest unknown is cheap. A confident wrong answer
is the most expensive thing a coding agent produces, because the next person
acts on it.

## Procedure

1. **Classify the claim.** Is it about code shape, runtime behaviour, user-visible
   output, or an external service? Each has a different primary source.
2. **Go to the primary source.** Code shape: read the definition *and* every
   production caller. Runtime behaviour: run the narrowest command that exercises
   it. UI: render the real frame and read it. External: fetch the page or call the
   endpoint.
3. **Run a control when the result could pass for the wrong reason.** Ask what the
   broken version would print. If the answer is the same, the check proves
   nothing.
4. **Quote the evidence.** The output, the line, or the frame. Not a paraphrase.
5. **State the limit.** "Passed on Windows, not tested on macOS" is part of the
   claim, not a footnote.

## Traps that have produced false claims

- A test passes, so the feature works. Tests also pass for unreachable code,
  unused primitives, and assertions on state that is never drawn.
- A grep finds a definition, so it is called. Grep for callers, then check that at
  least one caller is production code.
- A count was measured once. Numbers in docs are claims; re-measure before
  repeating them.
- The code looks right. The frame may still be wrong: wrap, truncation, a panel
  drawn over the one you meant.
- A command exited 0. Check that it did the thing, not just that it did not error.

## Output shape

> **Verdict:** one sentence.
> **Evidence:** the command/file/frame.
> **Limit:** what was not checked.

Keep it short. The evidence is the point; the prose is packaging.
