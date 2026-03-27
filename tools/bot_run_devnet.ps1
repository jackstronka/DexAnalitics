param(
  [string]$Position = "",
  [string]$OpenBuildResponseJson = "",
  [switch]$Execute = $false,
  [int]$EvalIntervalSecs = 300,
  [int]$PollIntervalSecs = 30,
  [string]$OptimizeResultJson = "",
  [string]$Keypair = "",
  [switch]$SkipPreflight = $false,
  [string]$BotRunDir = "",
  [string]$IlLedgerPath = "",
  [string]$PositionFeeLedgerPath = "",
  [switch]$SkipLedger = $false
)

$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[bot-run] " + $msg) }

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

if (-not $SkipLedger) {
  if (-not [string]::IsNullOrWhiteSpace($BotRunDir)) {
    $null = New-Item -ItemType Directory -Force -Path $BotRunDir
    if ([string]::IsNullOrWhiteSpace($IlLedgerPath)) {
      $IlLedgerPath = Join-Path $BotRunDir "il_ledger.jsonl"
    }
    if ([string]::IsNullOrWhiteSpace($PositionFeeLedgerPath)) {
      $PositionFeeLedgerPath = Join-Path $BotRunDir "position_fee_ledger.jsonl"
    }
  }
  elseif ([string]::IsNullOrWhiteSpace($IlLedgerPath) -and [string]::IsNullOrWhiteSpace($PositionFeeLedgerPath)) {
    $ts = Get-Date -Format "yyyyMMdd-HHmmss"
    $BotRunDir = Join-Path $repoRoot "data\bot-runs\devnet\$ts"
    $null = New-Item -ItemType Directory -Force -Path $BotRunDir
    $IlLedgerPath = Join-Path $BotRunDir "il_ledger.jsonl"
    $PositionFeeLedgerPath = Join-Path $BotRunDir "position_fee_ledger.jsonl"
    Info ("Ledger run dir: " + $BotRunDir)
  }
}

if (-not $SkipPreflight) {
  Info "Running preflight"
  if ($Execute) {
    & (Join-Path $PSScriptRoot "bot_preflight.ps1") -RequireKeypair
  } else {
    & (Join-Path $PSScriptRoot "bot_preflight.ps1")
  }
}

if (-not [string]::IsNullOrWhiteSpace($Keypair)) {
  if (-not (Test-Path $Keypair)) {
    throw ("[bot-run] Keypair file does not exist: " + $Keypair)
  }
}

if ([string]::IsNullOrWhiteSpace($Position)) {
  if ([string]::IsNullOrWhiteSpace($OpenBuildResponseJson)) {
    throw "[bot-run] Provide -Position or -OpenBuildResponseJson."
  }
  if (-not (Test-Path $OpenBuildResponseJson)) {
    throw ("[bot-run] open-build response file does not exist: " + $OpenBuildResponseJson)
  }
  $openBuild = Get-Content -Raw -Path $OpenBuildResponseJson | ConvertFrom-Json
  if ($null -eq $openBuild.position_address -or [string]::IsNullOrWhiteSpace([string]$openBuild.position_address)) {
    throw ("[bot-run] Missing position_address in " + $OpenBuildResponseJson)
  }
  $Position = [string]$openBuild.position_address
  Info ("Resolved position from open-build response: " + $Position)
}

$args = @(
  "run", "--bin", "clmm-lp-cli", "--",
  "orca-bot-run",
  "--position", $Position,
  "--eval-interval-secs", $EvalIntervalSecs,
  "--poll-interval-secs", $PollIntervalSecs
)

if ($Execute) {
  $args += "--execute"
}

if (-not [string]::IsNullOrWhiteSpace($OptimizeResultJson)) {
  if (-not (Test-Path $OptimizeResultJson)) {
    throw ("[bot-run] optimize-result file does not exist: " + $OptimizeResultJson)
  }
  $args += @("--optimize-result-json", $OptimizeResultJson)
}

if (-not [string]::IsNullOrWhiteSpace($Keypair)) {
  $args += @("--keypair", $Keypair)
}

if (-not $SkipLedger) {
  if (-not [string]::IsNullOrWhiteSpace($IlLedgerPath)) {
    $args += @("--il-ledger-path", $IlLedgerPath)
  }
  if (-not [string]::IsNullOrWhiteSpace($PositionFeeLedgerPath)) {
    $args += @("--position-fee-ledger-path", $PositionFeeLedgerPath)
  }
}

$mode = if ($Execute) { "limited-live" } else { "dry-run" }
Info ("Starting orca-bot-run in mode: " + $mode)
Info ("Position: " + $Position)
Info ("Eval interval: " + $EvalIntervalSecs + "s, Poll interval: " + $PollIntervalSecs + "s")
if (-not $SkipLedger -and (-not [string]::IsNullOrWhiteSpace($IlLedgerPath) -or -not [string]::IsNullOrWhiteSpace($PositionFeeLedgerPath))) {
  if (-not [string]::IsNullOrWhiteSpace($IlLedgerPath)) { Info ("IL ledger: " + $IlLedgerPath) }
  if (-not [string]::IsNullOrWhiteSpace($PositionFeeLedgerPath)) { Info ("Position-fee ledger: " + $PositionFeeLedgerPath) }
}
Info "Press Ctrl+C to stop session"

& cargo @args
$exitCode = $LASTEXITCODE
if ($exitCode -ne 0) {
  throw ("[bot-run] cargo exited with code " + $exitCode)
}

Info "Bot session finished"
