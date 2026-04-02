#Starts clmm-lp-api on :8081 without touching :8080 (Jenkins).

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $RepoRoot

# Ensure API isn't already running from previous session
try { & (Join-Path $RepoRoot "tools\\Stop-ClmmApi.ps1") | Out-Null } catch {}

$env:API_PORT = "8081"
$env:CLMM_REPO_ROOT = $RepoRoot

Write-Host "[Start-ClmmApi-8081] Starting API on :8081 (does not touch :8080)..." -ForegroundColor Cyan

Start-Process -FilePath "cargo" `
  -ArgumentList @("run", "-q", "-p", "clmm-lp-api", "--bin", "clmm-lp-api") `
  -WorkingDirectory $RepoRoot `
  -WindowStyle Normal

Write-Host "[Start-ClmmApi-8081] OK. API should be reachable at http://127.0.0.1:8081/api/v1/health" -ForegroundColor Green

