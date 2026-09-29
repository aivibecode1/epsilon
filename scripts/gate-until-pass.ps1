<#
.SYNOPSIS
    Run the verification battery until it passes, or until it stops being worth retrying.

.DESCRIPTION
    A cold battery on this machine is 8-15 minutes, and this repository's own rules
    make a blind retry worse than no retry:

    * `scripts/gate.ps1` TRUNCATES `target/gate-<label>.log` on start, so every attempt
      needs its OWN label or the previous attempt's evidence is destroyed.
    * A deterministic failure (a real bug, a wrong assertion) fails identically every
      time. Retrying it three times costs half an hour and teaches nothing.
    * An environmental failure (Avira `LNK1104`, `os error 2`/`5` from a quarantined
      artifact, `0xc0000142` compiler-startup contention) is worth exactly one retry.

    So this wrapper retries on evidence, not on hope: it fingerprints the failure and
    stops as soon as two attempts produce the same one, which is the definition of
    "not transient" for a test suite.

    It never edits `gate.ps1`. That script is the evidence of record and a retry
    wrapper that changed what it runs would be a wrapper that cannot be trusted.

.PARAMETER Label
    Base label. Attempt N is logged as `<Label>-r<N>`, so the base label stays
    free for the definitive run.

.PARAMETER MaxAttempts
    Ceiling on attempts. Default 3: one retry for a transient fault, one more to
    confirm a repeat, and no more.

.EXAMPLE
    pwsh -File scripts/gate-until-pass.ps1 -Label m200
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Label,
    [int]$MaxAttempts = 3
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$gate = Join-Path $PSScriptRoot 'gate.ps1'
if (-not (Test-Path $gate)) { throw "gate.ps1 not found beside this script: $gate" }

# Faults this machine produces on its own, as opposed to faults in the code.
$environmental = @(
    'LNK1104',            # Avira pinning a freshly linked artifact path
    'os error 5',         # Access is denied - the same quarantine, different surface
    'os error 2',         # the rename-after-write AV race (see m142-failed-round)
    '0xc0000142',         # compiler-startup contention at parallel jobs
    'c1\.dll',            # the cl.exe D8050 build fault
    'Access is denied'
)

function Get-FailureFingerprint {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return "no-log:$([IO.Path]::GetFileName($Path))" }
    $text = Get-Content -Raw -LiteralPath $Path
    $names = [regex]::Matches($text, '(?m)^test ([^\s]+) \.\.\. FAILED') |
        ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique
    $gates = [regex]::Matches($text, '(?m)^===== ([a-z-]+) =====\r?\n[\s\S]{0,4000}?^\[exit=([1-9])') |
        ForEach-Object { "$($_.Groups[1].Value):$($_.Groups[2].Value)" } | Sort-Object -Unique
    $parts = @()
    $parts += "tests:$(($names -join ','))"
    $parts += "gates:$(($gates -join ','))"
    foreach ($marker in $environmental) {
        if ($text -match $marker) { $parts += "env:$marker" }
    }
    return ($parts -join '|')
}

$previous = $null
$attempts = 0

while ($attempts -lt $MaxAttempts) {
    $attempts++
    $attemptLabel = if ($attempts -eq 1) { $Label } else { "$Label-r$attempts" }
    $log = Join-Path $root "target/gate-$attemptLabel.log"

    Write-Host ""
    Write-Host "=== battery attempt $attempts/$MaxAttempts  (label: $attemptLabel) ===" -ForegroundColor Cyan
    Write-Host "    log: $log"

    & $gate $attemptLabel | Out-Null

    $text = if (Test-Path $log) { Get-Content -Raw -LiteralPath $log } else { '' }
    if ($text -match '===== GATE PASS =====') {
        Write-Host ""
        Write-Host "GATE PASS on attempt $attempts - target/gate-$attemptLabel.log" -ForegroundColor Green
        exit 0
    }

    $fingerprint = Get-FailureFingerprint -Path $log
    $isEnvironmental = $fingerprint -match 'env:'

    Write-Host ""
    Write-Host "attempt $attempts did NOT pass." -ForegroundColor Yellow
    Write-Host "  fingerprint: $fingerprint"

    if ($fingerprint -eq $previous) {
        Write-Host "  same fingerprint as the previous attempt - that is not a flake, that is the code." -ForegroundColor Red
        Write-Host "  stopping. Evidence: $log"
        exit 1
    }
    if (-not $isEnvironmental -and $attempts -ge 2) {
        Write-Host "  no environmental marker, and it has now failed twice." -ForegroundColor Red
        Write-Host "  stopping. Evidence: $log"
        exit 1
    }

    $previous = $fingerprint
    if ($isEnvironmental) {
        Write-Host "  environmental fault detected - retrying on a fresh label." -ForegroundColor Yellow
    } elseif ($attempts -lt $MaxAttempts) {
        Write-Host "  retrying once to rule out a cross-test race." -ForegroundColor Yellow
    }
    else {
        Write-Host "  attempt ceiling reached." -ForegroundColor Red
        Write-Host "  stopping. Evidence: $log"
        exit 1
    }
}

exit 1
