# Full verification battery for Epsilon, for Windows.
#
#   pwsh -File scripts/gate.ps1 <label>
#
# This is the sibling of scripts/gate.sh, and on this machine it is the one that
# works. `bash` here is WSL: it has its own `$HOME`, its own PATH, and no Windows
# `cargo`, so gate.sh exits 127 before running a single gate — which reads in the
# log like five broken gates rather than one missing PATH entry. (It has cost this
# project more than one round; the story is in AGENTS.md, "Windows notes".)
# gate.sh stays for POSIX and CI, where bash is a real POSIX shell with cargo.
#
# Every attempt needs its OWN label: the log is truncated on start, so re-running
# with a label that already exists destroys the failed attempt's evidence.
#
# The workspace rule for this box is one build owner at a time and single-job
# builds: every gate failure at parallel jobs has been 0xc0000142 compiler-startup
# contention. CARGO_BUILD_JOBS can be overridden from the environment.

param(
    [string]$Label = "run"
)

$ErrorActionPreference = "Continue"
Set-Location (Split-Path -Parent $PSScriptRoot)

if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = "1" }

# The MSVC build environment, loaded before anything runs.
#
# Why this is here: `reqwest`'s `rustls` feature compiles `aws-lc-sys`, a C
# library. `cc` finds `cl.exe` on its own, but `cl.exe` then fails with exit
# code 2 because it has no INCLUDE/LIB and cannot find the Windows SDK headers
# - so the failure reads as a broken dependency tree when it is a missing
# environment. A non-interactive shell never runs `vcvars`, so a build that
# works in a Developer Prompt fails here, and it fails in clippy, test, demo
# and real-check alike: four red gates for one missing variable.
#
# Cost when it is already loaded: one `cmd /c` that returns immediately.
# Cost when it is missing: a build that cannot succeed at all. Cheap insurance
# on a machine where the battery is the only evidence a change is sound.
$vcvars = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
if (-not $env:INCLUDE) {
    if (Test-Path $vcvars) {
        $vars = & cmd /c "`"$vcvars`" >nul 2>&1 && set"
        foreach ($line in $vars) {
            if ($line -match '^([^=]+)=(.*)$') {
                Set-Item -Path "env:$($matches[1])" -Value $matches[2] -ErrorAction SilentlyContinue
            }
        }
    }
}

# Record the toolchain the gates will actually run against. This exists because
# the failure above is *silent about its own cause*: four gates report a broken
# dependency tree when the real problem is one missing environment variable, and
# the only way to tell those apart in a month is a line in the log.
if (-not $env:INCLUDE) {
    Write-Warning "gate.ps1: INCLUDE is still empty after vcvars. The C dependency (aws-lc-sys, via reqwest/rustls) cannot compile; every compile gate will fail with exit 101 and the error will name the dependency, not the environment."
    exit 101
}

# MSBuild's scratch directory, created up front.
#
# `aws-lc-sys` falls back to a CMake+MSBuild build on Windows, and MSBuild
# creates this directory itself with `Directory.CreateDirectory`. That call
# intermittently loses a race and throws
# `System.UnauthorizedAccessException: Access to the path
# '...\Temp\MSBuildTemp' is denied`, which CMake reports as
# `Failed to run MSBuild command` and cargo reports as a failed custom build
# command -- naming the dependency, never the cause. Creating it here removes
# the race instead of losing it. It costs one `New-Item` on a path that is
# either absent (we create it) or already ours (we do not).
$msbuildTemp = Join-Path $env:TEMP "MSBuildTemp"
if ($env:TEMP -and -not (Test-Path $msbuildTemp)) {
    New-Item -ItemType Directory -Force -Path $msbuildTemp -ErrorAction SilentlyContinue | Out-Null
}

$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) {
    $fallback = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $fallback) { $cargo = $fallback }
}
if (-not $cargo) {
    Write-Error "gate.ps1: cannot find cargo - set CARGO or fix PATH"
    exit 127
}

New-Item -ItemType Directory -Force -Path target | Out-Null
$log = "target/gate-$Label.log"
Set-Content -Path $log -Value "cargo: $cargo`nCARGO_BUILD_JOBS: $env:CARGO_BUILD_JOBS" -Encoding utf8

# Each gate: a name, then the argument list. Kept as data so the log records what
# actually ran rather than what the script hoped it ran.
$gates = @(
    # Documentation links are a gate, not a chore. The audit renumbering of
    # 2026-09-28 left 40 dead relative links across the docs and nothing
    # noticed, because a dead citation looks exactly like a live one until a
    # reader follows it. It costs a second and it is the only check that can
    # see this class. `audit/history/` is excluded on purpose: it is superseded
    # and deliberately unrepaired (AGENTS.md).
    @{ Name = "check-links"; Exe = "python"; Args = @("scripts/check-links.py", "--quiet") },
    @{ Name = "fmt --check"; Exe = $cargo; Args = @("fmt", "--all", "--", "--check") },
    # `-D warnings`: the workspace denies its configured lints, but clippy's own
    # `-W clippy::all` findings exit 0 by default - which is how a warning got past
    # this gate once (M69).
    @{ Name = "clippy"; Exe = $cargo; Args = @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings") },
    @{ Name = "test"; Exe = $cargo; Args = @("test", "--workspace") },
    @{ Name = "demo"; Exe = $cargo; Args = @("run", "-q", "-p", "epsilon-cli", "--", "--demo") },
    @{ Name = "real-check"; Exe = $cargo; Args = @("check", "-p", "epsilon-cli", "--features", "real") }
)

$overall = 0
foreach ($gate in $gates) {
    Add-Content -Path $log -Value "`n===== $($gate.Name) =====" -Encoding utf8
    $start = Get-Date
    $exe = if ($gate.Exe) { $gate.Exe } else { $cargo }
    & $exe @($gate.Args) *>&1 | Tee-Object -FilePath $log -Append | Out-Null
    $rc = $LASTEXITCODE
    $seconds = [int]((Get-Date) - $start).TotalSeconds
    Add-Content -Path $log -Value "[exit=$rc ${seconds}s]" -Encoding utf8
    if ($rc -ne 0) { $overall = 1 }
}

$verdict = if ($overall -eq 0) { "PASS" } else { "FAIL" }
Add-Content -Path $log -Value "`n===== GATE $verdict =====" -Encoding utf8
Write-Output "GATE $verdict - target/gate-$Label.log"
exit $overall
