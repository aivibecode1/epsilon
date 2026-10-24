# UX Triage

Play the hardest plausible user of this product, let them try to get something done, then
filter what they say through a pragmatism layer before anyone acts on it.

A persona finds **friction**, not bugs. A technically correct surface can still be unusable.
The filter is what makes this useful instead of merely entertaining.

## Procedure

1. **Name one persona.** Answer five things, in this order: who is the hardest user of this
   surface; how much technology they have absorbed; the **one** thing they came to do; what
   would make them give up; how they sound when frustrated. Specific enough to stay in
   character for the whole session — a competence level, a one-sentence goal and a voice,
   not "a user who does not like the app".

   For this repository the natural personas are a first-day terminal operator who has never
   driven an agent, a keyboard-only operator with no mouse, a screen-reader operator on the
   desktop, and someone on a narrow window with long paths and a long transcript.

2. **Attempt their one task in the running product.** Not a tour of the features. Their
   task, start to finish, and nothing else: start a session, ask one thing, read the answer,
   stop the run. Stay in character.

3. **Read the screen, do not infer it.** Render the surface and actually look at it, at the
   sizes it is used at — 80x24 and 100x30 for the terminal. A frame nobody looked at has
   not been asserted, however confident the state looks.

4. **Count the steps.** Count the keystrokes and the screens it took to finish the one task.
   This number is the measurement; everything else is opinion.

5. **Write the rant in character.** Blunt, dismissive, specific. This is input to the filter,
   not the deliverable. Keep it short.

6. **Step out of the persona and filter.** Mandatory. Never hand raw persona complaints to
   anyone as work.

7. **Turn every RED into evidence before anyone fixes it.** A claim of user-visible breakage
   is asserted on the frame and checked against production code, not on internal state.

## The pragmatism filter

| Bucket | What it is | Test |
|---|---|---|
| **RED — real defect** | Any user hits this, not just this persona | Would a competent, busy user hit it too? Is it an accessibility failure — contrast, focus, target size, keyboard operation? |
| **YELLOW — valid, low priority** | Real, but only at the extreme | Genuine, but the fix costs the many more than it helps |
| **WHITE — persona noise** | "I hate computers", not a product problem | Resistance to the medium; a fix that adds complexity for people who are fine |
| **GREEN — feature request** | A good idea wearing a complaint as a disguise | A missing capability or a missing onboarding moment, not a defect |

## The click-count heuristic

**More than five steps to the one task is RED**, whatever the persona's technical level and
whatever their explanation. A long path to a frequent task is a finding on its own; the
persona's complaining about it is only the excuse.

## Avoid / Prefer

| Avoid | Prefer |
|---|---|
| Shipping raw persona complaints as work | Filtering first, every time, without exception |
| Staying in character through the filter | Stepping out and judging as a product person |
| Touring the features | Attempting the one task the persona came for |
| Treating a missing feature as a usability defect | Routing it to GREEN, and reading the status ledger for what is already planned |
| A persona who finds nothing | Reading zero complaints as a too-comfortable persona, not as a clean surface |
| Asserting a defect on internal state | Rendering the frame, reading the screen, naming the size |
| Several personas at once | One persona, one session, one verdict |

## Test Matrix

| Bucket | What happens next |
|---|---|
| RED | Becomes a defect with a frame as its evidence, and a test that fails for the reason it names |
| YELLOW | Recorded, not scheduled. Read again when the surface is next touched |
| WHITE | Dropped, with the reasoning kept — it is evidence about the persona, not the product |
| GREEN | Recorded as a request. It competes for the ledger with everything else, so it is not a commitment |

## When NOT to use it

- Correctness, security or performance review. This finds friction, not rightness.
- Accessibility **conformance**: `AGENTS.md` §8 is the standard, and a persona who never
  mentions contrast does not make the contrast fine. Use §8, and `rtl-aware-development`
  for text direction.
- Before anything has shipped. Friction needs a real surface and a real goal.
- As a substitute for looking at the frame. This skill tells you to; the looking is the work.