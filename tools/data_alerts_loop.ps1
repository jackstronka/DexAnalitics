<#
.SYNOPSIS
  Long-lived loop: snapshot_health_alert + quick_verify_alert on intervals (no Task Scheduler).

.DESCRIPTION
  Run once at boot under **Shawl**, **NSSM**, or a dedicated terminal. One process sleeps between
  checks instead of OS cron/Task Scheduler triggers.

  Logs to data/snapshot_logs/data-alerts-loop.log

.PARAMETER SnapshotIntervalSeconds
  Minimum wall time between snapshot_health_alert runs (default 600 = 10 min).

.PARAMETER QuickVerifyIntervalSeconds
  Minimum wall time between quick_verify_alert runs (default 3600 = 1 h).

.PARAMETER SkipQuickVerify
  Only run snapshot_health_alert.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\data_alerts_loop.ps1

.EXAMPLE
  # Shawl (see doc/OPERATIONAL_CONTINUITY.md)
  shawl add --name clmm-data-alerts --cwd F:\CLMM-Liquidity-Provider -- powershell.exe -NoProfile -ExecutionPolicy Bypass -File F:\CLMM-Liquidity-Provider\tools\data_alerts_loop.ps1
#>
[CmdletBinding()]
param(
  [string] $RepoRoot = "",
  [int] $SnapshotIntervalSeconds = 600,
  [int] $QuickVerifyIntervalSeconds = 3600,
  [switch] $SkipQuickVerify,
  [switch] $SkipSlack,
  # Forwarded to snapshot_health_alert
  [int] $SnapshotThrottleMinutes = 15,
  [int] $MaxAgeMinutes10m = 25,
  [int] $MaxAgeMinutes5m = 15,
  [int] $MaxHeartbeatAgeMinutes10m = 22,
  [int] $MaxHeartbeatAgeMinutes5m = 12,
  [int] $ExpectOrcaTarget = 4,
  # Forwarded to quick_verify_alert
  [int] $QuickVerifyThrottleMinutes = 60,
  [switch] $QuickVerifySkipDecodeAudit
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
    # Last resort keeps script runnable in unusual hosts where script metadata is unavailable.
    $RepoRoot = (Get-Location).Path
  }
}

Set-Location $RepoRoot

$logDir = Join-Path $RepoRoot "data\snapshot_logs"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$logFile = Join-Path $logDir "data-alerts-loop.log"
$mutexName = "Global\clmm-lp-data-alerts-loop"
$loopMutex = $null
$hasMutex = $false

function Write-Log([string] $msg) {
  $line = "{0} {1}" -f (Get-Date -Format "o"), $msg
  Add-Content -Path $logFile -Value $line
  Write-Host $line
}

$snapScript = Join-Path $RepoRoot "tools\snapshot_health_alert.ps1"
$qvScript = Join-Path $RepoRoot "tools\quick_verify_alert.ps1"
if (-not (Test-Path -LiteralPath $snapScript)) { throw "Missing $snapScript" }
if (-not $SkipQuickVerify -and -not (Test-Path -LiteralPath $qvScript)) { throw "Missing $qvScript" }

try {
  $loopMutex = New-Object System.Threading.Mutex($false, $mutexName)
  $hasMutex = $loopMutex.WaitOne(0, $false)
  if (-not $hasMutex) {
    Write-Log ("lock already held -> start refused ({0})" -f $mutexName)
    exit 0
  }
} catch {
  Write-Log ("ERROR acquiring lock {0}: {1}" -f $mutexName, $_)
  throw
}

Write-Log ("data_alerts_loop start: snapshot every {0}s, quick_verify every {1}s, SkipQuickVerify={2}, lock={3}" -f $SnapshotIntervalSeconds, $QuickVerifyIntervalSeconds, $SkipQuickVerify, $mutexName)

try {
  $lastQuickVerify = [datetime]::MinValue

  while ($true) {
    $iterStart = Get-Date

    try {
      Write-Log "run snapshot_health_alert"
      $snapArgs = @{
        RepoRoot                   = $RepoRoot
        MinMinutesBetweenSameIssues = $SnapshotThrottleMinutes
        MaxAgeMinutes10m            = $MaxAgeMinutes10m
        MaxAgeMinutes5m           = $MaxAgeMinutes5m
        MaxHeartbeatAgeMinutes10m = $MaxHeartbeatAgeMinutes10m
        MaxHeartbeatAgeMinutes5m  = $MaxHeartbeatAgeMinutes5m
        ExpectOrcaTarget            = $ExpectOrcaTarget
      }
      if ($SkipSlack) { $snapArgs.SkipSlack = $true }
      & $snapScript @snapArgs
      Write-Log ("snapshot_health_alert exit {0}" -f $LASTEXITCODE)
    } catch {
      Write-Log ("snapshot_health_alert ERROR: {0}" -f $_)
    }

    if (-not $SkipQuickVerify) {
      $elapsedSinceQv = ($iterStart - $lastQuickVerify).TotalSeconds
      if ($elapsedSinceQv -ge $QuickVerifyIntervalSeconds) {
        try {
          Write-Log "run quick_verify_alert"
          $qvArgs = @{
            RepoRoot                      = $RepoRoot
            MinMinutesBetweenSameIssues   = $QuickVerifyThrottleMinutes
          }
          if ($SkipSlack) { $qvArgs.SkipSlack = $true }
          if ($QuickVerifySkipDecodeAudit) { $qvArgs.SkipDecodeAudit = $true }
          & $qvScript @qvArgs
          Write-Log ("quick_verify_alert exit {0}" -f $LASTEXITCODE)
          $lastQuickVerify = Get-Date
        } catch {
          Write-Log ("quick_verify_alert ERROR: {0}" -f $_)
        }
      }
    }

    $elapsed = ((Get-Date) - $iterStart).TotalSeconds
    $sleep = [Math]::Max(5, $SnapshotIntervalSeconds - [int][Math]::Ceiling($elapsed))
    Write-Log ("sleep {0}s" -f $sleep)
    Start-Sleep -Seconds $sleep
  }
} finally {
  if ($loopMutex -and $hasMutex) {
    try {
      $loopMutex.ReleaseMutex() | Out-Null
      Write-Log "lock released"
    } catch {
      Write-Log ("ERROR releasing lock: {0}" -f $_)
    }
  }
  if ($loopMutex) {
    $loopMutex.Dispose()
  }
}
