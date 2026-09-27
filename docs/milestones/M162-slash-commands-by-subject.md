# M162 — slash commands by subject

## What this did

`run_command` was **1,147 lines**: one `match`, 34 arms, and no single arm larger than 97.
It is now a dispatcher (`app/commands.rs:60`) over three themed executors:

| executor | commands | lines |
| --- | --- | --- |
| `run_provider_command` | `/models` `/model` `/think` `/scoped-models` `/login` `/logout` | 125 |
| `run_session_command` | `/fork` `/new` `/resume` `/sessions` `/session` `/tree` `/clone` `/reload` `/name` | 375 |
| `run_workspace_command` | the other 19, `/theme` through `/export` | 655 |

`app.rs` went from 3,757 lines to **2,611**. Nothing about how a command *behaves* changed.

## Why the 1,147 lines were the real problem, not the file's length

M161 cut `app.rs` from 4,526 to 3,757 by moving *whole functions*. That left the actual
defect untouched: a single function holding a quarter of the crate's TUI. Nothing about it
was hard to *do* — every arm is small, and no arm is larger than 97 lines. That is exactly
why it was bad. It could not be reviewed, could not be tested one concern at a time, and
offered a new command nowhere obvious to go. A thousand lines of easy code is harder to own
than a hundred lines of hard code, because nobody ever opens it.

## The move was by line range, and the arms are provably intact

Every arm was sliced out of `app.rs` by measured line range rather than retyped, then
compared against `HEAD` with the whitespace normalised (rustfmt re-indents the arms one level
deeper inside the new `impl`). **34 of 34 arm bodies are byte-identical.** Because the arms
already sat at the same depth inside the same `match`-inside-`fn` shape, the move needed no
re-indentation at all.

### The checker was wrong three times first

Worth writing down, because the pattern is the lesson:

1. **Off-by-one in the ranges.** Arm ranges were taken as `[head, next_head]` inclusive, so
   every one of the 33 boundary lines landed in *two* arms. The first build produced a
   duplicated `Command::Logout(provider) => {` header. Caught by the compiler, not by me.
2. **A prefix collision in the comparator.** Searching for `Command::Model` also matches
   `Command::Models`, and `Command::Session` also matches `Command::Sessions`, so seven arms
   compared unequal against the *wrong* arm. Fixed with `(?![A-Za-z_0-9])`.
3. **The search matched the dispatcher, not the executor.** `Command::Models` appears twice in
   the new file — once in the dispatcher's or-pattern list at indent 12, once as a real arm at
   indent 16 — and the regex found the dispatcher first. Fixed by carving out each executor
   body before searching inside it.

Each time the code was right and the harness was wrong. A verification tool that has not
been shown a known-bad input is not evidence.

## The routing tripwire, and the control that proved it works

The dispatcher names two groups and sends **everything else** to `run_workspace_command`. A
new `Command` variant therefore cannot be silently unrouted: it lands in the third executor,
where the compiler insists its arm exist. A router exhaustive by name would instead turn a
forgotten command into a runtime no-op — the kind of bug that ships.

Each executor's inner `match` must still be exhaustive over the whole enum, so each ends
with a fallback that is loud in test builds and silent in release:

```rust
#[allow(unreachable_patterns)]
other => {
    debug_assert!(false, "{other:?} is not a run_session_command case");
    return false
}
```

A user's release binary must not panic over an internal routing mistake; a test must not be
able to pass by quietly doing nothing. Both, not either.

**Control:** misrouting `Command::Timeline` from the workspace executor to the session
executor. The suite went **363 passed, 1 failed**, and the failure named the mistake exactly:

```
panicked at crates/epsilon-tui/src/app/commands.rs:591:17:
Timeline is not a run_session_command case
```

That is the whole reason the tripwire exists, demonstrated rather than asserted.

## Also fixed

`run_command`'s doc comment was duplicated — the line
`/// Execute a parsed slash command; returns `true` to quit the session.` appeared twice in a
row. It was inside the range that moved and is written once now.

## Verification

| gate | result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo test --workspace --features real` | **62 binaries, 1,288 passed, 0 failed** |
| `cargo clippy --workspace --all-targets --features real -- -D warnings` | exit 0 |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0, 6 messages, `tokens in/out: 300/150` |

`epsilon-tui` is 364 passed, unchanged from M161 — as it must be, since no test was added and
no behaviour changed. The count is the same tests, not a smaller set.

## Limits — what this milestone does *not* prove

- **No new test was added, and none should be.** A guard that fires on a routing mistake is
  the test; a test that enumerates all 34 variants would have to construct `Login { key }`
  and `Logout(provider)` payloads, and calling `/login` in a test writes to the user's real
  credential store. The structural guarantee does not need that risk.
- **`app.rs` is still 2,611 lines**, holding state mutation, viewport measurement, the
  editor bridge and the OAuth UI. The remaining seams are separated by *kind* rather than
  shape, and the `Composer`/`App` split into files (as opposed to impl blocks) is still ahead.
- **`run_workspace_command` is 655 lines and is the next thing to break up.** It is the
  catch-all by design, which is right for routing and wrong for cohesion: `/export` and
  `/theme` have nothing in common but being unmiscellaneous.
- **No real-console run**, because nothing about the frame or a keypress changed. The bytes
  on screen are the same bytes; the 364 TUI tests cover the panels and commands unchanged.
