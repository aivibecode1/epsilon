# Full verification battery for Epsilon, for Windows.
#
#   pwsh -File scripts/gate.ps1 <label>
#
# This is the sibling of scripts/gate.sh, and on this machine it is the one that
# works. `bash` here is WSL: it has its own `$HOME`, its own PATH, and no Windows
# `cargo`, so gate.sh exits 127 before running a single gate — which reads in the
# log like five broken gates rather than one missing PATH entry. (It has cost this
# project more than one round; the story is in AGENTS.md, "This machine".)
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

# Build output goes OUTSIDE this repository, and that is load-bearing rather
# than tidiness.
#
# M196: writing build output inside this directory makes `cl.exe` fail with
#
#   Command line error D8050 : cannot execute '...\c1.dll': failed to get
#   command line into debug records
#
# which cargo reports as `failed to run custom build command for <crate>` --
# naming a dependency rather than the cause. It is **specific to this
# directory**, measured by varying one thing at a time with a forced
# `cargo clean -p <crate>` each time so a cache hit could not pass for a fix:
#
#   ...\Desktop\Epsilon-target                       len 34   D8050 0
#   ...\My Agent Space\Eps2\target  (sibling)       len 46   D8050 0
#   ...\My Agent Space\Another Name Here\target     len 44   D8050 0
#   ...\Temp\bt aaaaaaaaaaaaaaaaaaaaaaaa (long+space) len 60  D8050 0
#   ...\My Agent Space\Epsilon\t2   (inside repo)   len 45   D8050 1
#   ...\My Agent Space\Epsilon\target                len 49   D8050 1
#
# A longer path passes and a shorter one fails, so it is not path length; and
# changing only the last name -- `Epsilone`, `EpsilonX`, `Xpsilon`, `epsilonn`
# -- all pass, so it is not the name. Something outside the compiler is holding
# this tree, and this is what that looks like from inside `c1.dll`. See
# `agents/memory/build-blockers-resolved.md`.
#
# `%TEMP%\epsilon-target` is used rather than a fixed drive path: it is on the
# same volume, it survives no configuration, and it is the one place a build
# artifact is expected to be disposable.
$env:CARGO_TARGET_DIR = Join-Path $env:TEMP "epsilon-target"

# The MSVC build environment, loaded before anything runs.
#
# M196 moved TLS to `ring`, so the workspace no longer has a C dependency and
# this is no longer load-bearing for a clean build. It stays because a
# developer with a native crate in their tree still needs it, and because the
# failure it prevents is the worst-shaped one this repository has: a
# non-interactive shell does not run `vcvars`, so INCLUDE/LIB are empty, `cc`
# finds `cl.exe` and it exits 2 with no useful message, and every compile gate
# reports a broken dependency tree. Cost when already loaded: one `cmd /c` that
# returns immediately.

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
    Write-Warning "gate.ps1: INCLUDE is still empty after vcvars. Any crate with a C build step will fail with exit 101, and the error will name the dependency rather than the environment."
    exit 101
}

# Scratch directory for the build.
#
# MSBuild (reached through any remaining native toolchain) creates
# `$TEMP\MSBuildTemp` itself and that intermittently throws
# `UnauthorizedAccessException`, so TEMP is pointed somewhere we control.
#
# The two constraints, both learned the hard way here:
#
# 1. NOT inside the repository. `load_project_instructions` walks UP from a
#    workspace looking for AGENTS.md/CLAUDE.md (prompt.rs:213-223), so a temp
#    directory at `<repo>/target/buildtmp` inherits the repository's own
#    AGENTS.md and two prompt tests fail with one extra entry -- a failure
#    caused entirely by where the scratch directory is.
# 2. NOT a fresh directory under %LOCALAPPDATA%. A newly created tree there
#    draws `PermissionDenied` from the antivirus on the first `tempfile`
#    directory created inside it, which failed 40 epsilon-cli tests at once. The
#    OS's own %TEMP% is the one tree that is created once, scanned, and left
#    alone -- so the fix is to use a subdirectory of %TEMP%, not a sibling.
$buildTmp = Join-Path $env:TEMP "epsilon-build"
New-Item -ItemType Directory -Force -Path $buildTmp | Out-Null
$env:TEMP = $buildTmp
$env:TMP = $buildTmp
New-Item -ItemType Directory -Force -Path (Join-Path $buildTmp "MSBuildTemp") -ErrorAction SilentlyContinue | Out-Null

# A WINDOWS-NATIVE perl, and no MSYS perl.
#
# The perl that happens to be first on PATH in some agent shells is the msys2
# build, and it dies with
# `NtCreateDirectoryObject(...): 0xC0000022` (ACCESS_DENIED) on anything that
# needs a shared object. Git for Windows' perl is a native win32 build and works.
# We copy perl alone into a clean directory rather than putting `Git\usr\bin` on
# PATH, because that directory also ships GNU coreutils' `link.exe`, which
# shadows the MSVC linker and fails the Rust link with `extra operand`.
#
# M196 moved TLS to `ring`, so no C library needs perl or NASM any more. This
# stays because it costs one directory copy and a PATH edit, and because a
# future native dependency would fail the same way if it were removed.

$perlBin = Join-Path $buildTmp "perlbin"
if (-not (Test-Path (Join-Path $perlBin "perl.exe"))) {
    New-Item -ItemType Directory -Force -Path $perlBin | Out-Null
    foreach ($f in @("perl.exe", "perl538.dll", "libgcc_s_seh-1.dll", "libwinpthread-1.dll", "gmsys-2.0.dll")) {
        $src = Join-Path "C:\Program Files\Git\usr\bin" $f
        if (Test-Path $src) { Copy-Item $src $perlBin -Force }
    }
}
$env:PATH = (@($perlBin) + @(
    $env:PATH -split ';' | Where-Object {
        $_ -and $_ -notmatch 'hermes\\tools\\git-' -and $_ -notmatch 'msys64'
    }
)) -join ';'

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
# **The debug knobs are written into the log when they are set, because they change what
# the evidence is.** `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0` cut a full
# battery from ~33 GB to ~4 GB (M208, measured) and make it far faster, and the price is a
# backtrace in a failing test that is close to useless. That trade is fine — but only if the
# log says it happened, because a log that does not record it describes a run that did not
# occur. The rule AGENTS.md states ("name the knob in the gate log and the milestone") was
# half-implemented: the milestone named it, the log did not.
$profile = @(
    'CARGO_PROFILE_DEV_DEBUG', 'CARGO_PROFILE_TEST_DEBUG', 'CARGO_PROFILE_RELEASE_DEBUG'
) | ForEach-Object {
    $v = [Environment]::GetEnvironmentVariable($_)
    if ($null -ne $v -and $v -ne '') { "$_=$v" }
}
$header = @("cargo: $cargo", "CARGO_BUILD_JOBS: $env:CARGO_BUILD_JOBS")
if ($profile) { $header += "profile knobs: $($profile -join ' ')" }
Set-Content -Path $log -Value ($header -join "`n") -Encoding utf8

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
    # `AGENTS.md` is maintainer-owned and locked: an agent that may rewrite the rules is
    # not governed by them. The lock is a SHA-256 in `AGENTS.md.sha256`, and this gate
    # fails when the bytes move. A proposal goes in `docs/AGENTS-PROPOSALS.md` instead.
    @{ Name = "check-agents-lock"; Exe = "python"; Args = @("scripts/check-agents-lock.py") },
    # Cargo does not merge a package `[lints]` table with `[workspace.lints]`: a
    # package table REPLACES it unless it says `workspace = true`, and cargo
    # rejects `workspace = true` outright when the same table carries overrides.
    # So a crate that must override one lint — `epsilon-safety`, for the FFI —
    # has to hand-copy the whole table, and a lint added at the root afterwards
    # reaches the other eleven crates and stops there. `epsilon-lsp` had exactly
    # that bug (D1/D21) and it was found by a human reading twelve manifests.
    # This is the check that means nobody has to.
    @{ Name = "check-lint-tables"; Exe = "python"; Args = @("scripts/check-lint-tables.py") },
    # Parity-matrix citations name production lines, and moving code re-points
    # every citation below it without touching the row: a claim that cannot be
    # followed. The export move deleted 53 lines and shifted nineteen rows in
    # round 14, all repaired by hand; this is the check that means nobody has
    # to. Reference-side paths are excluded (not committed) — see the script.
    @{ Name = "check-parity-citations"; Exe = "python"; Args = @("scripts/check-parity-citations.py", "--quiet") },
    # Verb counts are stated in prose in four places and updated by hand every
    # verb round; four rounds running each missed at least one site. Dated
    # records (ADR amendments, the STATUS log) are excluded by design — see
    # the script.
    @{ Name = "check-verb-counts"; Exe = "python"; Args = @("scripts/check-verb-counts.py", "--quiet") },
    # Matrix summaries, totals, open-row lists and embedded heading counts are
    # hand-maintained; stale ids sat in a missing list two rounds after going
    # done. Per-section placement is verified for pi-tui only (dsh-desktop
    # uses three namings; see the script).
    @{ Name = "check-parity-counts"; Exe = "python"; Args = @("scripts/check-parity-counts.py", "--quiet") },
    # A CRLF working copy is invisible to `git diff` (`.gitattributes` normalises
    # on commit), and it breaks every source-parsing test in the tree in a way that
    # reports something else entirely -- M231 lost a battery to a dispatcher
    # reported as having no catch-all arm, which was a lie. This costs a second and
    # it names the file and the line.
    @{ Name = "check-line-endings"; Exe = "python"; Args = @("scripts/check-line-endings.py", "--quiet") },
    # A C0 control byte inside a tracked text file is a corrupted *letter*, not
    # content -- `e` -> 0x1B, `a` -> 0x07, `b` -> 0x08 is the shape this repo has
    # shipped 15 times, twice inside `epsilon-runtime/src/text_safety.rs`, the module
    # whose whole job is stripping those bytes. No other gate can see it: fmt and
    # clippy read code, and a control byte in a comment is not code (ledger A49).
    @{ Name = "check-control-bytes"; Exe = "python"; Args = @("scripts/check-control-bytes.py", "--quiet") },
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
