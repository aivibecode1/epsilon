# Installed binary verification — 2026-09-28, after M185

The installed release is what the user actually runs, so it is verified separately from the
gates. The gates prove the source; this file proves the bytes on disk.

- **Binary:** `C:/Users/Hp/.cargo/bin/epsilon.exe`
- **Size:** 21,016,064 bytes
- **Built by:** `cargo install --path crates/epsilon-cli --features real --force`
- **Build time:** 10m 49s release
- **Installed from source commit:** `d3dbb4f` (docs-only since `c6eee08`, the last source change)

Verified against the previously-installed 06:22 build, which had the tool hooks only.

## Content markers

Checked with `strings`, not timestamps — a timestamp proves when a file was written, not what
is in it.

| Marker | Count | Meaning |
|---|---|---|
| `prompt templates:` | 1 | `/templates` listing (M185) |
| `.epsilon/prompts/` | 1 | project template discovery (M184) |
| `interpose` | 1 | ADR-019 bidirectional transport (M181) |
| `turn_end` | 17 | `ExtensionTurnHooks` (M183) |
| `EPSILON_TUI_CRASH` | 3 | panic hook and probe path |
| `ZZPROBE_MARKER` | **0** | correctly absent — that string lived only in a deleted probe file |

The last row is the one worth noting: a marker being *absent* is evidence too, and it is what
distinguishes "the binary predates the probe" from "the probe file was cleaned up".

## Live probes, real conhost windows

### 1. Template expansion — the M184/M185 feature, end to end

`/zzprobe auth-tokens` submitted into a real console:

```
❮ ❯ ZZPROBE_MARKER reviewed auth-tokens for correctness.
```

`ZZPROBE_MARKER` exists only in the template body and `auth-tokens` came from real keystrokes,
so the screen proves **expansion**, not echoing. The probe template
(`.epsilon/prompts/zzprobe.md`) was removed afterwards.

### 2. Typing during a genuinely streaming turn — unchanged

`--script audit/evidence/long-turn-script.json --stream-pace-ms 30`, typed during the stream:

```
• typed while streaming
❮ ❯ typed while streaming
```

Characters accepted mid-turn in the installed release, which is the original reported symptom
and the thing the whole TUI work existed to fix.

### 3. Terminal restoration after a panic — unchanged

`--crash`:

```
epsilon crashed: deliberate crash: EPSILON_TUI_CRASH is set (probe hook)
PASS: the terminal was restored and the crash was reported
```

`crash_reported: true`, `console_usable: true`.

## What is not verified

- **No real provider was used.** Every probe ran against the faux/paced providers, so nothing
  here says a paid API call works from the installed binary.
- **No third-party extension was loaded.** The extension markers are confirmed present in the
  binary, but no probe drove a live extension host from the installed exe.
- **Turn hooks were not exercised through the installed binary.** The marker is present and the
  bridge is covered by live-child tests in-tree; a real extension stopping a real run through
  the installed exe is not shown here.
- **One probe run was invalid and is not counted.** An earlier crash attempt passed
  `--script audit/evidence/crash-script.json`, a file that does not exist. The failure was in
  the invocation, not the binary. Recorded because an invalid probe is the same shape of
  mistake as the `m183` stale-snapshot failure: an absent result is not evidence until the
  instrument is checked.
