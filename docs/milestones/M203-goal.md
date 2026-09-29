# M203 — `/goal`: one persisted objective, readable by the human and the model

**Status:** landed. GATE PASS `m203` — see [Verification](#verification). Reinstall: **required**
— `/goal`, the footer badge and three new tools are all reachable from the installed binary.

## What

The harness's first capability that outlives a turn without outliving the *conversation*: one
long-running objective per session, recorded in the session log and nowhere else.

| Piece | What it does |
|---|---|
| `epsilon-contracts::goal` | `GoalSnapshot` / `GoalStatus` / `GoalView` / `GoalOp` / `GoalError` — DSH's goal domain (`packages/goal/goal/src/types.ts`), narrowed to what this harness drives |
| `EntryKind::Goal` | The one durable change: a complete post-mutation snapshot, or a revisioned tombstone for `clear`. `SessionStore::goal()` folds it in file order, exactly as `labels()` folds labels |
| `Agent::goal` / `goal_apply` / `goal_begin_round` | The service: compare-and-set mutations, arming that follows the phase, and round counting |
| `create_goal` / `get_goal` / `update_goal` | The model's side, over `ToolContext::goal_host` — the same inversion `task_host` uses |
| `/goal` | The human's side: `create`, `edit`, `pause`, `resume`, `complete`, `blocked`, `clear`, and the bare form that reports |
| The footer badge | `● goal 2/256` — the filled mark is the difference between "this continues by itself" and "this is waiting" |

## The five rules, and why each one is load-bearing

1. **The session log is the only store.** No side file, no settings key, no in-memory service
   that a restart loses. `SessionStore::goal()` derives the answer, so a goal written by another
   frontend — or by an earlier build — folds the same way.
2. **Compare-and-set.** Every mutation carries the exact `{id, revision}` it read. A *user*
   typing `/goal pause` sends none (they are looking at the goal), and a *model* tool call always
   sends one, because a tool that read the goal ten calls ago must not overwrite what the user
   changed in between. `a_stale_writer_is_refused_rather_than_obeyed` is the test.
3. **Activation is process-local.** `armed` is never persisted and is false in a process that has
   not asked. A goal survives a restart; the permission to keep working on it unattended does
   not, because the person who armed it is not necessarily the person who opens the session next.
4. **One current goal, replaceable only when finished.** A create is refused while an unfinished
   goal holds the slot; a *completed* one may be replaced without an explicit clear, which is
   what stops `/goal` being a command a user has to undo first.
5. **Rounds are the only budget, and blocking has a floor.** A round is counted when it is
   *sent* — one that crashed on the way was still spent — and the `blocked` verb is refused
   before `BLOCKED_MIN_ROUNDS` (3), DSH's guard against a model that calls its first obstacle a
   blocker.

## Where the divergence from DSH is, stated rather than implied

- **No round driver.** DSH splits the domain from `dsh-goal-round-driver`; `goal_begin_round()`
  exists, is tested, and returns the prompt a continuation would send — but nothing in the TUI
  calls it yet, so a goal today changes what the model is *told* (the objective rides the
  CONTEXT tier) rather than what the harness does between turns. That is the next slice, and it
  is the reason this one is small enough to verify.
- **Three tools, one file.** DSH ships them as three packages because the authority differs per
  verb; here they are three tools in `epsilon-tools/src/goal.rs`, which is the same contract in
  one crate. The authority rule lives in their descriptions.
- **`GoalError::NoStore` is ours.** DSH has no equivalent because a goal service that is mounted
  always has a session log. Epsilon has frontends without one (a `task` child, `--print`, a test
  agent), and "no goal is set" is the wrong sentence for them: nothing the user types would ever
  set one. Found by writing the test that expected the right sentence and getting the wrong one.

## What the tests cover

- `crates/epsilon-core/tests/goal_state.rs` (10 tests) — the lifecycle through the real `Agent`:
  durability across a reopen, a fresh process not inheriting activation, arm/disarm per phase,
  one-current-goal, the stale-writer refusal, a late tombstone not removing a newer goal, the
  blocker round-trip, round counting and the cap, edit validation, and the empty session.
- `crates/epsilon-tools/src/goal.rs` (6 tests) — the model's path: the ref a writer needs, "no
  goal" as a statement rather than an error, the blocking floor, a stale refusal that says what
  to do, clear as an outcome, and a session with no store saying so.
- `crates/epsilon-tui/src/app/goal_tests.rs` (7 tests) — the human's path and the frame: the
  command reaching durable state, the badge *rendered*, the badge whole-or-absent at 81 widths,
  every verb, an objective that starts with a verb word, every refusal visible, and bare `/goal`.
- `crates/epsilon-core/src/prompt.rs` — the goal rides CONTEXT and an absent goal leaves the
  prompt byte-identical.

## Limits — what this does not prove

- **Nothing continues a goal yet.** `goal_begin_round` is tested and unwired; a goal is currently
  a durable, visible, model-visible objective that a human starts each turn. The auto-continuation
  driver is M204's slice and the reason the badge distinguishes armed from disarmed *now* — the
  state it shows is real even though nothing consumes it.
- **The prompt's goal block is not scoped to the active branch.** Neither is the goal itself
  (`SessionStore::goal` walks the whole file, like `labels`). Branch navigation rewinds a
  conversation; it does not un-set an objective, and a goal that vanished because the user looked
  at another branch would be worse. Recorded because it is a decision, not an oversight.
- **No token, currency or wall-clock budget.** DSH is explicit that `max_goal_rounds` is the only
  budget and that absence is deliberate. A goal that runs 256 expensive rounds is bounded by the
  round count and by the human noticing.
- **`clear` keeps a tombstone the fold ignores unless it names the current goal** — tested — but
  a `Set` whose revision does not follow the previous one is *applied*. That is DSH's rule
  relaxed deliberately: refusing to derive state because a number skipped turns a writer's bug
  into an unreadable view. The compare-and-set still guards every mutation, which is where the
  number actually matters.
- **The badge is refreshed at two points** (a `/goal` command and the end of a turn), not on a
  goal-change event. A tool that changes the goal mid-turn shows up at turn end. An event would
  be better and is not worth a new variant on the wire until something else needs it.

## Verification

GATE PASS, 2026-09-29 — battery **`m203`**, run against this commit's contents with the
documentation already final. Seven gates, exit 0 each: `check-links` 0 s (445 relative links,
0 broken), `check-lint-tables` 0 s, `fmt --check` 3 s, `clippy --workspace --all-targets` 5 s,
`test --workspace` 308 s, `demo` 185 s, `real-check` 29 s.
**1,607 tests across 64 binaries, 0 failed** — summed from the 64 `test result: ok.` lines in
the log. M202 measured 1,579 across 63, so this slice adds 28 tests and one binary
(`crates/epsilon-core/tests/goal_state.rs`).

Evidence: [`audit/evidence/gate-m203.log`](../../audit/evidence/gate-m203.log).

**Reinstall:** required. `/goal`, the footer badge and the three tools are all reachable from the
installed binary, so `cargo install --path crates/epsilon-cli --features real --force` was run
after the battery passed.

**What the battery cannot see.** Nothing in it drives a goal through a real console: the
`/goal` path is covered by rendered `TestBackend` frames and by the tool surface, and the
*installed* binary's copy of the badge is covered only by the console probe run by hand. The
[console probe](../../scripts/console-probe.py) can type (`--typed`), so the end-to-end check is
one command — it is recorded here as the manual step rather than implied to be automatic.
