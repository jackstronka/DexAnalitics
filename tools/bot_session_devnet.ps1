param(
  [string]$Position = "",
  [string]$OpenBuildResponseJson = "",
  [switch]$Execute = $false,
  [int]$EvalIntervalSecs = 300,
  [int]$PollIntervalSecs = 30,
  [int]$MaxRuntimeMinutes = 0,
  [string]$OptimizeResultJson = "",
  [string]$Keypair = "",
  [string]$Notes = "",
  [string]$Signatures = "",
  [switch]$SkipPreflight = $false,
  [string]$BotRunDir = "",
  [string]$IlLedgerPath = "",
  [string]$PositionFeeLedgerPath = "",
  [switch]$SkipLedger = $false
)

$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[bot-session] " + $msg) }
function QuoteArg([string]$v) {
  if ($null -eq $v) { return '""' }
  return '"' + ($v -replace '"', '\"') + '"'
}
function ResolvePosition([string]$positionArg, [string]$openBuildJsonPath) {
  if (-not [string]::IsNullOrWhiteSpace($positionArg)) {
    return $positionArg
  }
  if ([string]::IsNullOrWhiteSpace($openBuildJsonPath)) {
    throw "[bot-session] Provide -Position or -OpenBuildResponseJson."
  }
  if (-not (Test-Path $openBuildJsonPath)) {
    throw ("[bot-session] open-build response file does not exist: " + $openBuildJsonPath)
  }
  $openBuild = Get-Content -Raw -Path $openBuildJsonPath | ConvertFrom-Json
  if ($null -eq $openBuild.position_address -or [string]::IsNullOrWhiteSpace([string]$openBuild.position_address)) {
    throw ("[bot-session] Missing position_address in " + $openBuildJsonPath)
  }
  return [string]$openBuild.position_address
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$Position = ResolvePosition -positionArg $Position -openBuildJsonPath $OpenBuildResponseJson

$mode = if ($Execute) { "limited-live" } else { "dry-run" }
Info ("Starting session in mode: " + $mode)
Info ("Position: " + $Position)
if ($MaxRuntimeMinutes -gt 0) {
  Info ("Max runtime: " + $MaxRuntimeMinutes + " minutes")
}

$runFailed = $false
$runTimedOut = $false
try {
  if ($MaxRuntimeMinutes -le 0) {
    $runArgs = @{
      EvalIntervalSecs = $EvalIntervalSecs
      PollIntervalSecs = $PollIntervalSecs
    }
    if (-not [string]::IsNullOrWhiteSpace($Position)) {
      $runArgs.Position = $Position
    }
    if (-not [string]::IsNullOrWhiteSpace($OpenBuildResponseJson)) {
      $runArgs.OpenBuildResponseJson = $OpenBuildResponseJson
    }

    if ($Execute) { $runArgs.Execute = $true }
    if (-not [string]::IsNullOrWhiteSpace($OptimizeResultJson)) { $runArgs.OptimizeResultJson = $OptimizeResultJson }
    if (-not [string]::IsNullOrWhiteSpace($Keypair)) { $runArgs.Keypair = $Keypair }
    if ($SkipPreflight) { $runArgs.SkipPreflight = $true }
    if (-not [string]::IsNullOrWhiteSpace($BotRunDir)) { $runArgs.BotRunDir = $BotRunDir }
    if (-not [string]::IsNullOrWhiteSpace($IlLedgerPath)) { $runArgs.IlLedgerPath = $IlLedgerPath }
    if (-not [string]::IsNullOrWhiteSpace($PositionFeeLedgerPath)) { $runArgs.PositionFeeLedgerPath = $PositionFeeLedgerPath }
    if ($SkipLedger) { $runArgs.SkipLedger = $true }

    & (Join-Path $PSScriptRoot "bot_run_devnet.ps1") @runArgs
  } else {
    $runScriptPath = Join-Path $PSScriptRoot "bot_run_devnet.ps1"
    $argList = @(
      "-NoProfile",
      "-ExecutionPolicy", "Bypass",
      "-File", (QuoteArg $runScriptPath),
      "-EvalIntervalSecs", $EvalIntervalSecs,
      "-PollIntervalSecs", $PollIntervalSecs
    )
    if (-not [string]::IsNullOrWhiteSpace($Position)) {
      $argList += @("-Position", (QuoteArg $Position))
    }
    if (-not [string]::IsNullOrWhiteSpace($OpenBuildResponseJson)) {
      $argList += @("-OpenBuildResponseJson", (QuoteArg $OpenBuildResponseJson))
    }

    if ($Execute) { $argList += "-Execute" }
    if ($SkipPreflight) { $argList += "-SkipPreflight" }
    if (-not [string]::IsNullOrWhiteSpace($OptimizeResultJson)) {
      $argList += @("-OptimizeResultJson", (QuoteArg $OptimizeResultJson))
    }
    if (-not [string]::IsNullOrWhiteSpace($Keypair)) {
      $argList += @("-Keypair", (QuoteArg $Keypair))
    }
    if (-not [string]::IsNullOrWhiteSpace($BotRunDir)) {
      $argList += @("-BotRunDir", (QuoteArg $BotRunDir))
    }
    if (-not [string]::IsNullOrWhiteSpace($IlLedgerPath)) {
      $argList += @("-IlLedgerPath", (QuoteArg $IlLedgerPath))
    }
    if (-not [string]::IsNullOrWhiteSpace($PositionFeeLedgerPath)) {
      $argList += @("-PositionFeeLedgerPath", (QuoteArg $PositionFeeLedgerPath))
    }
    if ($SkipLedger) { $argList += "-SkipLedger" }

    $joinedArgs = $argList -join " "
    $proc = Start-Process -FilePath "powershell" -ArgumentList $joinedArgs -PassThru -NoNewWindow

    $timeoutSec = $MaxRuntimeMinutes * 60
    Wait-Process -Id $proc.Id -Timeout $timeoutSec -ErrorAction SilentlyContinue
    $proc.Refresh()
    if (-not $proc.HasExited) {
      $runTimedOut = $true
      Write-Warning ("[bot-session] Max runtime reached, stopping bot process (pid=" + $proc.Id + ")")
      Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    } else {
      $exitCode = 0
      try {
        $exitCode = [int]$proc.ExitCode
      } catch {
        $exitCode = 1
      }
      if ($exitCode -ne 0) {
        throw ("bot_run_devnet exited with code " + $exitCode)
      }
    }
  }
} catch {
  $runFailed = $true
  Write-Warning ("[bot-session] bot_run_devnet failed: " + $_.Exception.Message)
}

$sessionNotes = if ([string]::IsNullOrWhiteSpace($Notes)) { "" } else { $Notes.Trim() }
if ($runFailed) {
  if ($sessionNotes) {
    $sessionNotes = $sessionNotes + " | run_status=failed"
  } else {
    $sessionNotes = "run_status=failed"
  }
} elseif ($runTimedOut) {
  if ($sessionNotes) {
    $sessionNotes = $sessionNotes + " | run_status=timeout"
  } else {
    $sessionNotes = "run_status=timeout"
  }
} else {
  if ($sessionNotes) {
    $sessionNotes = $sessionNotes + " | run_status=ok"
  } else {
    $sessionNotes = "run_status=ok"
  }
}

Info "Writing post-run report"
& (Join-Path $PSScriptRoot "bot_postrun_report.ps1") `
  -Position $Position `
  -Mode $mode `
  -EvalIntervalSecs $EvalIntervalSecs `
  -PollIntervalSecs $PollIntervalSecs `
  -Signatures $Signatures `
  -Notes $sessionNotes

if ($runFailed) {
  throw "[bot-session] Session finished with bot run failure (report saved)."
}
if ($runTimedOut) {
  throw "[bot-session] Session finished due to max runtime timeout (report saved)."
}

Info "Session finished successfully"
