# Write a handoff

Use this skill when a session is ending, context is nearly full, or another
person or agent will continue the work. A handoff is not a transcript summary;
it is the smallest set of facts that lets the next session act without
rediscovering them.

## Structure

**Objective.** One paragraph: what we are trying to achieve and why. Quote the
user's words when the wording matters.

**State.** What exists now: branches, files, running processes, installed
builds, open sessions. Use absolute paths or repository-relative paths, not
"the file we discussed".

**Decisions.** Each decision with its reason and the alternative rejected. The
reason is the part that is expensive to recover.

**Changes.** Files created or modified, each with one phrase. Note anything
deleted and why; deletion is easy to mistake for an accident.

**Verification.** Commands that were actually run and their result. Do not write
"tests pass"; write the command and the count you measured. Name what was not
verified.

**Open work.** The next concrete action, then the remaining tasks in order.
Each item says what "done" looks like.

**Risks.** What could break, what is fragile, what is deliberately deferred. Mark
a deliberate shortcut as deliberate so the next reader does not "fix" it by
accident.

## Rules

- Separate **fact** from **inference**. "The provider returns 429" and "I think
  the provider is rate-limiting" are different sentences.
- Prefer pointers over prose: a path, a test name, a log line. A pointer stays
  true when the summary drifts.
- Keep it under about 400 words unless the objective genuinely needs more.
- Do not include secrets, tokens, or personal data.
- End with the one command the next session should run first.

A good handoff survives the reader having no memory of this session. Read it
back as if you were that reader; if a step needs context that is not in the
document, add it.
