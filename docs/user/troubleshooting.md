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
| "session is held by another process" | the write lease names the holder; close that instance; do not manually remove a live holder's lock |
| Session file "damaged" / "newer format" | typed refusal, not corruption: see [`sessions.md`](sessions.md) for the limits of refusal and repair |
| OAuth login fails to open a browser | use `--paste`; client-id notes in [`auth.md`](auth.md) |
| Diagnostics nowhere | headless modes print replies to stdout and diagnostics to stderr; `RUST_LOG` does nothing — no crate here installs a tracing subscriber, so setting it changes no output |

## Still stuck

- `epsilon --help` — the shipped binary is the full reference.
- Check [`../../audit/03-defects.md`](../../audit/03-defects.md) — your symptom
  may be a known open defect, not your setup.
