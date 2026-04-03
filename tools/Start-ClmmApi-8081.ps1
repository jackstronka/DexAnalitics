#Starts clmm-lp-api on :8081 without touching :8080 (Jenkins).

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $RepoRoot

# Ensure API isn't already running from previous session
try { & (Join-Path $RepoRoot "tools\\Stop-ClmmApi.ps1") | Out-Null } catch {}

$env:API_PORT = "8081"
$env:CLMM_REPO_ROOT = $RepoRoot

# If the script runner is configured to use a non-default port, align API env override.
if ($env:CLMM_SCRIPT_RUNNER_PORT -and $env:CLMM_SCRIPT_RUNNER_PORT -match '^\d+$') {
  $env:SCRIPT_RUNNER_URL = "http://127.0.0.1:$($env:CLMM_SCRIPT_RUNNER_PORT)"
}

Write-Host "[Start-ClmmApi-8081] Starting API on :8081 (does not touch :8080)..." -ForegroundColor Cyan

$logDir = Join-Path $RepoRoot "tools\\logs"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$ts = Get-Date -Format "yyyyMMdd_HHmmss"
$logPath = Join-Path $logDir ("clmm-lp-api_8081_{0}.log" -f $ts)

# Run in a separate window and keep it open on errors, so failures are visible.
Start-Process -FilePath "pwsh" `
  -ArgumentList @(
    "-NoProfile",
    "-NoExit",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
    "& { `$ErrorActionPreference='Continue'; `$env:API_PORT='8081'; `$env:CLMM_REPO_ROOT='$RepoRoot'; `$env:RUST_LOG='info'; Write-Host ('[clmm-lp-api] logging to: $logPath') -ForegroundColor DarkGray; cargo run -q -p clmm-lp-api --bin clmm-lp-api 2>&1 | Tee-Object -FilePath '$logPath' }"
  ) `
  -WorkingDirectory $RepoRoot `
  -WindowStyle Normal

Write-Host "[Start-ClmmApi-8081] OK. API should be reachable at http://127.0.0.1:8081/api/v1/health" -ForegroundColor Green
Write-Host "[Start-ClmmApi-8081] Logs: $logPath" -ForegroundColor DarkGray

