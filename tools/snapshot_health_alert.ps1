<#
.SYNOPSIS
  Runs snapshot_health_check.ps1; on failure, posts to Slack (with throttle).

.DESCRIPTION
  One-shot check (exit 0/1). For periodic runs use Task Scheduler, cron, **or**
  the long-lived tools/data_alerts_loop.ps1 (Shawl/NSSM) — see doc/OPERATIONAL_CONTINUITY.md.
  Uses notify_slack_webhook.ps1 (repo-root .env SLACK_WEBHOOK_URL or env).

.PARAMETER MinMinutesBetweenSameIssues
  If the same comma-sorted issue list was already reported within this window, skip Slack.

.EXAMPLE
  .\tools\snapshot_health_alert.ps1
#>
[CmdletBinding()]
param(
  [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
  [int] $MinMinutesBetweenSameIssues = 15,
  [int] $MaxAgeMinutes10m = 25,
  [int] $MaxAgeMinutes5m = 15,
  [int] $ExpectOrcaTarget = 3,
  [switch] $SkipSlack
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

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
    -ExpectOrcaTarget $ExpectOrcaTarget 2>&1
} finally {
  $ErrorActionPreference = $oldEap
}
foreach ($line in @($out)) {
  $capture.Add("$line")
}
$code = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
$text = ($capture -join "`n")

$issuesSig = ""
if ($text -match "NOT OK snapshot health:\s*issues=([^\r\n]+)") {
  $raw = $Matches[1].Trim()
  $parts = $raw -split "," | ForEach-Object { $_.Trim() } | Where-Object { $_ } | Sort-Object
  $issuesSig = $parts -join ","
}

if ($code -eq 0) {
  Write-Host "snapshot_health_check: OK"
  exit 0
}

Write-Host $text
Write-Host "snapshot_health_check: NOT OK (exit=$code)"

if ($SkipSlack) {
  exit $code
}

$throttleDir = Join-Path $RepoRoot "data\agent-alerts\snapshot-slack-throttle"
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
    if ($prevSig -eq $issuesSig -and $null -ne $prevTs) {
      $delta = ($now - $prevTs).TotalMinutes
      if ($delta -lt $MinMinutesBetweenSameIssues) {
        $send = $false
        Write-Host "Slack throttle: same issues within ${MinMinutesBetweenSameIssues}m; skip."
      }
    }
  } catch {
    $send = $true
  }
}

if ($send) {
  $msg = "[snapshot-health] NOT OK (exit $code)`nissues: $(if ($issuesSig) { $issuesSig } else { '(parse stdout or see snapshot-health.jsonl)' })"
  $notify = Join-Path $RepoRoot "tools\notify_slack_webhook.ps1"
  & $notify -Text $msg
  @{
    issues_sig = $issuesSig
    ts_utc     = $now.ToString("o")
  } | ConvertTo-Json | Out-File -FilePath $statePath -Encoding utf8
}

exit $code
