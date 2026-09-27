# M176 — driving a picker in a real console, and the key the probe could not send

## The gap

M170 gave the probe `--submit`, which types a command and presses Enter. That reached the
commands whose whole behaviour is a single action — `/tree` opens, `/export` writes. It
could not reach the ones whose behaviour *is* interaction: every picker, every two-stage
selection, every "move through rows and choose one" path. `type_text` sends one
`KEY_EVENT` per character, and an arrow key has no character.

So the commands a user spends real time in were the ones no real-console run could reach,
and the ConPTY test cannot cover them either — it emulates the console, so it cannot say
what a genuine `ConsoleWindowClass` window did with an arrow key.

## The key language

`--keys` and `--submit` now take a sequence: literal text plus `<up> <down> <left>
<right> <enter> <tab> <esc> <home> <end> <pgup> <pgdn> <space> <backspace> <delete>`.
Angle brackets are not something a slash command can contain, so plain text still works
with no quoting rules. `parse_key_sequence` is pure and was checked directly, including
the awkward case:

```
'/tree'                    -> [('text', '/tree')]
'<up><down><enter>/tree'   -> [('key','up'), ('key','down'), ('key','enter'), ('text','/tree')]
'a<up>b'                   -> [('text','a'), ('key','up'), ('text','b')]
'<<up>>'                   -> [('text','<'), ('key','up'), ('text','>')]
```

### A regression I introduced and caught in the same edit

Extending `VK_BY_CHAR` **shadowed the original one-line dict**, which is what maps `"\r"`
to `0x0D` — the mapping M170's `--submit` depends on, and the one whose comment explains
that `"\r"` with a zero VK code is a keystroke no application recognises as Enter. The
merged dict keeps all four control entries and says why they are not part of the printable
repertoire, so the next reader does not "tidy" them away.

## The measurement

A single run is not enough. Both runs' frames showed the cursor on `#15`, which is
consistent with the arrows having done nothing. So the comparison is an A/B on the same
binary, same window, same size:

| keys sent | transcript says |
| --- | --- |
| `/tree<enter>` | `jumped to #15` |
| `/tree<enter><up><up><up><up>` | `jumped to #11` |

15 − 4 = 11. The cursor was on the last of 16 rows and four up-arrows moved it four rows.
**The arrow keys reached the application through a real conhost window and moved the
picker**, and Enter then jumped to that row. That is a user-visible claim with a
measurement behind it, which is the standard M160 set and not something the ConPTY test
can supply.

The picker itself, captured mid-run:

```
╭ timeline ────────────────────────────────────────────╮
│   #8  [tool]      epsilon-ready                       │
│   #9  [assistant] Run complete.                       │
│   #10 [user]      run the check                       │
│ ▎ #15 [assistant] (empty)                             │
│ 16/16 · ↑↓ move · enter jump · esc close              │
╰───────────────────────────────────────────────────────╯
```

## Verification

- Parser unit-checked directly, including `<<up>>`.
- A/B in genuine conhost windows: `jumped to #15` vs `jumped to #11`, `buffer-big: false`,
  `window: true`, `buffer-exact: true`, 100×30.
- Evidence: `audit/evidence/console-probe-m176-picker-no-arrows.json` and
  `console-probe-m176-picker-four-ups.json`.

## Limits

- **One picker, one interaction.** `/tree` → arrows → Enter. `/sessions`, `/models` and
  the two-stage picker are reachable now but were not driven; the lease means runs are
  serialised and each costs about 40 seconds.
- **No ctrl-modified keys.** The language has no `<c-...>` form, so `ctrl+p`, `ctrl+w`,
  `ctrl+t` and the approval dialog's arrow navigation are still unreachable from a real
  console. That is the next obvious gap and it is not closed.
- **Arrow keys are sent as bare virtual-key codes** with no scan codes and no
  `dwControlKeyState`, which is right for unmodified arrows and wrong for shifted ones.
- **The session is this repository's own working session**, so the rows are prior probe
  runs rather than a curated fixture, and the entry count changes between runs.
- **No PowerShell.** Every run is `cmd` via `COMSPEC`; the user's original report named
  both shells and PowerShell is still unverified by a real console.
