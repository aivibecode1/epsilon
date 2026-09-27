# A panic no longer takes the terminal with it (D19)

**Date: 2026-09-28. Binary: `D:/eps-target/debug/epsilon.exe`. Measured in a real conhost
at 100×30 under cmd.**

## The defect

`run` armed raw mode and the alternate screen, then returned normally — with the restore
callable *only* on the success path. A panic unwound past both, so a crash left:

- raw mode on — the shell behind it had no echo and no line editing;
- the alternate screen up — the TUI's last frame still on screen;
- the cursor hidden.

The process is gone but the *console* is not. From outside the process, that is
indistinguishable from a program that has stopped responding — which is the shape of the
reported typing-freeze, and impossible to diagnose without already knowing to suspect
raw mode.

## The fix

Three pieces, and each is load-bearing:

1. **`TerminalGuard`** — a `Drop` impl, armed immediately after `try_init` and before
   anything that can panic. `Drop` rather than the error path because `run` returns an
   `i32`: there is no `Err` to propagate, and by the time a panic has unwound, any code
   after the call has been skipped.
2. **A panic hook**, installed after the guard. It calls the *same* `restore` before
   printing, so the message lands on a console that works. A hook that only prints would
   write to a raw-mode alternate screen, which is where a panic message disappears.
3. **One `restore` implementation**, on `TerminalGuard::restore(&mut self)`, reached from
   both the `Drop` and the hook. Two copies of these steps is how they drift and a fix
   lands in one of them.

`EPSILON_TUI_CRASH` triggers a deliberate panic, checked once before the event loop
starts, so the console can be tested without a test harness that breaks its own console.

## What is proven, and what the probe had to learn

The end-to-end check is in `scripts/console-probe.py --crash`: crash the TUI, then type
`echo PROBE_SAYS_CONSOLE_IS_ALIVE` into the console the dead process owned and read the
screen back.

**The first version of that probe asserted nothing.** It checked that the console still
took a command — and that was true *with and without* the fix, because `cmd` re-establishes
echo for its own prompt when it starts. A probe that passes on the broken build is worse
than no probe, because it gets filed as evidence.

What actually differs is whether the TUI left the console as it found it. Without a
restore the alternate screen stays up and the crash message — which the hook can only
print once the terminal is legible — never appears. The verdict is built from that, and
the control confirms it:

| Build | Verdict |
|---|---|
| guard removed (control) | **FAIL** — the crash message never printed |
| with the fix | **PASS** — terminal restored, crash reported, command ran |

The screen after the crash, with the fix:

```
epsilon crashed: deliberate crash: EPSILON_TUI_CRASH is set (probe hook)
the terminal has been restored; this is a bug — please report it.
thread 'main' panicked at crates\epsilon-tui\src\app.rs:1020:9:
Press any key to continue . . .
C:\...\Epsilon>echo PROBE_SAYS_CONSOLE_IS_ALIVE
PROBE_SAYS_CONSOLE_IS_ALIVE
```

### Two probe bugs found on the way, both recorded in the code

- **`cmd /c` exits with its child**, and a console whose last process has gone is
  *closed*. The probe attached to nothing and read an empty screen, producing a FAIL
  that said nothing about raw mode. Fixed with `/k` and `pause`, which is also what a
  user's window actually looks like: the shell is still there, and the shell is the thing
  that has to stay usable.
- **`pause` eats the first keystroke.** `echo` arrived as `cho`, and the probe passed on
  `cmd`'s error message being visible rather than on the command having worked. Fixed by
  clearing the prompt first, and by asserting on the command's *output* rather than on
  the echo of what was typed.

## The unit test, and why it asserts less than it wanted to

`the_terminal_guard_restore_is_reachable_and_idempotent` checks that one `restore` serves
both entry points and is safe to run twice.

The first version of it counted `Drop` calls and asserted both fired. It passed with
`TerminalGuard::drop` emptied out — and then failed *with* the real body, because
`TerminalGuard` is a unit struct and a `Drop` with nothing to do is optimised away. A
test whose mechanism is not the behaviour is decoration; the comment says so.

The observable consequence is pinned by the probe above. The mechanism is pinned by the
unit test. Neither substitutes for the other, and the gap between them is stated rather
than papered over.

**Counts:** 380/380 in `epsilon-tui`, clippy clean.
