# M177 — the shell the owner named, measured for the first time

## The gap, stated plainly

The owner's report, quoted in this project's own brief: the TUI *"freezes and shows no
input or output the moment you start typing, in both CMD and PowerShell, even though
tokens are clearly being consumed underneath."*

Every real-console measurement in this project — M154, M170, M176 — was taken under `cmd`,
because `console-probe.py` hardcoded `cmd /c "ping …"`. **PowerShell was named in the
report and had never been opened once.** Two milestones in a row ended by listing that as an
unclosed limit. This closes it.

## The change

`--shell {cmd,powershell}`. Default stays `cmd`, so every prior invocation is unchanged.

The sleep that keeps the shell alive while the probe attaches is `Start-Sleep -Seconds 6`
under PowerShell and `ping -n 6 127.0.0.1 >nul` under cmd. Both were chosen for one
property, not for being idiomatic: **neither reads stdin**, so nothing competes with the
TUI for keys. The comment says so, because the reason is invisible otherwise.

## What PowerShell does, measured

### 1. A picker, with the same arithmetic as M176

`/tree<enter><up><up>` under PowerShell: **`jumped to #13`**. M176 measured `15 − 4 = 11`
under cmd; here `15 − 2 = 13`. The cursor started on the last of sixteen rows in both
shells, and the arrow keys moved it the number of rows sent. The interactive path behaves
identically in the shell the report named.

### 2. A live turn — the freeze, directly

```
⠋ running 5s · streaming…    ctx <1%/164k · ↑100 ↓50 · R0 W0 CH — · $0.0001 · DeepSeek V3.1
▎ ❯ how many tokens so far
▸ ▸ bash $ echo epsilon-ready · pending
▸ echo epsilon-ready
╭ permission required ────────────────────────────────────╮
│ cap  bash                                                 │
│ what echo epsilon-ready                                   │
│ y allow once   a always (session)                         │
│ n deny         e deny + guidance                          │
╰───────────────────────────────────────────────────────────╯
approval · y allow once · a always · n deny · e deny + guidance
```

This is the picture the report said did not exist. The composer shows what was typed, the
turn is running, the spinner and elapsed seconds advance, the token counters move, the cost
is shown, the tool call is displayed **with its argument**, and the approval dialog is
drawn *inside* the layout rather than over it.

### 3. The whole loop, answered

`y` — sent through M176's key language — approves, and the turn finishes:

```
idle                ctx <1%/164k · ↑200 ↓100 · R0 W0 CH — · $0.0001 · DeepSeek V3.1
▎ ❯ how many tokens so far
✓ ✓ bash $ echo epsilon-ready · ok · 6455ms
✓ epsilon-ready
• • permission allowed
• Run complete.
```

Each of those rows is a prior milestone showing up in a real console for the first time:
one card per tool call (M155, settled in place), a real `6455ms` duration (M158), token
counters accumulating across turns `↑100 ↓50` → `↑200 ↓100` (M140), cache honestly unknown
rather than fabricated as zero (`R0 W0 CH —`, M151), and a cost estimate (M151).

## Verification

Three runs, all in `ConsoleWindowClass` windows at 100×30 with
`buffer-big: false, window: true, buffer-exact: true`, shell reported as `powershell`:

- `audit/evidence/console-probe-m177-powershell-picker.json`
- `audit/evidence/console-probe-m177-powershell-running.json`
- `audit/evidence/console-probe-m177-powershell-turn-complete.json`

Battery `s178`: FMT=0, CLIPPY=0, TEST=0, DEMO=0; **62 binaries, 1,298 passed, 0 failed**.

## Limits

- **`powershell.exe` 5.1 from `PATH`, with `-NoProfile -NoLogo`.** A user's profile that
  changes the console (VT mode, a different `$Host`) is not covered, and `-NoProfile` is
  exactly the un-realistic part. `pwsh` 7 was not run; `PSEXE` overrides the binary.
- **Three runs, one window size, one workflow.** No resize, no maximise, no narrow
  terminal, no long-running tool, no `/export` or `/fork` under PowerShell.
- **The faux provider.** No real API was called, so nothing here says anything about real
  token accounting, cache hit rate, or cost accuracy.
- **`WriteConsoleInputW` still reports `err: 87`** on some calls. It is benign — the keys
  land, and the A/B and the `y` approval both prove that — but the probe records the error
  rather than hiding it, and that remains unexplained by the API documentation.
- **The ConPTY terminal test still runs under cmd.** It is not shell-parameterised, so
  "the TUI works in PowerShell" rests on the probe alone, not on the test suite.
