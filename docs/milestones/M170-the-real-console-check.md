# M170 — the real-console check, and a tool gap it exposed

## What this did

M168 changed nine production call sites and M169 retracted the claim about what they were
protecting against. The one thing neither had was a measurement: nobody had typed the
commands into a real Windows console. This milestone did that, and had to fix the probe
first.

## The probe could not type a slash command

`scripts/console-probe.py --keys` only runs *inside* `if press_enter:`, *after* the turn is
submitted — it exists to answer the permission dialog a `bash` call opens (M154). Asking it
to type a slash command submits an empty line first and types the command afterwards:

```
 • e                          <- only "e" of "/tree" survived, into an open dialog
```

That first attempt produced a photograph of the wrong moment which reads as "the command did
nothing", and I nearly reported it that way. It is the same shape as the M152 elapsed-time
contradiction: a real measurement, of the wrong state.

Added `--submit`: type `--keys` into the composer, *then* press Enter, with no turn in
between. Six lines, and every future command check needs it.

## `/tree` in a real conhost window

```
python scripts/console-probe.py --late --size 100x30 --at 7 --submit --keys "/tree" --settle 3 <exe> ""
```

```
   idle                                   ?/164k · ↑0 ↓0 · R0 W0 CH — · DeepSeek V3.1

   E P S I L O N
   a coding agent in your terminal

     model    DeepSeek V3.1
     context  ?/164k

     ready — ╭ timeline ────────────────────────────────────────────────────────────╮
             │   #4  [user]      how many tokens so far                            │
             │   #5  [assistant] (empty)                                          │
             │   #8  [tool]      epsilon-ready                                    │
             │   #9  [assistant] Run complete.                                     │
             │   #14 [user]      hello from the probe                             │
             │ ▎ #15 [assistant] (empty)                                           │
             │                                                                       │
             │ 16/16 · ↑↓ move · enter jump · esc close                            │
             ╰─────────────────────────────────────────────────────────────────────╯

   • ask anything
```

16 real branch entries, correct kind tags, real snippets, cursor on the last row, **no
crash**. This is the `open_timeline_picker` body, the merged `/tree` and `/timeline` path,
reading a real store through `session_store()`.

## `/export` in a real conhost window

```
 • • exported html → …\Epsilon\.epsilon\exports\session-export_1a0e1e49d550.html (16 entries)
```

The file exists: **7,293 bytes**, contains `<html>`/`<body>`, and contains the actual
transcript strings (`how many tokens so far`, `epsilon-ready`, `Run complete`). A success
notice is not evidence that an export wrote anything; the bytes are.

## A second thing this run confirmed for free

The first `/export` attempt could not start:

```
waiting for this session to be released — 12s elapsed, ~18s left (another process may still be writing it)
```

That is M138's startup lease countdown, rendered in a genuine console window with a real
lease held by a leftover process. It is the first direct evidence of that feature outside a
test, and it also shows the probe leaves the TUI alive long enough to hold the lease — worth
knowing before anyone runs two probes back to back.

## What this settles, and what it does not

**Settles:** the M168 code path works in a real console. `/tree` and `/export` both read a
real store and do their jobs. The M169 retraction stands — there was no reachable crash, and
this run confirms it: the store is there, attached, and used.

**Does not settle:** eight of the nine changed sites are still unexercised in a real
console — `/fork`, `/clone`, `/name`, `/share`, the Enter-to-follow-up key in `keys.rs`,
`switch_session_file`, and both export functions' JSON path. The probe can now reach them
one command per run, and the session lease means runs must be serialised.

## Verification

No code change to the harness. The probe gained `--submit`; the binary is unchanged from
`b6aa2c1`.

- `/tree` and `/export` measured on screen in a genuine `ConsoleWindowClass` window,
  100×30, `buffer-big: false, window: true, buffer-exact: true`.
- Export file verified by reading it back, not by trusting the notice.

## Limits

- **One console, one size, one run each.** CMD via `COMSPEC`, 100×30. PowerShell was not
  exercised, and neither was a resize mid-picker.
- **The session is this repo's own working session**, so the entries shown are prior probe
  runs rather than a curated fixture.
- **No `PATH`-level or window-resize claim** is made; those need `--maximize` and `--early`
  runs that were not done.
- **Sixteen uncovered slash commands remain uncovered.** The probe can now check them, one
  serialised run each, which is roughly 40 minutes of wall clock per batch.
