<#
.SYNOPSIS
  Restart loop for `clmm-lp-cli orca-bot-run` (Windows / ops continuity).

.DESCRIPTION
  Runs the Orca bot; if the process exits with a non-zero code, waits and starts again.
  Exit code 0 is treated as intentional shutdown (no restart) unless -RestartOnCleanExit.

  Use with Task Scheduler (At startup) or NSSM. See doc/OPERATIONAL_CONTINUITY.md.

.EXAMPLE
  .\orca_bot_run_supervised.ps1 -- --position YOUR_NFT_PUBKEY --eval-interval-secs 300

.EXAMPLE
  .\orca_bot_run_supervised.ps1 -LogDir D:\logs\orca-bot -- --position YOUR_NFT --execute --keypair D:\sec\key.json
#>
[CmdletBinding()]
param(
  [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path,
  [int]$RestartDelaySecs = 15,
  [int]$MaxRestarts = 0,
  [string]$LogDir = '',
  # Optional: send Slack alerts on crashes/restart bursts (requires SLACK_WEBHOOK_URL in env or repo-root .env).
  [switch]$SlackOnError,
  # Throttle identical failure signatures (minutes).
  [int]$SlackThrottleMinutes = 15,
  # Send Slack when MaxRestarts is reached and we abort.
  [switch]$SlackOnMaxRestarts,
  [switch]$CargoOnly,
  [switch]$RestartOnCleanExit,
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$BotArgs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $BotArgs -or $BotArgs.Count -eq 0) {
  Write-Error "Pass `orca-bot-run` arguments after `--`. Example: .\orca_bot_run_supervised.ps1 -- --position <PUBKEY> --execute --keypair C:\path\key.json"
  exit 2
}

. (Join-Path $PSScriptRoot 'clmm_rpc_tools_helpers.ps1')
[void](Initialize-ClmmToolsRpcEnv)

$notifyScript = Join-Path $RepoRoot "tools\notify_slack_webhook.ps1"
function Send-SlackThrottled([string] $signature, [string] $text) {
  if (-not $SlackOnError) { return }
  if (-not (Test-Path -LiteralPath $notifyScript)) { return }

  $throttleDir = Join-Path $RepoRoot "data\agent-alerts\orca-bot-supervised-slack-throttle"
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
  try {
    & $notifyScript -Text $text
    @{ sig = $signature; ts_utc = $now.ToString("o") } | ConvertTo-Json | Out-File -FilePath $statePath -Encoding utf8
  } catch {
    # best-effort
  }
}

$exe = if (-not $CargoOnly) { Resolve-ClmmLpCliExe $RepoRoot } else { $null }
if (-not $CargoOnly -and -not $exe) {
  Write-Warning "No release/debug clmm-lp-cli.exe under target\; using cargo run (slower). Use tools/build_clmm_lp_cli.ps1 or -CargoOnly."
}

$restartCount = 0
while ($true) {
  if ($MaxRestarts -gt 0 -and $restartCount -ge $MaxRestarts) {
    Write-Error "MaxRestarts ($MaxRestarts) reached; aborting."
    if ($SlackOnMaxRestarts) {
      Send-SlackThrottled -signature ("max_restarts_" + $MaxRestarts) -text ("[orca-bot] MaxRestarts reached (" + $MaxRestarts + "); aborting supervised loop.")
    }
    exit 1
  }

  $stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
  $argv = @('orca-bot-run') + $BotArgs
  Write-Host "=== orca_bot_run_supervised: cycle $restartCount at $stamp ==="

  if ($LogDir -and $PSVersionTable.PSVersion.Major -lt 7) {
    Write-Warning "-LogDir with Tee-Object: use PowerShell 7+ for reliable `$LASTEXITCODE after native exe; otherwise omit -LogDir or check logs manually."
  }

  $oldEap = $ErrorActionPreference
  try {
    $ErrorActionPreference = 'Continue'
    if ($exe -and -not $CargoOnly) {
      if ($LogDir -and -not [string]::IsNullOrWhiteSpace($LogDir)) {
        New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
        $logFile = Join-Path $LogDir "orca-bot-$stamp.log"
        Write-Host "Logging to $logFile"
        & $exe @argv *>&1 | Tee-Object -FilePath $logFile
      } else {
        & $exe @argv
      }
    } else {
      $cargoArgs = @('run', '-p', 'clmm-lp-cli', '--bin', 'clmm-lp-cli', '--') + $argv
      if ($LogDir -and -not [string]::IsNullOrWhiteSpace($LogDir)) {
        New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
        $logFile = Join-Path $LogDir "orca-bot-$stamp.log"
        Write-Host "Logging to $logFile"
        & cargo @cargoArgs *>&1 | Tee-Object -FilePath $logFile
      } else {
        & cargo @cargoArgs
      }
    }
    $code = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { -1 }
  } finally {
    $ErrorActionPreference = $oldEap
  }

  Write-Host "=== orca-bot-run exited with code $code ==="

  if ($code -ne 0) {
    $logHint = ""
    if ($LogDir -and -not [string]::IsNullOrWhiteSpace($LogDir)) {
      $logHint = " log=" + $logFile
    }
    Send-SlackThrottled -signature ("exit_" + $code) -text ("[orca-bot] crashed exit=" + $code + " cycle=" + $restartCount + $logHint)
  }

  if ($code -eq 0 -and -not $RestartOnCleanExit) {
    exit 0
  }

  if ($code -eq 0 -and $RestartOnCleanExit) {
    Write-Host "RestartOnCleanExit: sleeping $RestartDelaySecs s..."
    Start-Sleep -Seconds $RestartDelaySecs
    $restartCount++
    continue
  }

  $restartCount++
  Write-Host "Sleeping $RestartDelaySecs s before restart (attempt $restartCount)..."
  Start-Sleep -Seconds $RestartDelaySecs
}
