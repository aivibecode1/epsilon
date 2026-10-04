---
name: grill-me
description: Use when the user asks to be grilled or interviewed about a plan, before starting complex work (auth flows, schema changes, migrations, wire-contract changes), when a plan still has unresolved decisions, or before decomposing work across subagents.
---

# Grill Me

Interrogate a plan before any code is written, so that every branch is resolved and nothing
is silently assumed.

Two terms. The **design tree** is the plan drawn as decisions, each one branching into the
decisions that hang off it. The **frontier** is every decision whose prerequisites are
already settled — the questions that can be asked now, without guessing at answers nobody
has given yet.

## Procedure

1. **Draw the tree first.** Write down the decisions you can see, with the decisions that
   depend on each one underneath them. Do this before asking anything: a question asked
   without the tree around it is a guess wearing a question mark.

2. **Look up the facts yourself.** A question whose answer is discoverable in the workspace
   is not a question for the user. Read the code with `grep`, `read` and `glob`; let cargo
   answer what compiles and what passes; read `AGENTS.md` and the ADRs for what is already
   decided. Ask only what you cannot find. One build owner at a time: never start a build
   while a gate round or a delegated build is running.

3. **Ask the whole current frontier in one message.** Number the questions. Give each one
   your recommended answer and the one-line reason. A question whose answer depends on
   another open question belongs to a later round — never to this one.

4. **Recompute after every answer.** Each answer reshapes the tree: settled decisions push
   the frontier outward and unblock the questions under them. Re-derive it and ask again.

5. **Never block the round on one lookup.** If a fact needs a long search, only the
   questions downstream of it wait. Ask the rest of the frontier now.

6. **Synthesise when the frontier is empty.** List every decision taken, everything left
   open, and what is explicitly out of scope. Then ask for the go-ahead, and write no code
   until it is given.

## Branches the tree must cover

- **Objective and boundaries.** What is actually being achieved? What is explicitly in and
  explicitly out of scope? Who is the user of this?
- **Technical decisions.** Why this approach and not the obvious alternative? What happens
  when the second part fails? Does this codebase already have a pattern for this — if so,
  say which file, because reuse comes before addition (`AGENTS.md` §5).
- **This repository's own gates.** Is this a behaviour-level change, so it needs an ADR?
  Does it cross a wire, a trust boundary, or the permission gate? Does it touch a front end
  that must stay in step with the core?
- **Edge cases.** What happens at ten times the expected load? What if a dependency is
  down? What does the user lose if this goes wrong?

## Avoid / Prefer

| Avoid | Prefer |
|---|---|
| Asking a fact the repository already answers | Reading the file, or the crate, before asking |
| A question that depends on an unanswered question | Holding it for the next round |
| "I don't know" as a final answer | Two options, the trade-off, and your recommendation |
| Writing or editing code during the interrogation | Alignment only; code after the green light |
| Agreeing because the plan sounds fine | Looking harder when everything looks fine |
| Deciding for the user | Framing the decision and recommending an answer |

Interview in whatever language the user is writing in.

## Test Matrix

| Check | Pass looks like |
|---|---|
| Frontier order | Every question in a round had its prerequisites already settled |
| Lookup rule | No question in any round was answerable from the workspace |
| Recommendations | Every question carried a recommended answer |
| Coverage | Objective, technical, repository-gate and edge branches all appear in the tree |
| Completion | The frontier emptied before synthesis; no branch was assumed silently |
| Stop condition | Code written only after the user confirmed |

## When NOT to use it

- Code that already exists and needs review. That is a review, not an interrogation.
- A single mechanical change with one obvious implementation.
- When the user has already answered the question and asked for the work. Ask once more
  only if a *new* decision appeared.