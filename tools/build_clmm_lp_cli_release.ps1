# Back-compat wrapper: builds Release `clmm-lp-cli` only.
# Prefer the universal script: tools/build_clmm_lp_cli.ps1 (-Configuration Release|Debug)
#
# Usage (repo root):
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\build_clmm_lp_cli_release.ps1

param([string] $RepoRoot = "")

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$universal = Join-Path $PSScriptRoot "build_clmm_lp_cli.ps1"
if (-not (Test-Path -LiteralPath $universal)) { throw "[build-cli] missing $universal" }

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
  & $universal -Configuration Release
} else {
  & $universal -Configuration Release -RepoRoot $RepoRoot
}
