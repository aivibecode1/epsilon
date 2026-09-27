# The typing-freeze, measured: it is not reproducible

**Date: 2026-09-28. Binaries: `D:/eps-target/debug/epsilon.exe` (fresh debug build) and
`C:/Users/Hp/.cargo/bin/epsilon.exe` (the installed release).**

## The claim being tested

The reported defect, in the owner's words: the TUI *"freezes and shows no input or
output the moment you start typing, in both CMD and PowerShell, even though tokens are
clearly being consumed underneath."*

Three things are asserted at once: typing produces **no input**, the screen shows **no
output**, and the work underneath **continues**.

## Why the previous two audits could not test it

This is the part worth keeping, because it is a property of the repository rather than a
mistake by whoever came before.

`FauxProvider` emits an entire turn's events with no delay. `FauxEventStream::next` is
`self.pending.pop_front()` — there is no timer anywhere in the path. A turn is therefore
complete in microseconds, and **every** harness that has ever driven this TUI has
photographed a run that was already over. The `--tui-bench` path is worse still: it
calls `draw_frame` directly with a `bench_app` that holds no `Agent` at all.

`SlowProvider` does not close the gap. It sleeps *before* `stream()` opens, so it can
delay the **start** of a turn. It cannot delay the **duration** of one, because a turn's
length is decided by how many events the stream yields and how fast the consumer pulls
them. Nothing wired it to the TUI in any case.

So the reported state — a turn in flight for seconds while the user types — had never
been executed by anything. The earlier audit's claim that a permission dialog silently
swallowed keystrokes was a *reasonable inference from the symptom* and a wrong answer:
it was inferred from a screen that had never been captured in the state being described.

## What was built to test it

`PacedProvider` (`crates/epsilon-llm/src/paced.rs`), exposed as `--stream-pace-ms N`,
sleeps N ms *between* stream events. The first event is unpaced, because a real provider
opens promptly and then trickles.

Measured on a ~700-character script (≈100 deltas at 8 characters each), in print mode:

| Pace | Wall clock |
|---|---|
| `--stream-pace-ms 0` | **125 ms** |
| `--stream-pace-ms 30` | **3875 ms** |

`scripts/console-probe.py` gained `--exe-args` (pass arguments to the TUI) and `--during`
(type a second time while the turn is still streaming, and read the screen mid-flight).

## The result: typing during a live turn works, in both shells

`--shell cmd`, 100×30, typing while the header reads `running 1s · streaming…`:

```
 | ⠙ running 1s · streaming…              ctx <1%/164k · ↑0 ↓0 · R0 W0 CH — · DeepSeek V3.1
 | ▎ ❯ explain the freeze
 | • I am going to take my time with this answer so that the turn lasts long enough for a human to star
 | • typing into the composer while it is still in flight. That is the entire point of this script: it
 | • exists to hold the stream open, not to say anything interesting. A real provider streams a
 | • paragraph every few hundred mil
```

0.8 s later, in the same turn (`running 2s · streaming…`):

```
 | • typed while streaming
```

42 characters written, the cursor advanced to column 23, and the assistant text kept
growing. `--shell powershell`, same run shape:

```
running    -> ⠇ running 2s · streaming…   ctx <1%
typed text -> • typed while streaming
```

Earlier measurements, on both binaries and both shells: typing at rest, typing during
startup, submitting, answering a permission prompt with `y`, and reading the header after
`Run complete.` all worked.

## What this does and does not establish

**Established:** in the state the report describes — a turn genuinely streaming while
the user types — the TUI renders, accepts input, shows the typed text, and keeps
streaming. On this machine, with these binaries, at 100×30, under cmd and PowerShell.

**Not established:** that the owner has not seen it. The two can both be true, and the
gap between them has two candidates, neither of which this repository can currently
test:

1. **A stale binary.** The installed release is what the owner runs. It was probed and
   behaves identically to the fresh debug build, which argues against this but does not
   cover a build from before that.
2. **Something outside the measured configuration.** Terminal host, window size, console
   mode, scrollback, or a real provider rather than the paced faux one.

**The honest claim is therefore narrow:** the freeze is not reproducible under the
conditions that have been measured, and the repository previously could not have
measured them. Phase 1 of [`../07-plan.md`](../07-plan.md) removed the reason for that.
The next useful step is a probe on the owner's own terminal, with the real provider, at
the size and shell they actually use.

## What remains worth fixing regardless

The freeze is not the defect this audit found. The others are measured and are in
[`../03-defects.md`](../03-defects.md): the per-frame transcript clone (D18), a panic
that leaves the terminal unusable (D19), and a killed TUI blocking the next launch for
30 s (D20). D20 in particular produces *exactly* the reported symptom — a screen that
accepts no input, with the process still running underneath — and is confirmed by
hitting it on every probe run.
