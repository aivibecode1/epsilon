# Install Epsilon on Windows from this checkout.
#
#   powershell -ExecutionPolicy Bypass -File .\install.ps1
#   powershell -ExecutionPolicy Bypass -File .\install.ps1 -Prefix C:\Tools\epsilon
#   powershell -ExecutionPolicy Bypass -File .\install.ps1 -NoVerify
#
# Requires Rust 1.88 or newer (https://rustup.rs). Build output goes to a
# temporary target directory when CARGO_TARGET_DIR is unset: in-tree output has
# produced compiler failures that name a dependency rather than the directory.
# Admin rights are not required.

[CmdletBinding()]
param(
    [string]$Prefix = "",
    [switch]$NoVerify
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $Root

function Fail([string]$Message) {
    Write-Host "install.ps1: $Message" -ForegroundColor Red
    exit 1
}

$cargo = $null
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargoCommand) {
    $cargo = $cargoCommand.Source
} elseif (Test-Path (Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe")) {
    $cargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
}
if (-not $cargo) {
    Fail "cargo was not found. Install Rust 1.88+ from https://rustup.rs, reopen this terminal, and run the script again."
}

$rustc = Join-Path (Split-Path -Parent $cargo) "rustc.exe"
if (Test-Path $rustc) {
    $version = (& $rustc --version) -split " " | Select-Object -Index 1
    $parts = $version -split "\."
    $major = if ($parts.Count -ge 1 -and $parts[0] -match '^\d+$') { [int]$parts[0] } else { 0 }
    $minor = if ($parts.Count -ge 2 -and $parts[1] -match '^\d+$') { [int]$parts[1] } else { 0 }
    if ($major -lt 1 -or ($major -eq 1 -and $minor -lt 88)) {
        Fail "Rust $version is too old; Epsilon needs 1.88 or newer. Run: rustup update stable"
    }
}

if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = Join-Path $env:TEMP "epsilon-install-target"
}

Write-Host "Epsilon installer"
Write-Host "  checkout:     $Root"
Write-Host "  cargo:        $cargo"
Write-Host "  target dir:   $env:CARGO_TARGET_DIR"
if ($Prefix) { Write-Host "  install root: $Prefix" }

$installArgs = @("install", "--path", "crates/epsilon-cli", "--locked", "--force")
if ($Prefix) {
    $installArgs += @("--root", $Prefix)
}
& $cargo @installArgs
if ($LASTEXITCODE -ne 0) {
    Fail "cargo install failed with exit code $LASTEXITCODE"
}

if ($Prefix) {
    $binDir = Join-Path $Prefix "bin"
} elseif ($env:CARGO_HOME) {
    $binDir = Join-Path $env:CARGO_HOME "bin"
} else {
    $binDir = Join-Path $env:USERPROFILE ".cargo\bin"
}
$epsilon = Join-Path $binDir "epsilon.exe"

if (-not $NoVerify) {
    if (-not (Test-Path $epsilon)) {
        Fail "installation finished but $epsilon does not exist"
    }
    & $epsilon --version
    if ($LASTEXITCODE -ne 0) {
        Fail "$epsilon --version failed with exit code $LASTEXITCODE"
    }
}

Write-Host ""
Write-Host "Installed: $epsilon"
$pathEntries = ($env:Path -split ';') | Where-Object { $_ -and $_.TrimEnd('\') -eq $binDir.TrimEnd('\') }
if (-not $pathEntries) {
    Write-Host ""
    Write-Host "$binDir is not on PATH. Add it for your user:"
    Write-Host "  [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path','User') + ';$binDir', 'User')"
    Write-Host "Then open a new terminal."
}

Write-Host ""
Write-Host "Next:"
Write-Host "  epsilon --demo     # offline smoke test, no key and no network"
Write-Host "  epsilon --help     # modes, flags, environment and files"
Write-Host "  epsilon --rea      # optional: attach the REA reverse-engineering MCP server"
Write-Host ""
Write-Host "Windows note: shell commands run inside a Job Object (process tree and memory"
Write-Host "limits) but not a filesystem boundary. Use /plan for a read-only session."
