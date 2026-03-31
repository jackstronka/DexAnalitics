# Build `clmm-lp-cli` (Windows). Universal: Release (default) or Debug.
#
# Usage (repo root):
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\build_clmm_lp_cli.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\build_clmm_lp_cli.ps1 -Configuration Debug
#
# Release binary (preferred by tools/orca_*.ps1 via Resolve-ClmmLpCliExe):
#   target\release\clmm-lp-cli.exe
# Debug:
#   target\debug\clmm-lp-cli.exe

param(
  [ValidateSet("Release", "Debug")]
  [string] $Configuration = "Release",

  [string] $RepoRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Info([string]$m) { Write-Host ("[build-cli] " + $m) }

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
  if (-not $PSScriptRoot) { throw "[build-cli] PSScriptRoot unset; pass -RepoRoot." }
  $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
}
Set-Location $RepoRoot

$outDir = if ($Configuration -eq "Release") { "release" } else { "debug" }
$exe = Join-Path $RepoRoot ("target\" + $outDir + "\clmm-lp-cli.exe")

if ($Configuration -eq "Release") {
  Info "cargo build --release -p clmm-lp-cli --bin clmm-lp-cli"
  cargo build --release -p clmm-lp-cli --bin clmm-lp-cli
} else {
  Info "cargo build -p clmm-lp-cli --bin clmm-lp-cli  (dev profile)"
  cargo build -p clmm-lp-cli --bin clmm-lp-cli
}

if ($LASTEXITCODE -ne 0) { throw "[build-cli] cargo failed exit=$LASTEXITCODE" }
if (-not (Test-Path -LiteralPath $exe)) { throw "[build-cli] missing $exe" }
Info ("OK: " + $exe)
