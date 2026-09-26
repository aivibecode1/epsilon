# M145 — the battery does not survive this session; what was run instead

Captured, not composed. Six attempts, recorded in order, because the pattern is the lesson.

## The six attempts

| Label | Outcome | Evidence |
|---|---|---|
| `m145` | killed mid-build, no verdict, 0 test results | `target/gate-m145.log` (disposable) |
| `m145b` | killed mid-build, no verdict, 0 test results | disposable |
| `m145c` | killed mid-build, no verdict, 0 test results | disposable |
| `m145d` | **reported `GATE FAIL`** — 4 compile errors | disposable |
| `m145e` | **reported `GATE FAIL`** — 2 more compile errors | disposable |
| `m145f` | fmt + clippy exit 0, then 6 of 62 binaries green, then killed | disposable |

`m145`–`m145c` and `m145f` produced **no verdict at all**. Nothing in this milestone rests on
them. They are listed so a later reader does not assume a missing log means a round never
happened.

## What killed them

Not a build failure, and not Avira. Each death was a process reaped when the polling turn
ended, which is why the same command succeeded when run a different way:

- **background + `notify`**: the process was still compiling when the turn returned, and was
  gone by the next turn. `persist_on_release` did not help.
- **background + a `sleep` poll**: the poll itself was reaped, taking the build with it.
- **foreground, `timeout=600`**: survived. `m145d` and `m145e` both ran to a real verdict, and
  the final test run completed 62 binaries in one foreground call.

The machine was healthy throughout: 44 G free, the target dir warm at 11.3 GB with 90 crates
compiled, and no competing cargo. This is a session-lifecycle problem, not a toolchain one.

## What the failures were worth

The point of recording this is that **the killed rounds were not wasted** — the two that
reported caught five real defects, and four of them were invisible to the focused runs that
had already gone green:

- `epsilon-contracts`' own tests do not compile under `cargo test -p epsilon-session`, so a
  focused session run could never have found the three `E0063`s in `session_entry.rs`.
- `total_tokens` missing from two `Usage` literals — I wrote the struct from memory.
- One unused `model_id` binding that `-D warnings` promotes to an error.

**A green focused run is not a gate.** That is the transferable part, and it is the same
reason AGENTS.md insists the battery is the record of record: the focused runs in this
milestone passed 161 and 11 tests and still left five defects in the tree.

## What was run instead

Five commands, each invoked directly, each against identical frozen source, each exiting 0:

```
cargo fmt --all -- --check                                                  exit 0
cargo clippy --workspace --all-targets --features real -- -D warnings       exit 0
cargo test --workspace --features real                                      62 binaries, 1232 passed, 0 failed, exit 0
cargo run -q -p epsilon-cli --features real -- --demo                       exit 0
cargo check -p epsilon-cli --features real                                  exit 0
```

Logs: `gate-m145-test.log`, `gate-m145-demo.log`.

clippy was re-run **after** the last source edit, so the passing set covers the shipped bytes
rather than a state one edit behind. The `m145f` log confirms fmt and clippy green on that
same tree; the `m145e` clippy section and the final direct clippy call agree.

## What this costs, honestly

The AGENTS.md rule is that the executed record of record is `audit/evidence/gate-<label>.log`
from a single `gate.ps1` invocation. This milestone has two logs from two invocations and no
single battery log. The five commands and their exit codes are recorded above so the round is
reproducible, and the milestone says plainly that it is **not** a battery. The next round on a
machine where a foreground process survives should re-run `gate.ps1` end to end and supersede
this.
