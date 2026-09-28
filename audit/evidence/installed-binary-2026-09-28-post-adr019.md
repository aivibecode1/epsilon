# Installed release verification — 2026-09-28 (post-ADR-019)

Covers `C:/Users/Hp/.cargo/bin/epsilon.exe` as installed by
`cargo install --path crates/epsilon-cli --features real --force` after `6033fd2`.

- **Installed:** Sep 28 05:27, 20,933,120 bytes
- **Install:** release profile, `Finished in 15m 09s`, `Replacing C:\Users\Hp\.cargo\bin\epsilon.exe`
- **Closes:** D22 in [`../03-defects.md`](../03-defects.md) — the installed binary had
  predated ADR-019 (`e9d5b94` / `ece3af4` / `9f73a5f`)

The previous verification of this file was at `5632db2` and covered the session/TUI work
only. Nothing below restates it.

## Why markers, not timestamps

A timestamp says when a file was written; a marker says what is in it. This session has had
two builds finish after their source was edited, one of which was then mistaken for current.
The `5632db2` release was verified by `strings` for exactly that reason, and the same method
is used here.

## Markers in the installed binary

| Marker | Meaning | Found |
|---|---|---|
| `interpose_result` | ADR-019 host→child answer frame | **yes** |
| `before_tool_call` | the pi-shaped interpose event tag | **yes** |
| `blocked by extension policy` | the veto message built by `ExtensionToolHooks` | **yes** |
| `may intercept tool calls` | the startup line `main.rs` prints when hooks are installed | **yes** |
| `EPSILON_TUI_CRASH` | the deliberate-panic probe hook (`tui/src/app.rs:1023`) | **yes** |
| `stream-pace-ms` | `PacedProvider` selection | **yes** |

`"the terminal was restored"` is **absent, correctly**: that string is the console-probe
script's own verdict text (`scripts/console-probe.py:653`), not a string in the binary. It was
checked as a marker by mistake and is recorded here so the next reader does not repeat the
check and conclude a regression.

## Probe 1 — typing during a genuinely in-flight stream

```sh
python scripts/console-probe.py --late --size 100x30 --enter --settle 8 --during \
  --keys "typed while streaming" \
  --exe-args "--script audit/evidence/long-turn-script.json --stream-pace-ms 30" \
  C:/Users/Hp/.cargo/bin/epsilon.exe
```

The input box at the moment of capture:

```
• typed while streamingtyped while streaming
```

**40 characters** present in the editor, typed while the provider was mid-stream, and the
footer reads `insert · enter send · pageup scroll back · pagedown scroll forward`.

This is the property the owner's original report was about: typing during a run must show up
immediately rather than the UI appearing frozen. The stream is genuinely paced
(`PacedProvider`, 30 ms per event) rather than a `FauxProvider` replaying a queue, which is
what makes it a real in-flight measurement.

## Probe 2 — terminal restoration after a panic

```sh
python scripts/console-probe.py --late --crash --at 7 --size 100x30 \
  C:/Users/Hp/.cargo/bin/epsilon.exe
```

```
"crash_reported": true,
"verdict": "PASS: the terminal was restored and the crash was reported",
```

**Note on invocation:** `--exe-args "--crash"` is *wrong* — the probe sets
`EPSILON_TUI_CRASH` in the child environment itself (`scripts/console-probe.py:555`), and
passing `--crash` as an argument makes the CLI print `unknown argument: --crash` and exit
normally. The result is then a **false PASS**: the console is restored and responsive because
the program never crashed. The tell is `"crash_reported": false` alongside a
`"verdict"` of FAIL, but the screen is otherwise healthy.

This is the third time this session that a probe measured the wrong thing and reported
success — the earlier two were the reducer tests that passed against a broken bridge. A
control that passes because the thing under test never ran is the same failure as a test
that never fails.

## What this does not verify

- **Extension interception has not been exercised through the installed binary.** The e2e
  tests drive `ext-fixture` from the *test* build. The markers prove the code is linked in;
  nothing here proves a real third-party extension can veto a tool call in the shipped
  binary, because no real extension exists yet.
- **No marker or probe covers the interpose timeout change** (`INTERPOSE_TIMEOUT_SECS = 1`).
  It is a constant, not a string, so it cannot be probed this way; the unit test that found
  it is the only evidence.
- **The `--crash` false-PASS above** means the crash path was verified only by the corrected
  invocation. Both were run; only the second had the panic actually occur.

## Updated 06:22 — both arms of ADR-019

Reinstalled after `2fb5219` (15m 06s release build, 20,957,696 bytes). The before-arm-only
build this file first described is superseded.

| Marker | Meaning | Found |
|---|---|---|
| `rewritten_by_extension` | the note `after_tool_call` writes — **after-arm only** | **yes** |
| `tool_result` | the pi-shaped result event tag | **yes** |

The other five markers from the 05:27 build are still present. `rewritten_by_extension` is
the discriminating one: it exists in no other string in the tree, so its presence is proof
that this is not the earlier binary under a new timestamp.

Both probes re-run against this build:

| Probe | Result |
|---|---|
| crash / terminal restoration | `crash_reported: true` — **PASS** |
| typing during a paced stream | `• typed while streaming` in the editor, footer live |

Invocation note from the first version of this file still applies and was re-confirmed: the
crash probe sets `EPSILON_TUI_CRASH` itself, and `--exe-args "--crash"` yields a false PASS.
