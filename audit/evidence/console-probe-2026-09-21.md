# What the TUI does to a real Windows console

**Measured:** 2026-09-21, on the owner's machine, against
`target/debug/epsilon.exe` built from `6f3d544` (HEAD) plus the uncommitted
session-store fix. Host: legacy **conhost** (`ConsoleWindowClass`, not Windows
Terminal) in every run, which is the console the owner reported the problem in.

**Why a second harness.** `crates/epsilon-cli/tests/terminal.rs` drives the binary
through a ConPTY, and under ConPTY the console is emulated *inside the terminal
application* — so no test written against it can observe the console window being
resized, which is exactly what "the terminal shrinks" describes. This probe creates
a real console window instead, attaches to it, and asks Windows what happened to it.

**The probe:** [`scripts/console-probe.py`](../../scripts/console-probe.py). It
detaches from the harness console, opens a fresh one by running `cmd`, attaches,
records the window and buffer geometry, lets the shell launch the TUI *in that same
console*, records again, then pushes real `KEY_EVENT` records into the console input
buffer and reads the screen back.

> One trap, recorded because it cost a run: the TUI must be launched *by the shell in
> that console*. Spawning it as a child of the probe hands it the probe's stdout —
> a pipe — and the program faithfully renders its frame into the pipe. The first
> version of this probe did that, and the screen read back a bare `cmd` prompt while
> the frame appeared in the JSON.

## The measurements

| Console | Before: buffer / window | After the TUI starts | Typed text on screen |
|---|---|---|---|
| default (166 cols) | 166×9001 / 166×43 | 166×43 / **166×43** | cursor advanced 20 cells |
| forced 100×30 | 100×30 / 100×30 | 100×30 / **100×30** | composer shows it |
| 100×20 over a 9001-row buffer | 100×9001 / 100×20 | 100×20 / **100×20** | composer shows it |
| maximized (168×44) | 168×9001 / 168×44 | 168×44 / **168×44** | composer shows it |

**The window rectangle never changes.** In all four configurations the visible
window is the same size after the TUI is running as it was before, including the
maximized case, which is the setup where a buffer switch would be most visible.

What *does* change is the height of the **active screen buffer**, which drops to the
window height. That is not a defect: it is what an alternate screen *is* — a second
buffer with no scrollback, so the application cannot scribble into the user's
history. The primary buffer keeps its 9001 rows and is switched back to on exit.

**Typing and rendering work in every configuration that started.** In the late-typing
runs the composer contains the exact text pushed into the console input buffer, and
the frame around it is intact:

```
idle                                    ?/1.0M · ↑0 ↓0 · cache — · $0 · DeepSeek V4.1 Flash

E P S I L O N
  type      describe a task and press enter
  ...
────────────────────────────────────────────────────────────────────────────────
• maximized window
────────────────────────────────────────────────────────────────────────────────
enter send · ctrl-j newline · ctrl-p palette · ? help     …\My Agent Space\Epsilon · main
```

The header row is the visibility feature doing its job in a real console: context
(`?/1.0M`), session tokens, cache hit rate and cost, then the model.

## The one failure that did reproduce: a killed instance locks the next launch out

The 80×25 early-typing run never started the TUI. It printed this and **exited**:

```
open session store: another epsilon is already writing this session (pid=9944) —
one writer per session file; close it, or remove the .lock file if it is a crash
leftover (a live holder re-stamps it every few seconds)
```

`pid=9944` was the TUI from the *previous* probe run, force-killed seconds earlier.
The lease (`crates/epsilon-session/src/lease.rs`) decides a holder is dead only when
its lock file is older than `STALE_AFTER` — **30 seconds**, against a 10-second
heartbeat. So:

**Reproduction.** Launch the TUI, kill it without letting it exit (close the console
window, End Task, `taskkill`), and relaunch within 30 seconds. The second launch
refuses to start and exits. Do it twice in a row and it looks like the program
simply does not run.

That is a real usability defect, and it is a different one from the reported freeze:
the process exits rather than hanging, and the message does name the pid and the
lock file. It is recorded here rather than fixed because the fix is a design choice —
checking whether the recorded pid is still alive needs a platform call, and this
workspace forbids `unsafe` outside a reviewed per-crate override. Two shapes are
worth reviewing: a narrowly scoped liveness probe, or a refusal that waits out the
remaining staleness window with a countdown instead of exiting.

## What this does and does not establish

- **Established:** on this machine, at this commit, in a real conhost window, the TUI
  starts, sizes itself to the window, renders its chrome (including the visibility
  row), accepts typed input, and does not resize the user's window — in four
  geometries including a maximized window over a deep scrollback.
- **Not established:** that the owner's original report was wrong. It was two real
  defects, both since fixed and both measured at the time: the run loop redrew a full
  frame per keystroke and per streamed delta (`M94`, a 99-character burst echoing at
  ~30 characters/second before the fix), and every Windows `bash` call loaded the
  user's PowerShell profile, 6.2 s for a one-word `echo` (`M98`), which from inside
  the TUI is indistinguishable from a deadlock.
- **Not covered:** Linux and macOS consoles, a console *smaller* than the TUI's
  44×10 floor, and whether the primary buffer's scrollback is intact after exit.
  Those are open questions, not claims.

## Re-running it

```sh
cargo build -p epsilon-cli
python scripts/console-probe.py --late --size 100x30 target/debug/epsilon.exe "text"
python scripts/console-probe.py --late --maximize target/debug/epsilon.exe "text"
```

`--early` types while the program is still initialising, `--size CxR` (or
`CxRxBUFFER`) forces the geometry, `--maximize` maximizes the window. Output is one
JSON object on stdout; the four runs behind this note are kept verbatim in
[`console-probe-runs-2026-09-21.json`](console-probe-runs-2026-09-21.json), because
`target/` is disposable on this machine and evidence that only exists there is
evidence that has already been deleted.

## And the binary the owner actually runs

The measurements above are against `target/debug/epsilon.exe`. The owner runs the
**installed** `%USERPROFILE%\.cargo\bin\epsilon.exe`, so the same probe was run
against it after the round's reinstall — 100×30, real conhost:

```
host=ConsoleWindowClass  before win=100x30 buf=100x30
                         after  win=100x30 buf=100x30
• installed binary
enter send · ctrl-j newline · ctrl-p palette · ? help   …\My Agent Space\Epsilon · main
```

Same result: no window change, frame on screen, typed text in the composer. Raw run:
[`installed-binary-console-probe-2026-09-21.json`](installed-binary-console-probe-2026-09-21.json).
