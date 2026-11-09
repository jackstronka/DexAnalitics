<#
.SYNOPSIS
  E1 verify-fast (Windows). Same as `make verify-fast` / `python tools/verify_fast.py`.

.EXAMPLE
  .\tools\verify-fast.ps1
  .\tools\verify-fast.ps1 -Base origin/main -DryRun
#>
param(
    [string]$Base = "origin/main",
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$argsList = @("--base", $Base)
if ($DryRun) { $argsList += "--dry-run" }

$py = Get-Command py -ErrorAction SilentlyContinue
if ($py) {
    & py -3 (Join-Path $PSScriptRoot "verify_fast.py") @argsList
} else {
    & python3 (Join-Path $PSScriptRoot "verify_fast.py") @argsList
}
exit $LASTEXITCODE
