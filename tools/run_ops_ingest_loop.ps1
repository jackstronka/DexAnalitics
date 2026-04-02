<# 
.SYNOPSIS
  Run `clmm-lp-cli ops-ingest-loop` as a long-lived Windows service.

.DESCRIPTION
  Intended for **Shawl** / **NSSM**. This script wraps `target/release/clmm-lp-cli.exe` (preferred)
  or falls back to `cargo run` if the exe is missing.

  Use this instead of Task Scheduler for 24/7 ingest:
  snapshots -> swaps sync -> enrich -> audit -> health-check, then sleep (+ jitter/backoff inside CLI).

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File tools/run_ops_ingest_loop.ps1

.EXAMPLE
  # Shawl:
  # shawl add --name clmm-ops-ingest-loop --cwd F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider -- powershell.exe -NoProfile -ExecutionPolicy Bypass -File F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider\tools\run_ops_ingest_loop.ps1
#>
[CmdletBinding()]
param(
  [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
  [string] $Configuration = "release",
  [int] $IntervalSecs = 900,
  [int] $JitterSecs = 60,
  [switch] $RunSnapshots = $true,
  [int] $SwapsMaxSignatures = 600,
  [int] $SwapsMaxPages = 2,
  [int] $EnrichMaxDecode = 160,
  [int] $HealthMaxAgeMinutes = 30,
  [double] $HealthMinDecodeOkPct = 65.0,
  [switch] $FailOnAlert = $true,
  # Optional: send Slack alert on non-zero exit (requires SLACK_WEBHOOK_URL in env or repo-root .env).
  [switch] $SlackOnError,
  # Throttle identical failure signatures (minutes).
  [int] $SlackThrottleMinutes = 30,
  [string] $StdoutLog = "data\\logs\\ops-ingest-loop-stdout.log",
  [string] $StderrLog = "data\\logs\\ops-ingest-loop-stderr.log"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Set-Location $RepoRoot

function Ensure-ParentDir([string] $path) {
  $p = Split-Path -Parent $path
  if ($p -and -not (Test-Path -LiteralPath $p)) { New-Item -ItemType Directory -Force -Path $p | Out-Null }
}

Ensure-ParentDir $StdoutLog
Ensure-ParentDir $StderrLog

function Send-SlackThrottled([string] $signature, [string] $text) {
  if (-not $SlackOnError) { return }
  $throttleDir = Join-Path $RepoRoot "data\\agent-alerts\\ops-ingest-loop-slack-throttle"
  New-Item -ItemType Directory -Force -Path $throttleDir | Out-Null
  $statePath = Join-Path $throttleDir "state.json"
  $now = [datetime]::UtcNow
  $send = $true
  if (Test-Path -LiteralPath $statePath) {
    try {
      $st = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
      $prevSig = [string]$st.sig
      $prevTs = $null
      try { $prevTs = [datetime]::Parse([string]$st.ts_utc).ToUniversalTime() } catch { $prevTs = $null }
      if ($prevSig -eq $signature -and $null -ne $prevTs) {
        $delta = ($now - $prevTs).TotalMinutes
        if ($delta -lt $SlackThrottleMinutes) { $send = $false }
      }
    } catch { $send = $true }
  }
  if (-not $send) { return }

  $notify = Join-Path $RepoRoot "tools\\notify_slack_webhook.ps1"
  if (-not (Test-Path -LiteralPath $notify)) { return }
  try {
    & $notify -Text $text
    @{ sig = $signature; ts_utc = $now.ToString("o") } | ConvertTo-Json | Out-File -FilePath $statePath -Encoding utf8
  } catch {
    # best-effort
  }
}

$exe = Join-Path $RepoRoot ("target\\{0}\\clmm-lp-cli.exe" -f $Configuration)
$useExe = Test-Path -LiteralPath $exe

$argv = @(
  "ops-ingest-loop",
  "--interval-secs", "$IntervalSecs",
  "--jitter-secs", "$JitterSecs",
  "--run-snapshots", (if ($RunSnapshots) { "true" } else { "false" }),
  "--swaps-max-signatures", "$SwapsMaxSignatures",
  "--swaps-max-pages", "$SwapsMaxPages",
  "--enrich-max-decode", "$EnrichMaxDecode",
  "--health-max-age-minutes", "$HealthMaxAgeMinutes",
  "--health-min-decode-ok-pct", "$HealthMinDecodeOkPct",
  "--fail-on-alert", (if ($FailOnAlert) { "true" } else { "false" })
)

Write-Host ("[{0}] run_ops_ingest_loop: RepoRoot={1}" -f (Get-Date -Format o), $RepoRoot)
Write-Host ("[{0}] stdout={1} stderr={2}" -f (Get-Date -Format o), $StdoutLog, $StderrLog)

if ($useExe) {
  Write-Host ("[{0}] exec: {1} {2}" -f (Get-Date -Format o), $exe, ($argv -join " "))
  & $exe @argv 1>> $StdoutLog 2>> $StderrLog
  $code = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
  if ($code -ne 0) {
    Send-SlackThrottled -signature ("exit_" + $code) -text ("[ops-ingest-loop] EXIT " + $code + " (see logs: " + $StdoutLog + " / " + $StderrLog + ")")
  }
  exit $code
} else {
  Write-Warning "Missing $exe; falling back to cargo run (slower). Build first: tools/build_clmm_lp_cli.ps1"
  $cargoArgs = @("run", "-q", "-p", "clmm-lp-cli", "--bin", "clmm-lp-cli", "--") + $argv
  Write-Host ("[{0}] exec: cargo {1}" -f (Get-Date -Format o), ($cargoArgs -join " "))
  & cargo @cargoArgs 1>> $StdoutLog 2>> $StderrLog
  $code = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
  if ($code -ne 0) {
    Send-SlackThrottled -signature ("exit_" + $code) -text ("[ops-ingest-loop] EXIT " + $code + " (cargo fallback; see logs: " + $StdoutLog + " / " + $StderrLog + ")")
  }
  exit $code
}

