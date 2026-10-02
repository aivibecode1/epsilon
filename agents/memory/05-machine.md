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

**Do not add a `[build] target-dir` to any `cargo.toml`.** That is the standing rule and it
is right: the setting is per-machine and belongs to the gate script, not the project.

**But `scripts/gate.ps1` points `CARGO_TARGET_DIR` at `%TEMP%\epsilon-target`, and so
should your own `cargo` calls.** This note used to say the opposite — "builds go where
cargo puts them by default", into the repo — and it was **wrong**: writing build output
*inside this repository* makes `cl.exe` fail with
`Command line error D8050 : cannot execute c1.dll`, which cargo reports as a failed custom
build command for some C dependency, naming the crate rather than the cause. It is specific
to this directory: a sibling repo, a longer path, and a path with more spaces all build
fine. Seven controlled runs are in [`build-blockers-resolved.md`](build-blockers-resolved.md).

The note was wrong for weeks while being indexed as live, because nobody re-read it against
the script it claims to describe — which is the lesson this very file teaches two sections
below. **A note is a claim about the machine; the machine is the fact.**

**The disk knob that actually matters: turn debug info off.**

```sh
export CARGO_PROFILE_DEV_DEBUG=0     # measured 2026-09-29 (M208): a full seven-gate
export CARGO_PROFILE_TEST_DEBUG=0    # battery goes from ~33 GB to 4.19 GB
```

Measured again this round, without the knob: the cache reached **20.3 GB** and free space
fell to **15.6 GB**; with it, the same tree is **4.0 GB** and free space **45 GB**. Two
rounds of a battery is the difference between "plenty" and "a build fails halfway".

The price is real and must be disclosed rather than quietly applied: a panic backtrace in a
failing test becomes close to useless, so **drop the two variables for the round that
debugs a failure**. `gate.ps1` now writes the knobs into the gate log when they are set,
because a log that does not record them describes a run that did not happen.

**Half the cache was incremental compilation, and nobody had measured it.**

Measured 2026-10-08 (M255): with the debug knobs off, the tree was **12.6 GB** — of which
**7.3 GB was `debug/incremental`**. Deleting it took the tree to **5.4 GB** and free space to
**25.7 GB**, and costs only a slower rebuild of whatever changes next; dependency artifacts
are untouched.

```sh
rm -rf "$TEMP/epsilon-target/debug/incremental"   # ~7 GB back, no rebuild of deps
export CARGO_INCREMENTAL=0                        # stop it regrowing, slower incremental builds
```

Worth knowing before a big `cargo install`: a release install adds **~0.9 GB** on top
(`cargo clean --release` reclaims it exactly).

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

**The pin is on the path, so the knob is a one-time cost per tree, not per battery** (measured
2026-10-02, three rounds running). `LNK1104` named
`debug\deps\epsilon_core-4a8081d0361ce74c.exe` on two consecutive batteries; the third set
`CARGO_PROFILE_TEST_DEBUG=1` and did not hit it again. Cost breakdown:

| state | test leg |
|---|---|
| warm tree, default metadata | ~675 s, **and vulnerable to the pin** |
| cold tree, `CARGO_PROFILE_TEST_DEBUG=1` | 1,489–2,402 s — every test executable relinks |
| warm tree, `CARGO_PROFILE_TEST_DEBUG=1` | **505 s, and immune** |

So: set the knob the first time a session hits the fault, keep it for the rest of the session,
and do not `cargo clean` between rounds unless the disk forces it — cleaning throws away the
immunity along with the 4 GB.

## The PowerShell trap

`bash` on this machine is WSL: different `$HOME`, different `PATH`, no Windows `cargo`. It exits
127 in zero seconds and the log still says `GATE FAIL`, which reads like five failed gates instead
of one missing PATH entry. **Run `pwsh -File scripts/gate.ps1 <label>` here.** `scripts/gate.sh` is
for POSIX and CI.

## Every battery attempt gets its own label

`target/gate-<label>.log` is truncated on start, so re-using a label destroys the failed attempt's
evidence. Use a fresh label per attempt.
