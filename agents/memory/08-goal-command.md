# Working note — `/goal`: the next capability slice

**Status:** designed, not started. This note exists so the design work survives the context
window; the reference reading is done and the decisions below are the ones that cost time.

## What the reference actually specifies

DSH ships goals as a *domain* (`packages/goal/goal`), with the tools, the `/goal` command and the
round driver as separate consumers of one state. Read
`references/deepseek-harness-dsh-v0.2.0-rc.1/packages/goal/goal/README.md` and
`.../docs/subsystems/goal.md` before writing code; the summary that matters:

- **One current goal per session.** No parallel objectives; a finished goal may be replaced.
- **Event-sourced, and the session log is the only durable store.** Every mutation appends a
  durable `goal/change` carrying the complete post-mutation snapshot; `clear` writes a
  revisioned tombstone. Nothing about a goal lives outside the log.
- **Four durable phases** — `active`, `paused`, `blocked`, `complete` — plus a **process-local**
  `armed`/`disarmed` flag that is never persisted. Every session-start edge (resume, fork,
  restart) disarms, so an armed goal cannot continue by itself after a restart until someone
  resumes it.
- **Compare-and-set.** `get_goal` returns a detached view with `{id, revision}`; every mutation
  carries that exact pair and a stale one is refused. This is the whole staleness guard.
- **Rounds.** `maxGoalRounds` (default 256) bounds automatic continuation; a round advances only
  on an admitted goal-sourced `user/message`. Blocking requires the same condition to persist
  for ≥3 admitted rounds *by default* — "difficulty, uncertainty, or useful remaining work is not
  blocked".
- **Tools**: `create_goal`, `get_goal`, `update_goal` (verbs `edit`/`pause`/`resume`/`complete`/
  `blocked`/`clear`). **Command**: `/goal [<objective>|clear|edit <objective>|pause|resume]`.
- **Deliberately absent**: token/currency/wall-time budgets, an independent evaluator, a goal
  database, and any scheduling policy (that is the round driver's job).

## What Epsilon already has that this must use

- `epsilon-session`'s append-only JSONL tree with `EntryKind` variants, a strict decoder, and
  `KNOWN_ENTRY_KINDS` — a new `goal` entry kind is the natural durable home, and the M202
  amendment governs any field it gains later.
- `epsilon-contracts` for the types (`GoalId`, `GoalSnapshot`, `GoalStatus`, `GoalView`) — no
  new crate, which ADR-014 requires anyway.
- The tool registry (`epsilon-tools`) for the three tools, and `app/commands.rs` +
  `picker::SLASH_ITEMS` for `/goal`, which is where "every palette entry must name a command
  `parse_command` can read" is enforced.
- The TUI's header/footer for the one-line goal state, and the composer's context stack as the
  place a goal strip would go if it needs more than a line.

## The decisions this slice has to make, listed so they are not rediscovered

1. **Does `epsilon` grow an auto-continuation loop, or only the state and the command?** DSH
   splits them (domain vs `goal-round-driver`). Starting with state + command + tools, and
   *reporting* rounds rather than driving them, keeps the loop untouched — which the repository
   requires (ADR-002: port the loop shape verbatim, improve only at named seams).
2. **Where activation lives.** It is process-local by design in DSH. In Epsilon the equivalent is
   an `Arc<AtomicBool>`-shaped flag on the agent, set false on every startup path, so a resumed
   session never continues on its own.
3. **What the model sees.** DSH adds no tokens on a mutation: the tools return state and the
   driver renders the objective when it schedules work. If Epsilon injects the objective into the
   prompt, the injection must be reconstructable from the log (DSH's "model-visible ⟺ logged"),
   which means an entry, not a side channel.
4. **The TUI surface.** One line in the header is enough to start (`● goal 3/256 · fix the doc
   gates`), and it must appear in `/help` and in the ADR-011 "anything the UI advertises must
   work" table. A strip above the composer is the upgrade path if one line is not enough.

## The first RED this slice can have

Unlike M202, this one has a clean pre-fix RED for every piece: a `goal` entry the current build
calls *unknown* (so a session written by the new build refuses to open in the old one — expected
and correct), `/goal` today falls through to `unknown command`, and `create_goal` does not exist
in the tool registry. Start with the durable kind and a round-trip test, then the command, then
the tools.
