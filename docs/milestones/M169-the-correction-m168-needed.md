# M169 — the correction M168 needed, and a hazard I could not test

## Why this milestone exists

M168's commit message says nine call sites could "panic the TUI when no session store is
attached", and its milestone says typing `/tree` in a plain `--tui` run took the TUI down.
I went to verify that against a real console and found it was not true. This milestone is
the correction, plus the one thing I found while checking.

## The M168 claim was wrong

`crates/epsilon-cli/src/main.rs:1764` opens the session store and `?`-propagates any
failure. Line 1813 attaches it, unconditionally, before the TUI ever starts. Two independent
searches (`git grep attach_session_store` and a read of every `detach` site) agree that all
four production attach paths are unconditional: the TUI, the serve path, and both child-agent
paths.

**So a shipped `--tui` run always has a store, and the "no store attached" panic was not
reachable by a user typing a command.** M168's severity claim was wrong, and its stated
mechanism was wrong too. The milestone now carries a correction block; the code change
stands, because it matches the documented contract and the precedent already set by
`/session`, but calling it a live crash was overstated.

I did not check this before writing the claim. The lesson is the one this project keeps
learning: **a defect's severity is a claim about reachability, and reachability needs its own
search.**

## What is actually reachable

`Agent::live_resume` (`epsilon-core/src/lib.rs:803`) is the only place that detaches a store
the agent is using:

```rust
let restore_target = if already_holding {
    self.detach_session_store();   // :816
    drop(previous);                // the lease has to go before the reopen
    None
} else { previous };
```

When re-opening the file it already holds, the old handle must go first or the cross-process
write lease conflicts with itself. Then:

- `repair_on_open` returning `Err` — `:849` — `return Err(...)` with nothing re-attached.
- `model_view()` returning `Err` — `:854` — `?` with nothing re-attached.

Either leaves the agent with **no store**, permanently, and every session-reading command
would then panic. The user did nothing wrong: they resumed the file they were already in.

Fixed by attaching the new store *before* anything that can fail, keeping a clone to lock
while repairing. No error path can now leave the agent detached.

## I could not write a test for it, and did not ship one

Two attempts, both removed:

1. **A behavioural test.** Attach a store, `live_resume` a file that cannot be parsed, assert
   a store remains. `repair_on_open` only fails via `branch_entries()?` or a failed append —
   i.e. when the backing file becomes unreadable, which a test cannot arrange without racing
   the code it is testing. Junk content does not make it fail; repair just drops the bad
   entries and succeeds. **The control passed 1/1 with the fix reverted.**
2. **A source invariant** — "the attach appears before the first `return Err` after the
   detach". It cannot distinguish the two versions, because the *rollback* attach at `:829`
   sits between the detach and that first `return Err`, so the check passes either way.
   **The control passed 1/1 again.**

Neither test could fail, so neither is evidence, and a test that cannot fail is exactly the
sin this project has been catching in others since M160. Both are gone. The fix stays because
it is strictly safer and costs nothing — not because a test proved it.

**What would actually prove it:** a fixture that makes the backing file unreadable between
the successful open and the repair. That needs either a hook in `SessionStore` for the test
to drive, or a fault-injection point in `live_resume`. Neither exists, and adding one to
production code for an untestable hazard is its own cost. Not done.

## Verification

| gate | result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --features real -- -D warnings` | exit 0 |
| `cargo test --workspace --features real` | **62 binaries, 1,295 passed, 0 failed** |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0 |

Test count is unchanged from M168, which is the honest result: a fix with no test adds no
test.

## One build failure worth recording

The first run of this milestone died with `rustc.exe` exiting `0xc0000142,
STATUS_DLL_INIT_FAILED` — the Windows compiler-startup contention in AGENTS.md. A cargo from
a timed-out foreground call was still compiling; I let it drain before retrying, rather than
starting a second build owner. The rerun was clean.

## Limits

- **The fix is unproven by test.** Stated above and not softened.
- **The real-console check I set out to do was not done** — I went to verify M168's claim,
  found it false, and spent the milestone on the correction instead. A plain `--tui` run
  typing `/tree` and `/export` is still worth doing, for the M168 code path rather than the
  crash claim.
- **Sixteen of the eighteen uncovered slash commands remain uncovered.**
- **A future change could reintroduce the early return** and nothing would catch it.
