<#
.SYNOPSIS
  Runs quick_verify_data.ps1; on failure (exit 2), posts summary to Slack (with throttle).

.DESCRIPTION
  One-shot. Schedule via Task Scheduler, cron, or tools/data_alerts_loop.ps1 (e.g. hourly).
  Forwards parameters to quick_verify_data.ps1. Throttle key = failing check buckets.

.EXAMPLE
  .\tools\quick_verify_alert.ps1

.EXAMPLE
  .\tools\quick_verify_alert.ps1 -SkipDecodeAudit -MinMinutesBetweenSameIssues 30
#>
[CmdletBinding()]
param(
  [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
  [int] $MinMinutesBetweenSameIssues = 60,
  [switch] $SkipSlack,

  # --- passed through to quick_verify_data.ps1 ---
  [int] $LimitPerProtocol = 0,
  [double] $MinDecodeOkPct = 65.0,
  [int] $HealthMaxAgeMinutes = 180,
  [int] $MaxAllowedHealthAlerts = 0,
  [switch] $SkipDecodeAudit,
  [string] $SolanaRpcUrl = "",
  [string] $SolanaRpcFallbackUrls = "",
  [string] $ExpectedCluster = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$verifyScript = Join-Path $RepoRoot "tools\quick_verify_data.ps1"
if (-not (Test-Path -LiteralPath $verifyScript)) {
  throw "Missing $verifyScript"
}

Set-Location $RepoRoot

$sigParts = @()
$capture = New-Object System.Collections.Generic.List[string]
$code = 0
$oldEap = $ErrorActionPreference
try {
  $ErrorActionPreference = "Continue"
  $out = & $verifyScript `
    -LimitPerProtocol $LimitPerProtocol `
    -MinDecodeOkPct $MinDecodeOkPct `
    -HealthMaxAgeMinutes $HealthMaxAgeMinutes `
    -MaxAllowedHealthAlerts $MaxAllowedHealthAlerts `
    -SkipDecodeAudit:$SkipDecodeAudit `
    -SolanaRpcUrl $SolanaRpcUrl `
    -SolanaRpcFallbackUrls $SolanaRpcFallbackUrls `
    -ExpectedCluster $ExpectedCluster 2>&1
  foreach ($line in @($out)) {
    $capture.Add("$line")
    Write-Host $line
  }
  $code = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
} catch {
  $capture.Add("FATAL: $_")
  Write-Host "FATAL: $_"
  $code = 1
  $sigParts += "fatal"
} finally {
  $ErrorActionPreference = $oldEap
}

$reportPath = $null
$fullText = $capture -join "`n"
if ($fullText -match '(?m)report:\s+(.+\.json)\s*$') {
  $reportPath = $Matches[1].Trim()
}

$detail = ""
if ($code -eq 0) {
  Write-Host "quick_verify_data: OK"
  exit 0
}

if ($sigParts -contains "fatal") {
  $detail = ($capture | Where-Object { $_ -match "^FATAL:" } | Select-Object -Last 1)
  if (-not $detail) { $detail = "quick_verify_data terminated with error (exit $code)" }
} elseif ($reportPath -and (Test-Path -LiteralPath $reportPath)) {
  try {
    $j = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    $c = $j.checks
    if (-not [bool]$c.snapshot_tier12_ok) { $sigParts += "tier12" }
    if (-not [bool]$c.health_ok) { $sigParts += "health" }
    if (-not [bool]$c.decode_ok) { $sigParts += "decode" }
    $detail = "tier12_ok=$($c.snapshot_tier12_ok) health_ok=$($c.health_ok) alerts=$($c.health_alerts) decode_ok=$($c.decode_ok) decode_pct=$($c.decode_ok_pct)"
  } catch {
    $sigParts += "unknown"
    $detail = "parse report failed: $_"
  }
} else {
  if ($fullText -match 'OVERALL GO:\s+(\S+)') {
    $detail = "OVERALL GO=$($Matches[1]) (no report path parsed)"
  } else {
    $detail = "quick_verify failed exit=$code"
  }
  if ($sigParts.Count -eq 0) { $sigParts += "fail" }
}

$issuesSig = ($sigParts | Sort-Object -Unique) -join ","
if ([string]::IsNullOrWhiteSpace($issuesSig)) {
  $issuesSig = "unknown"
}

if ($SkipSlack) {
  exit $code
}

$throttleDir = Join-Path $RepoRoot "data\agent-alerts\quick-verify-slack-throttle"
New-Item -ItemType Directory -Force -Path $throttleDir | Out-Null
$statePath = Join-Path $throttleDir "state.json"

$now = [datetime]::UtcNow
$send = $true
if (Test-Path -LiteralPath $statePath) {
  try {
    $st = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    $prevSig = [string]$st.issues_sig
    $prevTs = $null
    try { $prevTs = [datetime]::Parse([string]$st.ts_utc).ToUniversalTime() } catch { $prevTs = $null }
    if ($prevSig -eq $issuesSig -and $null -ne $prevTs -and $issuesSig -ne "") {
      $delta = ($now - $prevTs).TotalMinutes
      if ($delta -lt $MinMinutesBetweenSameIssues) {
        $send = $false
        Write-Host "Slack throttle: same failure signature within ${MinMinutesBetweenSameIssues}m; skip."
      }
    }
  } catch {
    $send = $true
  }
}

if ($send) {
  $msg = "[quick-verify] NOT OK (exit $code)`n$detail"
  if ($reportPath) { $msg += "`nreport: $reportPath" }
  $notify = Join-Path $RepoRoot "tools\notify_slack_webhook.ps1"
  & $notify -Text $msg
  @{
    issues_sig = $issuesSig
    ts_utc     = $now.ToString("o")
  } | ConvertTo-Json | Out-File -FilePath $statePath -Encoding utf8
}

exit $code
