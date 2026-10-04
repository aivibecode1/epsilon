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

    # **Each `[exit=` belongs to the leg above it, and nothing else.**
    #
    # The first form matched a header and then the first non-zero exit *anywhere in the
    # next 4000 characters*, which named the wrong gate twice (ledger A46). A leg that
    # passed prints `[exit=0`, so the regex walked past it and picked up the next leg's
    # failure: `check-line-endings` failing was reported as `check-links:1`, and the
    # wrapper then stopped on a documentation fault believing it was the code. Separately,
    # a gate that printed more than 4000 characters (`fmt --check` prints a diff) matched
    # *nothing*, and an empty fingerprint compared equal to another empty one, so "same
    # fingerprint - that is not a flake" was reported for a battery that had named no gate.
    #
    # So: split on the headers, read the exit inside each segment, and never let a
    # segment reach past its own boundary.
    # `fmt --check` is one of the gate names, so the name is matched as "anything on the
    # line" rather than as `[a-z-]+`. The final `===== GATE FAIL =====` line matches too,
    # and is excluded by the same rule as a passing leg: no `[exit=` inside its segment.
    $headers = [regex]::Matches($text, '(?m)^===== (.+?) =====\r?$')
    $gates = @()
    for ($i = 0; $i -lt $headers.Count; $i++) {
        $start = $headers[$i].Index + $headers[$i].Length
        $end = if ($i + 1 -lt $headers.Count) { $headers[$i + 1].Index } else { $text.Length }
        $segment = $text.Substring($start, $end - $start)
        $exit = [regex]::Match($segment, '(?m)^\[exit=(\d)')
        if ($exit.Success -and $exit.Groups[1].Value -ne '0') {
            $gates += "$($headers[$i].Groups[1].Value):$($exit.Groups[1].Value)"
        }
    }
    $gates = $gates | Sort-Object -Unique

    $parts = @()
    $parts += "tests:$(($names -join ','))"
    # `NONE` rather than an empty field: a log with no failing leg named is a truncated
    # or malformed log, and the caller has to be able to tell that from "two attempts
    # failed identically". An empty string compared equal to itself is how a battery
    # with nothing to say reported agreement.
    $parts += "gates:$(if ($gates) { $gates -join ',' } else { 'NONE' })"
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
    # A battery that failed but named no failing leg has a truncated or malformed log.
    # That is not agreement with the previous attempt and it is not a flake -- it is
    # no evidence at all, and retrying on it burns the battery for nothing (A46).
    # The fingerprint is built as `tests:<names>|gates:<names>|env:<markers>`, so an
    # empty test list is the literal prefix `tests:|` and an empty gate list is `NONE`.
    if ($fingerprint.StartsWith('tests:|') -and $fingerprint -like '*|gates:NONE|*') {
        Write-Host "  the log names no failing gate and no failing test - it was truncated or the battery was killed." -ForegroundColor Red
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
