# M205 — the harness says what it knows

**Status:** landed. GATE PASS `m205` — see [Verification](#verification). Reinstall: **required**
(both changes are things the user reads).

## What

Two sentences the harness could always have said and did not. Neither is a bug fix; both are the
class the M196 invariant is about — **a state the UI can be in that the user cannot interpret**.

### 1. `this session already holds N entries …` (ledger A17)

A plain `epsilon` launch opens the session file, appends its new turns to it, and starts with an
**empty in-memory history** — `restore_history` runs only on `--resume`/`-c`. So a returning user
gets a first-run screen while the header reports a few percent of the context already used. Both
facts are true (that figure is the system prompt and the tool schemas, M194's C2 fix) and together
they read as a bug: the screen says nothing has happened, the header says something has.

The condition is exact rather than a guess — *the store has entries and this process is not
carrying them* — and that is exactly the state that looks wrong, so the notice is silent for a
fresh session and silent for a resumed one. It names the count and the command that brings the
history back (`epsilon -c`, or `/resume <file>`) instead of leaving the user to discover that the
file still has it.

### 2. `goal has N of M rounds left …`

Rounds are the only budget a goal has — DSH's design, and deliberate — which makes the cap the
thing that stops a loop nobody is watching. A user who set a goal and walked away got no signal at
all until it stopped, and at a real provider's prices that is a money question rather than a
tidiness one.

A **ladder** rather than a threshold: 10% of the cap, 4%, 2%, and the final round, each firing
once as it is crossed, so the warning cannot become the noise that gets ignored. The steps are
fractions rather than counts because `/goal edit rounds <n>` means a cap can be any number at all.
The notice names the two commands that do something about it.

One detail that had to be right: the ladder reads the count *after* the round that is starting, so
the two notices a continuing goal prints agree about which round it is on. The first version read
the pre-round snapshot and the pair read `goal round 90/100 — continuing` beside `11 of 100 rounds
left`.

## Where both live

- `App::note_prior_history`, called once from `app/event.rs` after the startup notices.
- `App::note_goal_budget`, called from `continue_goal` when a round actually starts.

Neither adds a wire field, a config key or a dependency.

## Limits — what this does not prove

- **The prior-history notice is one launch late for a session that is *empty* because it was just
  created.** That is the intended silence, not a gap, but it does mean a user who clears a session
  and relaunches sees nothing — which is correct and worth stating.
- **Nothing tells the user the *other* half**: that a plain launch's new turns join the same file
  as the old ones, so `-c` later mixes two conversations into one context. The notice says the
  history is not loaded; it does not say the file keeps growing. A17 was scoped to the sentence
  that closes the confusing screen, and the file's identity (`tui.jsonl` = one rolling session per
  frontend, or one per launch?) is A16's open design question.
- **The budget ladder is not tested at a cap other than 100.** The arithmetic is integer division
  with a `max(1)` floor, so a cap of 5 gives `{1, 1, 1, 1}` — one warning at the last round — and
  a cap of 3 gives `{1, 1, 1, 1}`. That is the intended degradation and it is read, not run.
- **Neither notice is in a golden frame.** They are asserted at the notice channel, which is where
  a startup notice and a driver notice actually go; a frame test would prove the renderer, which
  the goal badge already covers.

## Verification

GATE PASS, 2026-09-29 — battery **`m205`**, run against this commit's contents with the
documentation already final. Seven gates, exit 0 each: `check-links` 0 s (452 relative links,
0 broken), `check-lint-tables` 0 s, `fmt --check` 2 s, `clippy --workspace --all-targets` 1 s,
`test --workspace` 283 s, `demo` 27 s, `real-check` 3 s.
**1,617 tests across 64 binaries, 0 failed** — summed from the 64 `test result: ok.` lines in the
log. M204 measured 1,613 across the same 64, so this slice adds the four tests above.

Evidence: [`audit/evidence/gate-m205.log`](../../audit/evidence/gate-m205.log).

**Reinstall:** required — both changes are sentences the user reads — and done.
