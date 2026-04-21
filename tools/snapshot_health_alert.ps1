<#
.SYNOPSIS
  Runs snapshot_health_check.ps1; on failure, posts to Slack (with throttle).

.DESCRIPTION
  One-shot check (exit 0/1).   For periodic runs use **Task Scheduler** (see `tools/register_snapshot_health_scheduled_task.ps1`), cron, **or**
  the long-lived tools/data_alerts_loop.ps1 (Shawl/NSSM) — see doc/OPERATIONAL_CONTINUITY.md.
  Uses notify_slack_webhook.ps1 (repo-root .env SLACK_WEBHOOK_URL or env).

.PARAMETER MinMinutesBetweenSameIssues
  If the same comma-sorted issue list was already reported within this window, skip Slack.

.EXAMPLE
  .\tools\snapshot_health_alert.ps1
#>
[CmdletBinding()]
param(
  [string] $RepoRoot = "",
  [int] $MinMinutesBetweenSameIssues = 15,
  [int] $MaxAgeMinutes10m = 25,
  [int] $MaxAgeMinutes5m = 15,
  [int] $MaxHeartbeatAgeMinutes10m = 22,
  [int] $MaxHeartbeatAgeMinutes5m = 12,
  [int] $ExpectOrcaTarget = 4,
  [switch] $SkipSlack
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
  $scriptDir = $null
  if ($PSScriptRoot) {
    $scriptDir = $PSScriptRoot
  } elseif ($MyInvocation.MyCommand.Path) {
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
  }

  if ($scriptDir) {
    $RepoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
  } else {
    $RepoRoot = (Get-Location).Path
  }
}

Set-Location $RepoRoot

$checkScript = Join-Path $RepoRoot "tools\snapshot_health_check.ps1"
if (-not (Test-Path -LiteralPath $checkScript)) {
  throw "Missing $checkScript"
}

$capture = New-Object System.Collections.Generic.List[string]
$oldEap = $ErrorActionPreference
try {
  $ErrorActionPreference = "Continue"
  $out = & $checkScript `
    -RepoRoot $RepoRoot `
    -MaxAgeMinutes10m $MaxAgeMinutes10m `
    -MaxAgeMinutes5m $MaxAgeMinutes5m `
    -MaxHeartbeatAgeMinutes10m $MaxHeartbeatAgeMinutes10m `
    -MaxHeartbeatAgeMinutes5m $MaxHeartbeatAgeMinutes5m `
    -ExpectOrcaTarget $ExpectOrcaTarget 2>&1
} finally {
  $ErrorActionPreference = $oldEap
}
foreach ($line in @($out)) {
  $capture.Add("$line")
}
$code = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
$text = ($capture -join "`n")

$throttleDir = Join-Path $RepoRoot "data\agent-alerts\snapshot-slack-throttle"
New-Item -ItemType Directory -Force -Path $throttleDir | Out-Null
$statePath = Join-Path $throttleDir "state.json"
$notify = Join-Path $RepoRoot "tools\notify_slack_webhook.ps1"

$prevSig = ""
$prevTs = $null
$prevOk = $true
if (Test-Path -LiteralPath $statePath) {
  try {
    $st = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    $prevSig = [string]$st.issues_sig
    if ($null -ne $st.ok) { $prevOk = [bool]$st.ok }
    try { $prevTs = [datetime]::Parse([string]$st.ts_utc).ToUniversalTime() } catch { $prevTs = $null }
  } catch {
    $prevSig = ""
    $prevTs = $null
    $prevOk = $true
  }
}

$issuesSig = ""
if ($text -match "NOT OK snapshot health:\s*issues=([^\r\n]+)") {
  $raw = $Matches[1].Trim()
  $parts = $raw -split "," | ForEach-Object { $_.Trim() } | Where-Object { $_ } | Sort-Object
  $issuesSig = $parts -join ","
}

if ($code -eq 0) {
  if (-not $SkipSlack -and -not $prevOk) {
    & $notify -Text "[snapshot-health] RECOVERY: collectors healthy again."
  }
  @{
    ok        = $true
    issues_sig = ""
    ts_utc    = ([datetime]::UtcNow).ToString("o")
  } | ConvertTo-Json | Out-File -FilePath $statePath -Encoding utf8
  Write-Host "snapshot_health_check: OK"
  exit 0
}

Write-Host $text
Write-Host "snapshot_health_check: NOT OK (exit=$code)"

if ($SkipSlack) {
  @{
    ok        = $false
    issues_sig = $issuesSig
    ts_utc    = ([datetime]::UtcNow).ToString("o")
  } | ConvertTo-Json | Out-File -FilePath $statePath -Encoding utf8
  exit $code
}

$now = [datetime]::UtcNow
$send = $true
if ($prevSig -eq $issuesSig -and $null -ne $prevTs -and -not $prevOk) {
  $delta = ($now - $prevTs).TotalMinutes
  if ($delta -lt $MinMinutesBetweenSameIssues) {
    $send = $false
    Write-Host "Slack throttle: same issues within ${MinMinutesBetweenSameIssues}m; skip."
  }
}

if ($send) {
  $msg = "[snapshot-health] NOT OK (exit $code)`nissues: $(if ($issuesSig) { $issuesSig } else { '(parse stdout or see snapshot-health.jsonl)' })"
  & $notify -Text $msg
}

@{
  ok        = $false
  issues_sig = $issuesSig
  ts_utc    = $now.ToString("o")
} | ConvertTo-Json | Out-File -FilePath $statePath -Encoding utf8

exit $code
