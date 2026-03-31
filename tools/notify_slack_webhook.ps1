<#
.SYNOPSIS
  Send a one-off message to Slack via Incoming Webhook (no Slack SDK).

.DESCRIPTION
  Reads webhook URL from -WebhookUrl or environment variable SLACK_WEBHOOK_URL.
  Keep the URL secret (same sensitivity as a password). Do not commit it.

.EXAMPLE
  $env:SLACK_WEBHOOK_URL = 'https://hooks.slack.com/services/...'
  .\notify_slack_webhook.ps1 -Text 'orca-bot: supervised loop exceeded MaxRestarts'

.EXAMPLE
  .\notify_slack_webhook.ps1 -WebhookUrl $env:SLACK_WEBHOOK_URL -Text 'Test' -Username 'clmm-lp'

.EXAMPLE
  # With SLACK_WEBHOOK_URL in repo-root .env (no env var needed):
  .\tools\notify_slack_webhook.ps1 -Text 'test'
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$Text,
  [string]$WebhookUrl = $env:SLACK_WEBHOOK_URL,
  [string]$Username = '',
  # Optional: read SLACK_WEBHOOK_URL from this file if -WebhookUrl / env still empty (default: repo-root .env).
  [string]$DotEnvPath = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Read-SlackWebhookFromDotEnv {
  param([Parameter(Mandatory)][string]$Path)
  if (-not (Test-Path -LiteralPath $Path)) {
    return $null
  }
  foreach ($line in Get-Content -LiteralPath $Path) {
    $t = $line.Trim()
    if ($t -match '^\s*#' -or $t -eq '') {
      continue
    }
    if ($t -notmatch '^\s*SLACK_WEBHOOK_URL\s*=\s*(.+)$') {
      continue
    }
    $v = $matches[1].Trim()
    if (
      ($v.Length -ge 2) -and (
        (($v.StartsWith('"')) -and ($v.EndsWith('"'))) -or
        (($v.StartsWith("'")) -and ($v.EndsWith("'")))
      )
    ) {
      $v = $v.Substring(1, $v.Length - 2).Trim()
    }
    if (-not [string]::IsNullOrWhiteSpace($v)) {
      return $v
    }
  }
  return $null
}

if ([string]::IsNullOrWhiteSpace($WebhookUrl)) {
  $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
  $envFile = if ($DotEnvPath) { $DotEnvPath } else { Join-Path $repoRoot '.env' }
  $fromFile = Read-SlackWebhookFromDotEnv -Path $envFile
  if (-not [string]::IsNullOrWhiteSpace($fromFile)) {
    $WebhookUrl = $fromFile
  }
}

if ([string]::IsNullOrWhiteSpace($WebhookUrl)) {
  Write-Error @"
Set SLACK_WEBHOOK_URL, add a line SLACK_WEBHOOK_URL=https://... to repo-root .env, or pass -WebhookUrl.
Order: 1) -WebhookUrl  2) env SLACK_WEBHOOK_URL  3) .env at $(Join-Path (Resolve-Path (Join-Path $PSScriptRoot '..')).Path '.env')
"@
  exit 2
}

$payload = [ordered]@{ text = $Text }
if (-not [string]::IsNullOrWhiteSpace($Username)) {
  $payload['username'] = $Username
}
$json = ($payload | ConvertTo-Json -Compress)
$bytes = [System.Text.Encoding]::UTF8.GetBytes($json)

try {
  Invoke-RestMethod -Uri $WebhookUrl -Method Post -Body $bytes -ContentType 'application/json; charset=utf-8'
} catch {
  Write-Error "Slack webhook POST failed: $_"
  exit 1
}
