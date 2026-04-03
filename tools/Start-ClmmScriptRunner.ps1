# Convenience wrapper: start localhost script runner with env loaded from repo-root .env.
# This avoids manual `$env:CLMM_SCRIPT_RUNNER_TOKEN=...` each time.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

$runner = Join-Path $repoRoot "tools\\script_runner\\Start-ClmmScriptRunner.ps1"
$stopper = Join-Path $repoRoot "tools\\Stop-ClmmScriptRunner.ps1"

# We own this runner port: always stop stale runner instances before restart.
if (Test-Path -LiteralPath $stopper) {
  try { & pwsh -NoProfile -ExecutionPolicy Bypass -File $stopper | Out-Null } catch {}
}

# If the default port is taken by some other service (PID 4 / HTTP.sys), pick a different port.
if (-not $env:CLMM_SCRIPT_RUNNER_PORT -or $env:CLMM_SCRIPT_RUNNER_PORT.Trim().Length -eq 0) {
  try {
    # PID 4 / HTTP.sys can appear as "listener" even when the runner is healthy.
    # Use /health to decide whether we should keep :9847.
    $runnerHealthy = $false
    try {
      $resp = Invoke-RestMethod -Method Get -Uri "http://127.0.0.1:9847/health" -TimeoutSec 1 -ErrorAction Stop
      if ($null -ne $resp -and $resp.ok -eq $true) { $runnerHealthy = $true }
    } catch {
      # best effort only
    }

    if (-not $runnerHealthy) {
      Write-Warning "Runner on :9847 not responding; switching runner to port 9857 (override with CLMM_SCRIPT_RUNNER_PORT)."
      $env:CLMM_SCRIPT_RUNNER_PORT = "9857"
      if (-not $env:SCRIPT_RUNNER_URL) {
        $env:SCRIPT_RUNNER_URL = "http://127.0.0.1:9857"
      }
    }
  } catch {
    # best effort only
  }
}

& pwsh -NoProfile -ExecutionPolicy Bypass -File $runner
