# Starts API (:8081) + Vite (:3000) without kill-port (does not touch :8080 / Jenkins).

param(
  # If true, also starts the localhost scripts runner (tools/Start-ClmmScriptRunner.ps1) in a separate window.
  [switch] $WithRunner
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$WebDir = Join-Path $RepoRoot "web"

if (-not (Test-Path (Join-Path $WebDir "package.json"))) {
  throw "Missing web/package.json (run from CLMM repo root)."
}

if (-not (Test-Path (Join-Path $WebDir "node_modules"))) {
  Write-Host "[Start-Dashboard-Safe] npm install (first run)..." -ForegroundColor Cyan
  Push-Location $WebDir
  try { npm install } finally { Pop-Location }
}

# Start API in a separate window, always :8081
& (Join-Path $RepoRoot "tools\\Start-ClmmApi-8081.ps1")

# Optionally start script runner in a separate window.
if ($WithRunner.IsPresent) {
  $runner = Join-Path $RepoRoot "tools\\Start-ClmmScriptRunner.ps1"
  if (Test-Path -LiteralPath $runner) {
    Write-Host "[Start-Dashboard-Safe] Starting script runner (:9847)..." -ForegroundColor Cyan
    Start-Process -FilePath "pwsh" `
      -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $runner) `
      -WorkingDirectory $RepoRoot `
      -WindowStyle Normal
  } else {
    Write-Warning "[Start-Dashboard-Safe] Missing tools/Start-ClmmScriptRunner.ps1; runner not started."
  }
}

# Start Vite in this window (no kill-port, no touching API ports)
Set-Location $WebDir
$env:API_UPSTREAM = "http://127.0.0.1:8081"
Write-Host "[Start-Dashboard-Safe] Starting Vite on :3000 (proxy -> $env:API_UPSTREAM)..." -ForegroundColor Cyan
npx vite

