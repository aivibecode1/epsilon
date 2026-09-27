# M172 — the list the parser owns, and the seven commands it found

## What this does

M171's sweep drove all 30 commands but kept a **hand-written list** in the test file, and
its own milestone said so: *"a new command added without updating it is a silent gap."*
That is not a limit to record and move past when the fix is small. This removes the copy.

`COMMAND_HEADS` now lives in `picker.rs`, next to the `match` it describes, and the sweep
reads it. Two checks enforce that the two cannot drift:

| test | direction | what it catches |
| --- | --- | --- |
| `every_head_in_command_heads_is_parsed` | list → parser | a listed head that no longer parses |
| `the_parser_does_not_know_a_command_the_list_does_not` | parser → list | **a new command no test drives** |

The second is the direction that matters, and the one a hand-written list structurally
cannot check. It reads `parse_command`'s own arms out of the file, so it cannot lag behind
the code.

## It found seven commands on its first run

Not manufactured, not predicted — the check's first execution:

```
the parser accepts ["timeline", "help", "login", "auth", "connect", "quit", "q"]
but no test drives them
```

**M167's census and M171's sweep were both wrong, and in the same way.** M167 counted
`Command` *variants*; M171 counted a hand-typed list. Neither counted what the parser
actually accepts. Seven user-typable heads had no test driving them.

Classified from the arms themselves rather than guessed:

```
"timeline" | "branch"  => Some(Command::Timeline)
"help" | "?"           => Some(Command::Help)
"login" | "auth" | "connect" => Command::Login
"quit" | "q" | "exit"  => Some(Command::Quit)
```

- `help` and `login` are **new variants** → added to `COMMAND_HEADS`, so the sweep now runs
  31 of 32 heads.
- `timeline`, `auth`, `connect`, `quit`, `q` are **aliases** of a listed head → added to
  `ALIAS_HEADS`, each annotated with the command it aliases. `/timeline` is swept through
  `/branch`, which reaches the same arm.
- `?` is not listed: it is the only non-word head, and the reverse check filters those out
  rather than reporting them. That is stated in the code so it reads as a decision.

This is the payoff of writing the check as a *guard* rather than as a count: the count was
wrong, and the guard found the discrepancy on the day it was written.

## What I broke on the way

A whole-file Python rewrite of `picker.rs` destroyed it — 1,470 lines replaced by the 65
lines of my own test code. This is the third time in this project a whole-file Python write
has damaged a source file, and the fourth recorded instance of the same lesson in the error
log. Restored with `git checkout --`, then redone with anchored `patch` calls.

Those patches had their own problems, all caught immediately by reading the diff:

- The const landed **after** `parse_command`'s doc comment, stealing it. Moved back.
- A patch replacing the `pub fn parse_command(` line **consumed the function signature**,
  orphaning its body. Restored.
- The byte search for `"\n}\n"` **found nothing on a CRLF checkout** — the line-ending trap
  AGENTS.md warns about. The check is now line-based, which is immune to either ending.

Each of those was visible in the tool's own diff output before it reached a commit. The
lesson is not "be careful with Python"; it is that a whole-file rewrite has no anchor to
fail against, whereas a diff you can read is the actual safety mechanism.

## Verification

- `epsilon-tui --lib`: **374 passed**, 0 failed (372 + 2 drift checks).
- The reverse check's own first run is the control, captured above: it failed and named the
  seven heads. No manufactured mutant was needed, and none is claimed.
- The sweep's control is M171's, unchanged: restoring the panicking accessor on one command
  still fails it at `crates/epsilon-core/src/lib.rs:1002`.
- Workspace battery `s294`: FMT=0, CLIPPY=0, TEST=0, DEMO=0; **62 binaries, 1,298 passed,
  0 failed** - +2 for the drift checks.

## Limits

- **The reverse check is a source-shaped assertion**, and says so in its own doc comment. It
  reads quoted tokens from `parse_command`'s body, so a token written in an unexpected shape
  would be missed. It is the price of `Command` not being `Debug` and therefore not
  comparable; the alternative is the second hand-written list this milestone removed.
- **Aliases are not swept directly.** `/timeline` reaches `Command::Timeline` through
  `/branch`. Same arm, same code — but no test types the literal string `/timeline`, so a
  future change that special-cases that spelling would not be caught.
- **The sweep's assertion is still weak on purpose**: "did not panic, and did something
  visible". 31 of 32 heads now meet that bar; none of them is checked for doing the *right*
  thing.
- **`/exit` is filtered out by name.** `quit` and `q` are aliases of it and are therefore
  not swept either, so `Command::Quit` has no execution coverage at all — the one gap the
  design creates, in exchange for not letting the sweep end its own session.
