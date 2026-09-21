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
    @{ Name = "fmt --check"; Args = @("fmt", "--all", "--", "--check") },
    # `-D warnings`: the workspace denies its configured lints, but clippy's own
    # `-W clippy::all` findings exit 0 by default - which is how a warning got past
    # this gate once (M69).
    @{ Name = "clippy"; Args = @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings") },
    @{ Name = "test"; Args = @("test", "--workspace") },
    @{ Name = "demo"; Args = @("run", "-q", "-p", "epsilon-cli", "--", "--demo") },
    @{ Name = "real-check"; Args = @("check", "-p", "epsilon-cli", "--features", "real") }
)

$overall = 0
foreach ($gate in $gates) {
    Add-Content -Path $log -Value "`n===== $($gate.Name) =====" -Encoding utf8
    $start = Get-Date
    & $cargo @($gate.Args) *>&1 | Tee-Object -FilePath $log -Append | Out-Null
    $rc = $LASTEXITCODE
    $seconds = [int]((Get-Date) - $start).TotalSeconds
    Add-Content -Path $log -Value "[exit=$rc ${seconds}s]" -Encoding utf8
    if ($rc -ne 0) { $overall = 1 }
}

$verdict = if ($overall -eq 0) { "PASS" } else { "FAIL" }
Add-Content -Path $log -Value "`n===== GATE $verdict =====" -Encoding utf8
Write-Output "GATE $verdict - target/gate-$Label.log"
exit $overall
