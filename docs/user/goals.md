# Goals — one long-running objective per session

A **goal** is a sentence describing what you are ultimately trying to achieve, recorded in the
session file, shown on the footer, and worked on by the harness between turns. It is for work that
spans many turns — "keep the documentation gates green", "port the last three RPC ops", "get the
Windows build clean" — and it is the wrong tool for a single request.

```text
/goal keep the doc gates green          ● goal 1/256   the first round starts immediately
   …the model works, reports, stops…
   …the harness starts the next round…   ● goal 2/256
   …the model marks it complete…         ✓ goal complete 3/256
```

## The command

| Command | What it does |
|---|---|
| `/goal <objective>` | Start a goal, arm it, and begin the first round now |
| `/goal` | Show the current goal, or say there is none |
| `/goal help` | List the verbs |
| `/goal edit <objective>` | Rewrite the objective; the phase does not change |
| `/goal edit rounds <n>` | Raise or lower the round cap |
| `/goal pause` | Stop continuing; the goal is kept |
| `/goal resume` | Continue again, and clear any blocker |
| `/goal complete` | Mark it achieved |
| `/goal blocked <code> <reason>` | Record a concrete condition that has to change first |
| `/goal clear` | Remove it. Its history stays in the session file |

Only a leading word that is *exactly* one of those verbs is treated as a verb: `/goal clear the
failing gates and pause nothing` is an objective, not a `clear`.

## What the harness does between turns

At the end of a turn, if the goal is **active, armed in this process, and under its cap**, the
harness sends the next round itself. A round is an ordinary user-role message — you can read it in
the transcript, it is in the session file, and a resumed session replays it — naming the objective,
the round and the cap, and telling the model what ends the loop.

It does **not** start a round when any of these is true:

- the goal is paused, completed, blocked, or out of rounds;
- a run is already in flight, or a permission dialog is waiting for you;
- **you are part-way through typing.** A draft means you are mid-sentence; the round is held, not
  lost, and your own turn continues the goal.
- **the last run produced nothing at all** — a provider error, an expired credential, a network
  failure. Repeating a round that cannot make progress would spend the whole budget doing it.

## Stopping it

- **`Esc` during a run disarms the goal** and says so. The goal itself is untouched — the harness
  simply stops continuing it on its own until you `/goal resume`. This is deliberate: an abort is
  you saying stop, and a harness that restarts the instant the turn ends is arguing with you.
- `/goal pause` does the same thing deliberately, and keeps the phase as `paused` so the badge
  reads `○ goal paused 2/256`.
- `/goal complete` is for when the objective is achieved.
- `/goal clear` removes the goal entirely.

## What survives a restart, and what does not

| | Survives | Why |
|---|---|---|
| the objective, the phase, the revision, the rounds spent | **yes** | it is in the session file, like every other fact about the session |
| permission to continue it by itself | **no** | the person who armed it is not necessarily the person who opens the session next. A fresh process reads the goal, shows it as `○ goal 1/256`, and waits for `/goal resume` |

The same applies after `/fork`, `/clone` and `--resume`: the goal comes back, the arming does not.

## Cost, and the cap

Rounds are the only budget a goal has — there is no token, currency or wall-clock limit, and that
is a deliberate choice rather than an unfinished one. The cap defaults to **256 rounds** and is
what stops a loop nobody is watching:

- `/goal edit rounds <n>` raises or lowers it;
- the harness warns as it runs down, at 10% of the cap, 4%, 2%, and the last round, each said once,
  with the two commands that do something about it;
- reaching the cap stops the loop and says so.

A goal that has spent its cap cannot be resumed until the cap is raised. All of this is in the
session file, so `/goal` after a restart shows the rounds already spent.

## What the model can do

The model has the same controls, as three tools, so it can finish a goal instead of being told to:

| Tool | What it does |
|---|---|
| `create_goal` | Start one — only when you asked for long-running work |
| `get_goal` | Read the objective, phase, rounds and the `{id, revision}` an update needs |
| `update_goal` | `edit`, `pause`, `resume`, `complete`, `blocked`, `clear` |

Every update carries the exact `{id, revision}` that `get_goal` returned, and a stale pair is
refused rather than applied — so a tool call that read the goal ten calls ago cannot overwrite a
change you made in between. `blocked` is refused before three rounds of work: being stuck on the
first obstacle is not a blocker, and difficulty, uncertainty or remaining work never are.

The objective also rides the prompt, so every turn knows what the session is for without being
reminded. A session with no goal produces exactly the prompt it produced before goals existed, so
this costs nothing when you do not use it.

## Limits

- **Headless modes do not drive rounds.** `-p`, `--serve` and `--rpc` run a turn and stop; a goal
  in a headless session is state without a driver. That is deliberate — a batch run has nobody to
  notice a loop — and it means `/goal` is a TUI feature today.
- **There is no evaluator.** The caller that records completion is authoritative: the harness does
  not check that the objective was really achieved, and the cap is the only thing between a
  confused model and a long, expensive loop.
- **One goal per session.** Parallel objectives are not supported; a completed goal can be
  replaced, an unfinished one has to be paused, completed, blocked or cleared first.
- **The goal is visible to clients** through the shared state payload (`rpc get_state`,
  `GET /state`), including whether the harness is armed — but there is no way to *change* a goal
  over the wire yet.
- **A goal is not scoped to a branch.** Navigating the session tree rewinds the conversation, not
  the objective: the goal is the session's, not the lane's.
