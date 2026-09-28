# The build blockers — PARTIALLY resolved

**Status: the repository builds, and every gate except `test` is green. The last
blocker is a machine fault, not a code fault.** Read this before touching the
build again; the ruled-out list is the expensive part.

## Where it stands

| Gate | State |
|---|---|
| `check-links` | green |
| `fmt --check` | green |
| `clippy -D warnings` | **green, 0 warnings** — the first time this tree ever compiled here |
| `demo` | green |
| `real-check` | green |
| `test` | 404 of 406 pass; the 2 failures are a machine-level `PermissionDenied` |

## RESOLVED (each measured, not inferred)

1. **The MSVC environment is never loaded in a non-interactive shell.** `vcvars`
   does not run, so `INCLUDE`/`LIB` are empty; `cc` finds `cl.exe`, which then
   exits 2 with no useful message. → `gate.ps1` loads `vcvars64.bat`.

2. **`aws-lc-sys` is a C library needing NASM, CMake and MSBuild**, and NASM is
   not installed here. Its `.asm` generation also runs **perl**, and the perl
   first on PATH in an agent shell is the **msys2** build, which dies with
   `NtCreateDirectoryObject(...): 0xC0000022` (ACCESS_DENIED). Git for Windows'
   perl is a native win32 build and works → `gate.ps1` copies perl alone into a
   directory of its own, because `Git\usr\bin` also ships GNU coreutils'
   `link.exe`, which shadows the MSVC linker and fails the Rust link with
   `extra operand`.

3. **MSBuild cannot create `$TEMP\MSBuildTemp`** — `UnauthorizedAccessException`,
   surfaced by CMake as `Failed to run MSBuild command` and by cargo as a failed
   custom build command. → `gate.ps1` points `TEMP` at a directory it owns and
   pre-creates it. **3 runs, 0 errors.**

4. **`D8050: cannot execute c1.dll: failed to get command line into debug
   records`** — deterministic under cargo (7 per build, 3/3 runs) and
   **impossible to reproduce by hand** with the same arguments, environment and
   CWD. Nine hypotheses measured and thrown away first: command-line length
   (1,547 chars against a 32,767 limit), path spaces, `-Z7`, the MSVC env,
   `TEMP`, 8-way parallelism, build-dir CWD, non-ASCII paths, NASM.

## SOLVED: D8050 is caused by building INSIDE the Epsilon repo directory

**A delegated investigation proposed a cause and I checked it. The proposal was
wrong on both of its claims, and the controlled experiment that refuted it also
produced the real answer.**

The proposal was that D8050 is a debug-record path-length bug, triggered by a
long path containing a space. Two measurements kill it:

| target dir | `-Fo` path len | D8050 |
|---|---|---|
| `…\Temp\bt aaaaaaaaaaaaaaaaaaaaaaaaaa` | **128** | **0** |
| `…\My Agent Space\Epsilon\target` | **117** | **7** |

**A longer path passes and a shorter one fails**, so it is not length. And
`C:\b t` (short, with a space) also passes, so it is not the space.

What is actually the variable, from five controlled runs varying one thing at a
time — every one with a forced `cargo clean -p ring` so a cache hit could not
pass for a fix:

| target dir | len | D8050 |
|---|---|---|
| `…\Desktop\Epsilon-target` | 34 | 0 |
| `…\Desktop\MYAGEN~1\Epsilon\target` (8.3) | 43 | 0 |
| `…\My Agent Space\Eps2\target` (sibling) | 46 | 0 |
| `…\My Agent Space\Another Name Here\target` | 44 | 0 |
| `…\Temp\bt aaaa…` (long, spaces) | 60 | 0 |
| **`…\My Agent Space\Epsilon\t2`** (inside the repo) | 45 | **1** |
| **`…\My Agent Space\Epsilon\target`** | 49 | **1** |

and then, holding everything else fixed and changing only the final name:

| final name, all under `My Agent Space` | D8050 |
|---|---|
| **`Epsilon`** | **1** |
| `Epsilone` (one char longer) | 0 |
| `EpsilonX` (a superset) | 0 |
| `Xpsilon` (same length) | 0 |
| `epsilonn` (lower case) | 0 |

**The failure occurs only when build output is written inside the real Epsilon
directory.** A sibling with the same parent, the same depth, the same length,
the same spaces, and a name that is a prefix, a superstring, or the same length
— all pass.

**Conclusion: the directory is being watched or held by something outside the
compiler**, and D8050 is what that looks like from inside `cl.exe`. Reaching
`c1.dll` requires a handle the scanner is holding. That is consistent with the
avalanche of unexplained `UnauthorizedAccessException`s on `%TEMP%` earlier in
the same session, and with this repository's own recorded experience of Avira
blocking writes on freshly created files (AGENTS.md, "Windows notes").

**The fix that requires nothing:** build somewhere else.

```sh
export CARGO_TARGET_DIR=C:/bt   # or wherever, outside this directory
```

Verified: `exit=0, D8050=0` on a full rebuild.

**And a corollary about how this was found:** nine hypotheses were measured and
thrown away before a delegated agent supplied one that was also wrong, and the
controlled experiment designed to *confirm* it is what isolated the real cause.
A test built to confirm a belief is worth more when it can fail.

**What is still an owner's decision:** the underlying reason. Identifying the
exact component holding the handle, and excluding the directory from the
scanner, both require administrator rights and a security-configuration change.
Not something to do silently from an agent session.

## Two failures I caused, both fixed, both instructive

- **A scratch directory inside the repository** broke two prompt tests.
  `load_project_instructions` walks UP from a workspace looking for AGENTS.md
  (`prompt.rs:213-223`), so a temp dir at `<repo>/target/buildtmp` inherits the
  repository's own `AGENTS.md` and the counts come out one too high. The scratch
  directory now lives under `%TEMP%`, outside the repository.

- **The terminal test never ran on a machine whose build path contains a
  space**, which this repository's does. `cmd /c "<path with space>"` fails
  because the pty re-quotes the argument and cmd searches for a program literally
  named `"C:\...\epsilon.exe"`. Measured and rejected: extra quoting, `call` via
  argv, and the 8.3 short path (this volume has none). The fix is
  `call "<path>"` **typed as keystrokes**, which is also what a user does. All 8
  terminal tests now pass, CMD and PowerShell.

## What I got wrong, and it cost most of the session

- **A PowerShell pipeline's `$LASTEXITCODE` read as cargo's** — reported
  `EXIT: 0` for a build that was exiting 101.
- **A subagent reported "build succeeded"** from a stale cache; a forced
  `cargo clean -p aws-lc-sys` showed it had not.
- **Four hand-rolled `cl.exe` probes gave false results** because PowerShell
  mangled argument quoting. They "disproved" things that were never tested.
- **A probe reporting `DENIED` for a temp-permission problem** was in fact
  reporting a C build failure. I read the wrong signal and chased the wrong
  thing for several rounds. **A probe must check what it claims to check.**

What actually worked: **stop running the compiler by hand and read the crate's
own build script** for the switches it already honours — then verify each layer
with the real battery rather than a hand-rolled probe.

## Disk

`%TEMP%` held **3,187 abandoned directories** from test runs I killed, plus a
stale `eps-m179-target` cargo cache of **9.6 GB** from M179. Both removed; C:
went from 13 GB free to 22 GB. Check `%TEMP%` after any run that is killed
rather than allowed to finish.

## Machine

- `CARGO_BUILD_JOBS=1`. One build owner. Fresh label per attempt.
- `pwsh -File scripts/gate.ps1 <label>`. `gate.sh` is WSL here and cannot find
  cargo.
- **Build output goes to `%TEMP%\epsilon-target`, NOT the repo's `target/`.**
  See the D8050 section above: building inside this directory fails. The
  battery's own log still lands in `<repo>/target/gate-<label>.log` so it is
  found where the docs say it is.
- Never on `D:` — the owner's rule.
