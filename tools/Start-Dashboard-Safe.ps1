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
# Optionally start script runner in a separate window.
if ($WithRunner.IsPresent) {
  $runner = Join-Path $RepoRoot "tools\\Start-ClmmScriptRunner.ps1"
  if (Test-Path -LiteralPath $runner) {
    # If 9847 is taken (commonly by HTTP.sys / PID 4), pick a stable alternate port.
    try {
      # Netstat can show HTTP.sys reservations (PID 4) even when the runner is healthy.
      # Use /health instead of LISTENING detection.
      $runnerHealthy = $false
      try {
        $resp = Invoke-RestMethod -Method Get -Uri "http://127.0.0.1:9847/health" -TimeoutSec 1 -ErrorAction Stop
        if ($null -ne $resp -and $resp.ok -eq $true) { $runnerHealthy = $true }
      } catch {
        # best effort only
      }

      if (-not $runnerHealthy) {
        $env:CLMM_SCRIPT_RUNNER_PORT = "9857"
        $env:SCRIPT_RUNNER_URL = "http://127.0.0.1:9857"
        Write-Warning "[Start-Dashboard-Safe] Runner on :9847 not responding; runner will use :9857 and API will use SCRIPT_RUNNER_URL=$env:SCRIPT_RUNNER_URL"
      }
    } catch {
      # best effort
    }

    Write-Host "[Start-Dashboard-Safe] Starting script runner (default :9847)..." -ForegroundColor Cyan
    # Keep window open on errors, so failures are visible.
    Start-Process -FilePath "pwsh" `
      -ArgumentList @("-NoProfile", "-NoExit", "-ExecutionPolicy", "Bypass", "-File", $runner) `
      -WorkingDirectory $RepoRoot `
      -WindowStyle Normal
  } else {
    Write-Warning "[Start-Dashboard-Safe] Missing tools/Start-ClmmScriptRunner.ps1; runner not started."
  }
}

# Start API in a separate window, always :8081 (inherits any SCRIPT_RUNNER_URL override from this process).
& (Join-Path $RepoRoot "tools\\Start-ClmmApi-8081.ps1")

# Vite proxy points at :8081 — if we start the browser before `cargo` finishes, UI shows HTTP 502.
$apiHealthUrl = "http://127.0.0.1:8081/api/v1/health"
$maxWaitSec = 180
$intervalSec = 2
$deadline = (Get-Date).AddSeconds($maxWaitSec)
Write-Host "[Start-Dashboard-Safe] Czekam na API (pierwszy build Rusta może trwać kilka minut): $apiHealthUrl" -ForegroundColor Cyan
$apiUp = $false
$elapsed = 0
while ((Get-Date) -lt $deadline) {
  try {
    $null = Invoke-RestMethod -Method Get -Uri $apiHealthUrl -TimeoutSec 3 -ErrorAction Stop
    $apiUp = $true
    break
  } catch {
    # API jeszcze nie nasłuchuje albo trwa kompilacja
  }
  Start-Sleep -Seconds $intervalSec
  $elapsed += $intervalSec
  if (($elapsed % 10) -eq 0 -and $elapsed -gt 0) {
    Write-Host "[Start-Dashboard-Safe] nadal czekam... ${elapsed}s / ${maxWaitSec}s (okno API + tools\logs)" -ForegroundColor DarkGray
  }
}
if ($apiUp) {
  Write-Host "[Start-Dashboard-Safe] API odpowiada — startuję Vite." -ForegroundColor Green
} else {
  Write-Warning "[Start-Dashboard-Safe] Brak odpowiedzi z API po ${maxWaitSec}s. Vite i tak wystartuje; sprawdź drugie okno pwsh (cargo) i log w tools\logs — np. PostgreSQL / DATABASE_URL."
}

# Start Vite in this window (no kill-port, no touching API ports)
Set-Location $WebDir
$env:API_UPSTREAM = "http://127.0.0.1:8081"

# If the API enforces X-API-Key, expose it to Vite as a client env var.
# Vite only forwards env vars prefixed with VITE_ to the frontend.
if (-not $env:VITE_API_KEY -or $env:VITE_API_KEY.Trim().Length -eq 0) {
  if ($env:CLMM_API_KEY -and $env:CLMM_API_KEY.Trim().Length -gt 0) {
    $env:VITE_API_KEY = $env:CLMM_API_KEY.Trim()
  } elseif ($env:API_KEY -and $env:API_KEY.Trim().Length -gt 0) {
    $env:VITE_API_KEY = $env:API_KEY.Trim()
  }
}

Write-Host "[Start-Dashboard-Safe] Starting Vite on :3000 (proxy -> $env:API_UPSTREAM)..." -ForegroundColor Cyan
npx vite

