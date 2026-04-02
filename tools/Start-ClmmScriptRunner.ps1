# Convenience wrapper: start localhost script runner with env loaded from repo-root .env.
# This avoids manual `$env:CLMM_SCRIPT_RUNNER_TOKEN=...` each time.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

& pwsh -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoRoot "tools\\script_runner\\Start-ClmmScriptRunner.ps1")
