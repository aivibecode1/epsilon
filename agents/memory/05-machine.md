# This machine — build, disk, verification

Durable notes about the *environment*, not about the project. Re-read before any long build.

## Disk

**Build on `C:`, in the repo's own `target/`. Never on `D:` — owner's rule, 2026-09-28.**

| Drive | Use |
|---|---|
| `C:` | the repo, `~/.cargo`, the antivirus, the user, and **all build artifacts** |
| `D:` | **nothing. Do not build there. Do not write build output there. Leave it alone.** |

A full workspace build produces tens of GB: in-repo `target/` has historically grown past
35 GB and pushed `C:` to 96% full. So:

```sh
export CARGO_BUILD_JOBS=1     # NOT superstition: parallel jobs fail here with 0xc0000142
df -h /c                       # watch C:, it is the only drive that matters now
```

**Do not set `CARGO_TARGET_DIR`, and do not add a `[build] target-dir` to any config.**
Builds go where cargo puts them by default: `C:/Users/Hp/Desktop/My Agent Space/Epsilon/target`.

`D:/eps-target` held 21 GB of build output and **has been deleted**. It is not coming back.

## How this went wrong twice, and what to actually check

**1. I inherited the convention and never verified it.** `01-plan-execution.md` said builds
go to `D:/eps-target`. My first command of the session was
`export CARGO_TARGET_DIR=D:/eps-target && cargo check ...`, and because this shell keeps
state, every later command inherited it. I never asked *why* — and when the owner asked
"how eps-target goes to D: ?", the honest answer was: **I typed it into my own shell.**
It was not in `~/.cargo/config.toml`, not in the repo, not in the Windows environment, and
not in `scripts/gate.ps1`.

**2. Then I nearly deleted it.** `AGENTS.md` said "clear the cache after every test/build
round", and I reached for that rule — which would have destroyed the only thing keeping C:
free. The owner said *"leave D:"*.

**The real sequence, which is the part worth keeping:** the owner stopped me from wiping
D:, I "corrected" the docs to say *never delete D:*, and the actual answer turned out to be
neither — **D: should never have been used at all.** Two rounds of confidently following
and then confidently un-following a rule, because nobody checked where it came from.

**What to check before acting on a location, a path, or a convention in a memory note:**
where does it actually come from? `reg query "HKCU\Environment"`, `cat ~/.cargo/config.toml`,
`env | grep CARGO`. Thirty seconds. A note is a *claim about* the machine; the machine is
the fact.

**Cleanup, correctly this time:** `rm -rf target` at the end of each round, in the repo.
Copy anything worth keeping (gate logs) out of `target/` first, or the wipe takes it.

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
