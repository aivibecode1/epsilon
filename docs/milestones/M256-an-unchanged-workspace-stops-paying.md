# M256 — an unchanged workspace stops paying for the same failure

`GateReport::carried_over` was `false` at **every** construction site in the crate. The
field existed, and so did the sentence that explains a skipped re-run:

> goal gate `cargo test` still failing (not re-run: the workspace has not changed since the
> last attempt)

`GateReport::sentence()` has a whole arm for it. Nothing has ever produced it. And
`workspace_fingerprint` — the function that exists to decide exactly this — was called from a
test and nowhere else.

M253 put gates on the round path, which is what made this cost something real: an unattended
goal with `cargo test` as its gate re-ran the suite every round, thirty seconds a time, to
learn the identical thing.

## What happens now

Hermes skips re-running a gate whose workspace has not moved since the same failure
(`goals.py:371-391`, `:1306-1320`). So does this, with three rules that the naive version gets
wrong:

**Only failures are carried.** A gate that *passed* is always re-run. The thing that made it
pass can be undone by something the fingerprint does not see — a deleted dependency, a changed
environment, an upstream service — and a replayed pass would report "done" about a workspace
nobody has checked.

**No fingerprint means no carry-over.** This is why `workspace_fingerprint` now returns
`Option<String>` instead of `String`. Outside a repository both `git` calls return nothing, so
the old version returned a **constant** — which is the safe direction for "should this re-run?"
and the *dangerous* one for "may this be skipped?". A constant fingerprint means never check
again. The absence is now carried rather than flattened into a value that looks like an answer.

**The cache is per-runner and a clone starts empty.** Copying it would let one runner vouch
for another runner's memory: a clone sharing a sandbox but pointed at a different workspace
would answer "the tree has not moved" from a fingerprint taken somewhere else. An empty clone
costs one extra gate run and can never produce a wrong "unchanged".

The replayed report is otherwise identical to the original — same exit code, same output tail
— and it is marked `carried_over`, so the sentence says why it did not run. A reader seeing a
stale answer is told it is one.

## Verification

Battery `m256` — **GATE PASS on attempt 1: 1,835 tests across 75 binaries, 0 failed**
(`audit/evidence/gate-m256.log`). Up from 1,830.

Six new tests, and the four carry-over ones build a **real git repository** in a temp
directory, because a fingerprint that cannot tell a moved tree from a still one would skip
every gate for ever. The replay test uses a command that fails *and prints a marker*, so a
carried-over report is distinguishable from a re-run by more than a flag.

**Negative control:** with the carry-over lookup disarmed,
`an_unchanged_workspace_replays_the_failure_instead_of_re_running_it` and
`a_workspace_that_moved_re_runs_the_gate` fail, and the other fifteen pass.

**Reinstall:** none. `epsilon-cli` and `epsilon-tui` are untouched; the change is inside the
gate runner both already call.

## What nothing exercises

- **The cache is in-memory, so a restart re-runs everything.** That is the safe direction and
  it is deliberate, but it means the saving only exists within one unattended session, which
  is where it matters.
- **`cargo test` is never the gate under test.** The tests use commands that fail in
  milliseconds, because a test that burns thirty seconds to prove a skip is a test nobody
  runs. The *timing* claim — that this saves real time — is therefore reasoned, not measured.
- **A gate whose failure depends on something outside the workspace** (a network service, a
  clock, an env var) will be carried over on an unchanged tree, because the fingerprint cannot
  see those. Hermes has the same property. A gate that reads the outside world is a gate
  whose verdict has a shelf life, and this does not shorten it.
- **Still no end-to-end gated-goal test** through `UpdateGoalTool`.
- **The desktop window still has no round driver**, so gates are terminal-only.