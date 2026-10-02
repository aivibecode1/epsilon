# Working note — goal gates, and what is still missing

**Status: storage + enforcement landed at M240. Declaration does not.**

## The one sentence

**A goal may declare commands that must exit zero before it can be completed.** The storage is
durable (`GoalSnapshot::gates`, `#[serde(default)]` so old sessions open) and the enforcement is
in `UpdateGoalTool`'s `Complete` arm.

## The trap I walked into

The first version **refused to run a gate when the sandbox was `None`**, on the reasoning that
an unconfined session cannot produce *evidence*.

That is backwards. `bash` refuses when a backend is **missing** and runs when the user
explicitly asked for `Confinement::Off`. `Off` is a choice, not a failure to confine — and
refusing there made the whole feature inert for exactly the people who opted in.

**The rule:** fail closed when the user asked to be confined and the machine cannot. Honour the
choice when the user asked not to be confined. Those are two different situations and they must
not share a branch.

The runner shares its `Sandbox` with the shell tool, resolved from the same `Confinement` in the
same `register_builtins_with` call. That is deliberate: a gate outside confinement is a confused
deputy, and one resolution means a transcript cannot name two different backends for a tool call
and the gate that judged it.

## What is missing, in the order it should be built

> **Re-checked 2026-10-06. Item 1 below was stale and is now wrong.** `/goal edit gate
> <command>` exists (`epsilon-tui/src/app/commands.rs:819-832` → `GoalOp::Edit { gates }`, and
> the contract carries `gates` at `epsilon-contracts/src/goal.rs:324-331`), and the gate
> runner **is** wired in production (`epsilon-tools/src/lib.rs:163-166`, built from the same
> sandbox as `bash`). Item 2 is real and is now **A26** in `audit/03-defects.md`. Read the
> rows below against the code, not against this note.

1. ~~**No syntax to declare a gate.**~~ **FALSE as of M241.** `GoalOp::Edit` carries `gates`
   and `/goal edit gate <cmd>` parses it. The remaining part of this item is still true and is
   worth keeping: **the end-to-end test that drives `UpdateGoalTool` with a gated goal does
   not exist**, and a test that passes against a build where the rule does not exist is
   exactly the trap recorded below this list.
2. ~~**The round driver does not consult gates.**~~ **CLOSED at M254 (A26).**
   `epsilon_runtime::begin_goal_round` runs the gates and returns `GoalRound::GateFailed`
   carrying the failure as the continuation prompt; the terminal runs it on its own task and
   starts the round when it answers. **The cost estimate was wrong and high**: it said six
   call sites, and it was zero — `build_runtime` delegates to a new
   `build_runtime_and_gate_runner`, and `epsilon-tools` hands back the runner the registry
   was just given, so there is still exactly one sandbox. `epsilon-tools` stayed a
   **dev-dependency** of the terminal; the type crosses as `epsilon_runtime::GoalGateRunner`.
   **A gate failure still costs a round** — that is what stops a red suite being retried for
   free — and the gate runs off the frame loop, because `cargo test` is thirty seconds and
   awaiting it in `drive` is a dead terminal.
3. ~~**`workspace_fingerprint` is computed and tested for stability, never acted on.**~~
   **CLOSED at M256.** `GoalGateRunner` now remembers each gate's last **failure** with the
   fingerprint it failed at, and replays it instead of re-running. Three rules the naive
   version gets wrong:
   * **Only failures are carried.** A pass is always re-run — what made it pass can be undone
     by something the fingerprint cannot see.
   * **No fingerprint means no carry-over.** `workspace_fingerprint` therefore returns
     `Option<String>`. Outside a repository the old `String` version returned a **constant**,
     which is safe for "should this re-run?" and dangerous for "may this be skipped?" — a
     constant means never check again.
   * **A clone starts with an empty cache**, so one runner cannot vouch for another's memory.
   In-memory by choice: a restart re-runs everything, which is the safe direction.
   **The timing claim is reasoned, not measured** — the tests use millisecond commands, because
   a test that burns thirty seconds to prove a skip is one nobody runs.
4. ~~**No gate has ever run confined in a test.**~~ **CLOSED at M255.** Every gate test used
   `GoalGateRunner::new(None)` — the unconfined path — so the branch that decides whether to
   run unconfined had never executed. Four tests now cover it against
   `epsilon_safety::best_available()`, which on this machine is the **real Windows Job Object
   backend**: a gate runs *through* the backend, a failing gate reports identically there, an
   unavailable backend **refuses rather than running unconfined**, and the backend's own
   `capabilities()` declaration is pinned.
   **That last one matters more than it looks.** `epsilon-cli`'s doctor prints
   `capabilities()` verbatim. On Windows a job object bounds memory, process count and CPU and
   kills the tree on timeout — it does **not** bound the filesystem, so `WorkspaceWrite` is a
   mode label there, not an enforced boundary. `jobobject.rs` says so in a comment; it is now
   a test, because a change that made Windows claim full enforcement would pass every gate
   test and leave the doctor lying.
5. **The `/goal edit gate` parser is not driven by a test** for `gate clear` versus `gate <cmd>.

**A trap worth remembering:** a test beside the tool that exercised `goal_apply` rules **passed
against a build where the rule did not exist**, because it drove a `FakeHost`. A test that
exercises a stand-in is a test of the stand-in. Rules live in `Agent::goal_apply`; their tests live in
`epsilon-core/tests/goal_state.rs`.