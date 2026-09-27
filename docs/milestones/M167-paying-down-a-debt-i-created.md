# M167 — paying down a debt I created in M162, and the 18 commands no test runs

## What this did

`run_session_command` — 387 lines, nine arms, the largest method in the TUI — became a
dispatcher over three executors:

| new executor | commands | arm lines |
| --- | --- | --- |
| `run_navigate_command` | `Sessions`, `Session`, `Tree` | 166 |
| `run_branch_command` | `Fork`, `Clone`, `Name` | 123 |
| `run_restore_command` | `New`, `Resume`, `Reload` | 83 |

All nine arm bodies are verbatim. `run_session_command` keeps the `debug_assert!` catch-all,
and each executor carries its own copy, with its own name in the message.

## First: I was wrong about `over_ui`

I said last turn that `over_ui` was 206 lines and the largest method left. It is not. There
is exactly **one** `over_ui` in the crate — a 10-line test seam at `app.rs:834` that builds
an `App` over a caller's `UiState`.

The 206 came from the same measurement I had already proved unreliable in M164: deriving a
method's length from the gap to the next method head. A detector that misses a method
attributes its span to the previous one. I used that broken number twice — once to choose
the slice, once to tell you. Measuring properly (scanning for each method's real closing
`    }`) shows the largest methods are the four command executors **I created in M162/M163**:
387, 308, 289, 137.

**I traded one 1,147-line dispatcher for four smaller ones and never split inside them.**
`run_session_command` at 387 is larger than anything the original audit complained about. The
M162 lesson — group by subject, not by arm size — was applied at the top level and not
underneath it. This milestone starts paying that down.

## The control fired vacuously, and that was the finding

The obvious control is a routing mutant: send `Name` to `run_navigate_command` and check the
guard complains. I did that. **365 passed, 0 failed.**

Two explanations were possible and they mean opposite things. The workspace has no
`[profile.test]` override, so `debug_assert!` was live — the guard was not silenced. Which
left the other possibility, and it was the true one: **no test runs `/name`.**

So I measured the whole class rather than patching one instance. Mapping every slash token to
the `Command` it produces, then searching the TUI test corpus:

> **18 of 30 user-typable slash commands are never executed by a single TUI test.**

```
/branch /tree /fork /clone /new /resume /trust /reload /export /import
/share /name /changelog /settings /scoped_models /skills /logout /exit
```

Confirmed by a second, independent search, as a negative claim requires. For `/tree`,
`/name`, `/fork` and `/export`, the literal `"/token"` occurs **only in `picker.rs`** — the
parser — and `Command::X` is constructed only in `commands.rs` and `picker.rs`. The
positive control on the method is `/search`: 4 hits in `app/tests.rs`, and it is correctly
absent from the uncovered list.

This is the same defect class as the M160 `0ms` durations and the M165 ungutted OAuth
adapter: **a code path the product advertises and no test exercises.** `/name` alone is a
54-line arm — third-largest in the crate — that has never run.

## The control that does work

Same mutant, applied to `Sessions`, which *is* covered by
`sessions_command_browses_and_enter_switches_the_session`:

```
363 passed, 2 failed
Sessions is not a run_branch case
  app::tests::sessions_command_says_when_there_is_nothing_to_browse
  app::tests::sessions_command_browses_and_enter_switches_the_session
```

So the routing guard is real and the split is correct. The two failures also show the shape
of the gap precisely: the *tested* commands are guarded by tests, and the untested ones are
not guarded by anything.

A note on the first measurement, because it was wrong in an instructive way: I first tried to
count this by searching for `Command::Variant` in test files and got `0/12 covered`, which
is obviously false — `/search` is demonstrably tested. Tests drive commands by *typing* the
slash text, not by constructing the enum. The number that came back was not a small error;
it was an artefact of searching the wrong axis, and I would have reported it if I had not
sanity-checked it against a command I already knew was covered.

## Verification

| gate | result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --features real -- -D warnings` | exit 0 |
| `cargo test --workspace --features real` | **62 binaries, 1,289 passed, 0 failed** |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0 |

`epsilon-tui` 365, unchanged. No test added or removed.

## Limits — what this does not prove

- **The 18 untested commands are still untested.** This milestone *measured* the gap and
  proved the routing around it; it did not close it. That is the next slice, starting with
  the largest untested arms (`/tree` 97 lines, `/trust` 78, `/share` 73, `/export` 63,
  `/name` 54). A count of "18" is a to-do list, not a fix.
- **`run_transfer_command` (308), `run_view_command` (289) and `run_provider_command` (137)
  are untouched** and are the same shape this milestone just split.
- **The 18/30 figure counts commands, not behaviour.** A covered command may still be
  covered shallowly — one happy path, no error path. Arm-level coverage was not measured,
  and the slash-token search cannot see a test that drives a command some other way.
- **No real-console run**, because no frame or keypress changed.
