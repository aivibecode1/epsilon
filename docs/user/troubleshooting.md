# Troubleshooting

For users distinguishing setup failures from known runtime defects.

## First stop: `epsilon doctor`

```sh
epsilon doctor
```

Reports model resolution, credentials (with the exact command that fixes a missing one),
sandbox backend, session store, and project trust — without starting a run.

## Common cases

| Symptom | Likely cause / fix |
|---|---|
| `no model` or provider errors | no credential: `epsilon auth login <provider>` or set `<PROVIDER>_API_KEY`; `epsilon --list-models` to pick an id |
| "unknown provider" from `--provider real` | built without real providers — install default features (see [`install.md`](install.md)) |
| Run hangs, no output for minutes | stream stalled: the watchdog fails the turn after 120 s of silence; tune with `EPSILON_STREAM_IDLE_SECS` (`0` disables) |
| Rate limit / 5xx | retried automatically with backoff (the wait is shown); add `--fallback <model>` for a retryable-failure chain |
| Command refuses to run | sandbox fail-closed: no confinement backend on this platform (macOS has none); `--sandbox` modes in [`permissions.md`](permissions.md) |
| Permission denied unexpectedly | matching denies override later allows — check project `.epsilon/config.toml` and trust state (`/trust`) |
| Startup waits on the session lease | another window holds the session: a countdown is drawn (`waiting for this session to be released — Ns elapsed`), and if that holder is **alive** the wait ends in a refusal naming it — close that instance; **do not** delete a live holder's lock. If the holder is **gone** (terminal closed, process killed) the lock is taken after about **six** seconds, and locks untouched for thirty seconds are reaped at launch. Details and the two timings: [`sessions.md`](sessions.md) |
| Session file "damaged" / "newer format" | typed refusal, not corruption: see [`sessions.md`](sessions.md) for the limits of refusal and repair |
| OAuth login fails to open a browser | use `--paste`; client-id notes in [`auth.md`](auth.md) |
| Diagnostics nowhere | headless modes print replies to stdout and diagnostics to stderr; `RUST_LOG` does nothing — no crate here installs a tracing subscriber, so setting it changes no output |
| Boxes or `?` where the UI draws characters | the console cannot render the Unicode set: run `EPSILON_ASCII=1 epsilon` for the seven-bit set (`+-|`, `x`, `>`), or `EPSILON_ASCII=0 epsilon` if your console *does* have a Unicode font and you want the rounded look. Since M219 the default is ASCII on a classic Windows console and Unicode in Windows Terminal, a VS Code terminal, or any VT-capable `$TERM`, so this only applies when the guess is wrong — or on Windows where you want to *opt back in* to Unicode with `0`. **The variable covers the UI's own characters — the rules, the cursor, the ticks — and nothing else.** A CJK filename, an emoji in tool output, or a model reply in another script is *content*, and it is shown as the tool produced it: `EPSILON_ASCII=1` does not touch it, because rewriting content to fit a font would be showing the reader something the tool did not say. On a console that cannot render the script, install a font that can (Windows Terminal, or the newer console hosts) — that is the fix for content, where the variable is the fix for chrome |

## Still stuck

- `epsilon --help` — the shipped binary is the full reference.
- Check [`../../audit/03-defects.md`](../../audit/03-defects.md) — your symptom
  may be a known open defect, not your setup.
