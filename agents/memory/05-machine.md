# This machine — build, disk, verification

Durable notes about the *environment*, not about the project. Re-read before any long build.

## Disk

| Drive | Free (2026-09-27) | Use |
|---|---|---|
| `C:` | ~36.7 GB | repo, `~/.cargo`, the antivirus, the user |
| `D:` | ~59.7 GB | **build target dir** — use this, not `C:` |

A full workspace build produces tens of GB. In-repo `target/` has historically grown past 35 GB and
pushed `C:` to 96% full. **Always** set:

```powershell
$env:CARGO_TARGET_DIR = "D:\eps-target"
$env:CARGO_BUILD_JOBS = "1"
```

`CARGO_BUILD_JOBS=1` is not superstition here: every gate failure at parallel jobs on this box has
been `0xc0000142` (compiler startup contention), not a code error.

## Clean up after every round

```powershell
Remove-Item -Recurse -Force D:\eps-target -ErrorAction SilentlyContinue
```

Do not clean while a build or test is running. Do not purge the shared `~/.cargo` registry or user
runtime data — only this repo's disposable artifacts.

## Build is the long pole

A cold `cargo build --workspace --all-targets` with one job runs roughly 30–45 minutes; the C
dependency tree (reqwest/rustls/axum/ratatui) dominates. Start it in the background and do
read-only study while it runs. One build owner at a time — never run a build concurrently with a
gate round or a `cargo install`.

## Antivira

Avira on this box has flagged freshly linked test executables (`HEUR/APC.Misc`) and blocked the
write with `LNK1104` / "Access is denied" on paths that do not exist. It binds to the exact output
path. Do not change security configuration from an agent session. Workaround if it recurs: rebuild
the test profile under fresh metadata (`CARGO_PROFILE_TEST_DEBUG=1` changes every test binary's
name) and disclose the knob.

## The PowerShell trap

`bash` on this machine is WSL: different `$HOME`, different `PATH`, no Windows `cargo`. It exits
127 in zero seconds and the log still says `GATE FAIL`, which reads like five failed gates instead
of one missing PATH entry. **Run `pwsh -File scripts/gate.ps1 <label>` here.** `scripts/gate.sh` is
for POSIX and CI.

## Every battery attempt gets its own label

`target/gate-<label>.log` is truncated on start, so re-using a label destroys the failed attempt's
evidence. Use a fresh label per attempt.
