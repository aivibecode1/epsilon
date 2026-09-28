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

## NOT RESOLVED

**D8050 survives.** Switching TLS from `aws-lc-rs` to `ring` cleared the NASM
and perl problems and removed `aws-lc-sys` entirely — but `ring` is *also* a C
build, and now fails with **the same D8050**, on the same machine, in the same
environment where `cl.exe` compiles the identical file by hand with exit 0.

So the fault belongs to `cl.exe` on this machine, not to any one crate. Worth
recording: D8050 is **undocumented by Microsoft** (only its sibling D8049 has
official docs); the identical string appears in
[microsoft/vcpkg#25955](https://github.com/microsoft/vcpkg/issues/25955), closed
as "not enough information"; and a StackOverflow answer attributes it to user
rights with a session missing `USERPROFILE` (which is set here).

**Left to try — all owner's decisions, none to be taken silently from an agent
session:** install NASM and revert to `aws-lc-rs`; or repair/reinstall the
Visual Studio toolchain, which was the vcpkg maintainer's only suggestion; or
run the battery where MSVC is known good. Changing antivirus configuration or
installing software is out of scope for an automated session.

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

Build on `C:` only, in the repo's `target/`. `CARGO_BUILD_JOBS=1`. One build
owner. `pwsh -File scripts/gate.ps1 <label>`. Fresh label per attempt.
`gate.sh` is WSL here and cannot find cargo.
