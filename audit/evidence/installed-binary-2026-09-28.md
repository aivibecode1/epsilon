# The installed binary carries this session's fixes

**Date: 2026-09-28. Binary: `C:/Users/Hp/.cargo/bin/epsilon.exe` (release, `cargo install
--path crates/epsilon-cli --features real --force`, 10m 23s, replacing the pre-session
build).**

Per AGENTS.md, the global binary is reinstalled whenever CLI or TUI behaviour changes,
because the owner tests the *installed* one. It is therefore the binary that matters, and a
fix verified only in `target/debug/` has not been verified in the thing that runs.

## Presence check

Each string is a marker that can only exist if the corresponding change is in the shipped
binary. `strings` on the installed executable:

| Marker | Count | From |
|---|---:|---|
| `messages on this branch` | 1 | `/tree` re-projection, `7489c00` |
| `epsilon crashed` | 1 | the D19 panic hook, `a4e9fa2` |
| `stream-pace-ms` | 2 | `PacedProvider`, `f318bc5` |
| `not reported` | 2 | `UNKNOWN_BILLED_MODEL`, D17, `12ae518` |

The first check this replaces was the useful one. An earlier release build in this session
— completed at 03:11 — turned out to **predate** the `/tree` change, because cargo had
already snapshotted the sources before the edit landed. A timestamp looked like proof and
was not; looking for the string in the binary is proof. The install was re-run from the
final tree for the same reason.

## Probe 1 — typing during a live turn, on the release build

```
python scripts/console-probe.py --late --size 100x30 --enter --settle 8 --during \
  --exe-args "--script audit/evidence/long-turn-script.json --stream-pace-ms 30" \
  C:/Users/Hp/.cargo/bin/epsilon.exe "final check"
```

```
⠏ running 2s · streaming…                          ctx <1%/164k · ↑0 ↓0 …
• typed while streaming
```

The turn is genuinely in flight, the typed text is on screen, and this is the binary the
owner runs rather than a build in `target/`.

## Probe 2 — a panic restores the terminal, on the release build

```
python scripts/console-probe.py --late --crash --at 7 --size 100x30 \
  C:/Users/Hp/.cargo/bin/epsilon.exe x
```

```
PASS: the terminal was restored and the crash was reported
```

This is the same check whose control was run earlier in the session and correctly
**FAILED** with the guard removed — see
[`tui-panic-restore-2026-09-28.md`](tui-panic-restore-2026-09-28.md). A probe that has only
ever been seen passing proves nothing; this one has been seen failing.

## What this does not establish

The reported freeze still does not reproduce, and that claim is now measured on both a
debug and a release build, under cmd and PowerShell. It remains a statement about this
machine, these binaries, 100×30, and the paced faux provider. It is **not** a claim that the
owner has not seen it — the two untestable candidates from
[`tui-typing-during-stream-2026-09-28.md`](tui-typing-during-stream-2026-09-28.md) are
unchanged: a terminal or provider configuration nobody here has reproduced, and the history
of reports that predate every build in this session.
