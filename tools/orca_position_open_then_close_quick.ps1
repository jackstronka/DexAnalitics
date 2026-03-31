# Quick open -> close a Whirlpool position (small amounts).
#
# Flow:
# 1) `orca-position-open` (live tx) with `--range-width-pct` and small `--amount-a/--amount-b`
# 2) parse `position PDA:` from CLI output
# 3) sleep `sleep_secs`
# 4) `orca-position-close --position <PDA>`
# 5) (optional) verify `orca-positions-list --owner <OWNER> => entries: 0`
#
# Usage (repo root):
#   .\tools\orca_position_open_then_close_quick.ps1 -Pool <WHIRLPOOL_POOL> -Keypair <path-to-wallet.json>
#
# Defaults (based on previous examples):
#   Pool:    Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE
#   Range:   10% band (symmetric)
#   AmountA: 1_000_000 (base units)
#   AmountB: 1_000      (base units)
#   Open slippage: 50 bps; close: 500 bps (override -CloseSlippageBps)
#   Sleep:    5 seconds
#   If target\release\clmm-lp-cli.exe exists, it is used (faster than cargo run). -CargoOnly forces cargo.
#   Before open: runs tools/orca_position_open_preflight.ps1 unless -SkipPreflight (-ReserveSolLamports, default 15M lamports).
#   Optional: -AutoFund runs on-pool exact-out swaps (tools/orca_position_preflight_core.ps1) until preflight passes, then preflight + open.
#   -OpenOnly: send open tx only (skip sleep, close, verify). Use for live positions / rebalance; close via tools/orca_curated_rebalance.ps1 -Action Close or orca-position-close.
#

param(
  [string] $Pool = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE",
  [string] $Keypair = "",
  [string] $Owner = "",
  [double] $RangeWidthPct = 10,
  [UInt64] $AmountA = 1000000,
  [UInt64] $AmountB = 1000,
  [UInt16] $SlippageBps = 50,
  [UInt16] $CloseSlippageBps = 500,
  [UInt64] $SleepSecs = 5,
  # Native SOL (lamports) kept aside for fees/rent; not counted as wSOL deposit in preflight.
  [UInt64] $ReserveSolLamports = 15000000,
  [switch] $AutoFund,
  [UInt32] $AutoFundMaxRounds = 8,
  [UInt16] $FundSwapSlippageBps = 150,
  [UInt32] $FundDeficitBufferBps = 100,
  [switch] $CargoOnly,
  [switch] $SkipPreflight,
  [switch] $Verify,
  [switch] $Usd,
  [switch] $OpenOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[orca-open-close-quick] " + $msg) }
function Fail([string]$msg) { throw ("[orca-open-close-quick] " + $msg) }

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  $mainnetEnv = Join-Path $PSScriptRoot "mainnet_rpc_env.ps1"
  if (Test-Path -LiteralPath $mainnetEnv) {
    Info ("SOLANA_RPC_URL not set; dot-sourcing " + $mainnetEnv)
    . $mainnetEnv
  }
}
if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  Fail "Missing SOLANA_RPC_URL. Dot-source tools/mainnet_rpc_env.ps1 first or export SOLANA_RPC_URL."
}

. (Join-Path $PSScriptRoot "clmm_rpc_tools_helpers.ps1")
if (Initialize-ClmmToolsRpcEnv) {
  Info "Default CLMM_RPC_DENYLIST=ankr,projectserum (mainnet fallbacks). Set CLMM_RPC_DENYLIST yourself to override."
}

if ([string]::IsNullOrWhiteSpace($Keypair)) {
  if ($env:KEYPAIR_PATH -and -not [string]::IsNullOrWhiteSpace($env:KEYPAIR_PATH)) {
    $Keypair = $env:KEYPAIR_PATH
  } elseif ($env:SOLANA_KEYPAIR_PATH -and -not [string]::IsNullOrWhiteSpace($env:SOLANA_KEYPAIR_PATH)) {
    $Keypair = $env:SOLANA_KEYPAIR_PATH
  } else {
    $Keypair = Join-Path $env:USERPROFILE ".config\solana\clmm_lp_bot_mainnet.json"
  }
}
if (-not (Test-Path -LiteralPath $Keypair)) {
  Fail ("Keypair file not found: " + $Keypair)
}

function ResolveOwnerFromKeypair([string]$kpPath) {
  if (Get-Command "solana-keygen" -ErrorAction SilentlyContinue) {
    return (& solana-keygen pubkey $kpPath | Select-Object -First 1).Trim()
  }

  # Fallback: derive pubkey from keypair JSON (array of 64 bytes: secret[32] + pubkey[32])
  $kpJson = Get-Content -LiteralPath $kpPath -Raw | ConvertFrom-Json
  if ($null -eq $kpJson) { throw "Could not parse keypair JSON at $kpPath" }
  $bytes = [byte[]]($kpJson | ForEach-Object { [byte]$_ })
  if ($bytes.Length -lt 64) { throw "Keypair JSON too short (expected 64 bytes), len=" + $bytes.Length }
  $pubkeyBytes = $bytes[32..63]

  function Base58Encode([byte[]] $data) {
    $alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".ToCharArray()
    if ($null -eq $data) { return "" }

    $zeroCount = 0
    for ($i = 0; $i -lt $data.Length; $i++) {
      if ($data[$i] -ne 0) { break }
      $zeroCount++
    }

    $digits = New-Object System.Collections.Generic.List[int]

    for ($i = 0; $i -lt $data.Length; $i++) {
      $carry = [int]$data[$i]
      for ($j = 0; $j -lt $digits.Count; $j++) {
        $carry += $digits[$j] * 256
        $digits[$j] = [int]($carry % 58)
        $carry = [int]([Math]::Floor($carry / 58))
      }
      while ($carry -gt 0) {
        $digits.Add([int]($carry % 58))
        $carry = [int]([Math]::Floor($carry / 58))
      }
    }

    $sb = New-Object System.Text.StringBuilder
    for ($i = 0; $i -lt $zeroCount; $i++) { [void]$sb.Append('1') }
    for ($i = $digits.Count - 1; $i -ge 0; $i--) { [void]$sb.Append($alphabet[$digits[$i]]) }

    if ($sb.Length -eq 0) { return "" }
    return $sb.ToString()
  }

  return (Base58Encode $pubkeyBytes)
}

if ([string]::IsNullOrWhiteSpace($Owner)) {
  $Owner = ResolveOwnerFromKeypair $Keypair
}
if ([string]::IsNullOrWhiteSpace($Owner)) {
  Fail "Could not resolve owner pubkey."
}

Info ("RPC=" + $env:SOLANA_RPC_URL)
Info ("Pool=" + $Pool)
Info ("Keypair=" + $Keypair)
Info ("Owner=" + $Owner)
$preferExe = -not $CargoOnly.IsPresent
$cliExeResolved = Resolve-ClmmLpCliExe $repoRoot
$cliMode = if ($CargoOnly.IsPresent) { "cargo run (forced)" } elseif (-not [string]::IsNullOrWhiteSpace($cliExeResolved)) { $cliExeResolved } else { "cargo run (no target\\*\\clmm-lp-cli.exe)" }
Info ("range_width_pct=" + $RangeWidthPct + " amount_a=" + $AmountA + " amount_b=" + $AmountB + " open_slippage_bps=" + $SlippageBps + " close_slippage_bps=" + $CloseSlippageBps + " cli=" + $cliMode)

if ($AutoFund.IsPresent) {
  Info ("Auto-fund: exact-out swaps on pool (max_rounds=" + $AutoFundMaxRounds + " swap_slippage_bps=" + $FundSwapSlippageBps + " deficit_buffer_bps=" + $FundDeficitBufferBps + ")...")
  . (Join-Path $PSScriptRoot "orca_position_preflight_core.ps1")
  try {
    Invoke-OrcaPositionAutoFundFromPool -Pool $Pool -RepoRoot $repoRoot -Owner $Owner -Keypair $Keypair `
      -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
      -PreferReleaseExe $preferExe -MaxRounds $AutoFundMaxRounds -SwapSlippageBps $FundSwapSlippageBps `
      -DeficitBufferBps $FundDeficitBufferBps -Quiet:$false
  } catch {
    Fail $_.Exception.Message
  }
}

if (-not $SkipPreflight.IsPresent) {
  Info "Running position-open preflight (orca-pool-read + wallet JSON)..."
  $preflightPath = Join-Path $PSScriptRoot "orca_position_open_preflight.ps1"
  $pfArgs = @{
    Pool               = $Pool
    Keypair            = $Keypair
    Owner              = $Owner
    AmountA            = $AmountA
    AmountB            = $AmountB
    ReserveSolLamports = $ReserveSolLamports
  }
  if ($CargoOnly.IsPresent) { $pfArgs.CargoOnly = $true }
  & $preflightPath @pfArgs
  if ($LASTEXITCODE -ne 0) {
    Fail ("Position open preflight failed (exit " + $LASTEXITCODE + "). Fund wallet or swap (tools/orca_swap.ps1); or pass -SkipPreflight if you accept blind open.")
  }
} else {
  Info "Skipping position-open preflight (-SkipPreflight)."
}

# ---- Helpers: measure + extract ledger costs ----
function Find-LedgerRowBySignature([string] $LedgerPath, [string] $Signature) {
  if (-not (Test-Path -LiteralPath $LedgerPath)) { return $null }
  $lines = Get-Content -LiteralPath $LedgerPath
  for ($i = $lines.Count - 1; $i -ge 0; $i--) {
    $line = $lines[$i]
    if ($null -eq $line) { continue }
    # Avoid regex quoting issues; do a simple substring match.
    $needle = '"signature":"' + $Signature + '"'
    if ($line.IndexOf($needle, [System.StringComparison]::Ordinal) -lt 0) { continue }
    return ($line | ConvertFrom-Json)
  }
  return $null
}

function Resolve-LedgerPath([string] $RepoRoot) {
  if ($env:CLMM_POSITION_LIFECYCLE_LEDGER_PATH -and -not [string]::IsNullOrWhiteSpace($env:CLMM_POSITION_LIFECYCLE_LEDGER_PATH)) {
    return $env:CLMM_POSITION_LIFECYCLE_LEDGER_PATH
  }
  if ($env:CLMM_POSITION_OPEN_LEDGER_PATH -and -not [string]::IsNullOrWhiteSpace($env:CLMM_POSITION_OPEN_LEDGER_PATH)) {
    return $env:CLMM_POSITION_OPEN_LEDGER_PATH
  }
  return (Join-Path $RepoRoot "data\\ledger\\orca_position_lifecycle.jsonl")
}

$swTotal = [System.Diagnostics.Stopwatch]::StartNew()
$openConfirmedAtMs = $null
$closeConfirmedAtMs = $null

# ---- 1) open (live) ----
$openArgv = @(
  "orca-position-open",
  "--pool", $Pool,
  "--keypair", $Keypair,
  "--range-width-pct", ([string]$RangeWidthPct),
  "--amount-a", ([string]$AmountA),
  "--amount-b", ([string]$AmountB),
  "--slippage-bps", ([string]$SlippageBps)
)

Info "Opening position (live tx)..."
$swTotal.Restart()
$state = @{
  positionPda = $null
  openSig = $null
  openConfirmedAtMs = $null
}

try {
  Invoke-ClmmLpCliStream -RepoRoot $repoRoot -PreferReleaseExe $preferExe -Argv $openArgv -StepLabel "orca-position-open" -OnLine {
    param($line)
    if ($null -eq $state.positionPda -and $line -match "position PDA:\s*([1-9A-HJ-NP-Za-km-z]{32,})") {
      $state.positionPda = $Matches[1]
    }
    if ($null -eq $state.openSig -and $line -match "signature:\s*([1-9A-HJ-NP-Za-km-z]{80,})") {
      $state.openSig = $Matches[1]
      if ($null -eq $state.openConfirmedAtMs) {
        $state.openConfirmedAtMs = [int]$swTotal.ElapsedMilliseconds
      }
    }
  }
} catch {
  Fail $_.Exception.Message
}

$positionPda = $state.positionPda
$openSig = $state.openSig
$openConfirmedAtMs = $state.openConfirmedAtMs

if ([string]::IsNullOrWhiteSpace($positionPda)) {
  Fail "Could not parse position PDA from open output."
}
Info ("Opened position PDA=" + $positionPda)
if (-not [string]::IsNullOrWhiteSpace($openSig)) {
  Info ("Open signature=" + $openSig)
}

if ($OpenOnly.IsPresent) {
  $swTotal.Stop()
  Info "Open-only: skipping close/sleep/verify (position remains on-chain)."
  Write-Host ""
  Write-Host "Close later:"
  Write-Host ("  .\\tools\\orca_curated_rebalance.ps1 -Action Close -Position " + $positionPda + " -Keypair <path>")
  Write-Host ("  clmm-lp-cli orca-position-close --position " + $positionPda + " --keypair <path> --slippage-bps " + $CloseSlippageBps)
  exit 0
}

# ---- 2) wait ----
if ($SleepSecs -gt 0) {
  Info ("Sleeping " + $SleepSecs + "s before close...")
  Start-Sleep -Seconds $SleepSecs
}

# ---- 3) close ----
$closeArgv = @(
  "orca-position-close",
  "--position", $positionPda,
  "--keypair", $Keypair,
  "--slippage-bps", ([string]$CloseSlippageBps)
)

Info "Closing position..."
$closeState = @{
  closeSig = $null
  closeConfirmedAtMs = $null
}

try {
  Invoke-ClmmLpCliStream -RepoRoot $repoRoot -PreferReleaseExe $preferExe -Argv $closeArgv -StepLabel "orca-position-close" -OnLine {
    param($line)
    if ($null -eq $closeState.closeSig -and $line -match "signature:\s*([1-9A-HJ-NP-Za-km-z]{80,})") {
      $closeState.closeSig = $Matches[1]
      if ($null -eq $closeState.closeConfirmedAtMs) {
        $closeState.closeConfirmedAtMs = [int]$swTotal.ElapsedMilliseconds
      }
    }
  }
} catch {
  Fail $_.Exception.Message
}

$closeSig = $closeState.closeSig
$closeConfirmedAtMs = $closeState.closeConfirmedAtMs
if (-not [string]::IsNullOrWhiteSpace($closeSig)) {
  Info ("Close signature=" + $closeSig)
}

$swTotal.Stop()
if ($null -ne $openConfirmedAtMs -and $null -ne $closeConfirmedAtMs) {
  $confirmToConfirmMs = $closeConfirmedAtMs - $openConfirmedAtMs
  Info ("Timing (confirm->confirm): open=" + $openConfirmedAtMs + "ms close_confirm=" + $closeConfirmedAtMs + "ms confirm_to_confirm=" + $confirmToConfirmMs + "ms total=" + [int]$swTotal.ElapsedMilliseconds + "ms")
} else {
  Info ("Timing: total=" + [int]$swTotal.ElapsedMilliseconds + "ms (could not parse confirm timestamps)")
}

# ---- 5) report cost from ledger ----
$ledgerPath = Resolve-LedgerPath $repoRoot
$openRow = if (-not [string]::IsNullOrWhiteSpace($openSig)) { Find-LedgerRowBySignature $ledgerPath $openSig } else { $null }
$closeRow = if (-not [string]::IsNullOrWhiteSpace($closeSig)) { Find-LedgerRowBySignature $ledgerPath $closeSig } else { $null }

if ($null -ne $openRow -and $null -ne $closeRow) {
  $openFee = [UInt64]$openRow.tx_fee_lamports
  $closeFee = [UInt64]$closeRow.tx_fee_lamports
  $netDeltaOpen = if ($null -ne $openRow.fee_payer_net_lamports_delta) { [Int64]$openRow.fee_payer_net_lamports_delta } else { 0 }
  $netDeltaClose = if ($null -ne $closeRow.fee_payer_net_lamports_delta) { [Int64]$closeRow.fee_payer_net_lamports_delta } else { 0 }

  $totalFeeLamports = $openFee + $closeFee
  $totalFeeSol = [double]$totalFeeLamports / 1000000000.0
  $totalNetDeltaLamports = $netDeltaOpen + $netDeltaClose
  $totalNetDeltaSol = [double]$totalNetDeltaLamports / 1000000000.0

  Write-Host ""
  Write-Host "=== Cost estimate (from ledger) ==="
  Write-Host ("Ledger path: " + $ledgerPath)
  Write-Host ("Open fee lamports:   " + $openFee)
  Write-Host ("Close fee lamports:  " + $closeFee)
  Write-Host ("Total network fee:   " + $totalFeeLamports + " lamports (" + $totalFeeSol + " SOL)")
  Write-Host ("Net SOL delta (fee payer, post-pre sum): " + $totalNetDeltaLamports + " lamports (" + $totalNetDeltaSol + " SOL)")

  # token deltas on close (best-effort)
  if ($null -ne $closeRow.token_b_net_delta_raw) {
    Write-Host ("Close token_b delta: " + $closeRow.token_b_net_delta_raw + " base units (" + $closeRow.token_b_net_delta_ui + " UI)")
  }
  if ($null -ne $closeRow.token_a_net_delta_raw) {
    Write-Host ("Close token_a delta: " + $closeRow.token_a_net_delta_raw + " base units (" + $closeRow.token_a_net_delta_ui + " UI)")
  }

  if ($Usd) {
    # Best-effort SOL/USD price from CoinGecko (current spot).
    try {
      $solUsd = (Invoke-RestMethod -Uri "https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd" -TimeoutSec 15).solana.usd
      $feeUsd = $totalFeeSol * [double]$solUsd
      $netUsd = $totalNetDeltaSol * [double]$solUsd
      Write-Host ("SOL/USD (CoinGecko):  " + $solUsd)
      Write-Host ("Total fee in USD:     " + ([math]::Round($feeUsd, 6)))
      Write-Host ("Net delta in USD:     " + ([math]::Round($netUsd, 6)))
    } catch {
      Write-Host "USD conversion skipped (CoinGecko fetch failed)."
    }
  }
} else {
  Write-Host ""
  Write-Host "Ledger cost report skipped (could not find open/close rows by signature)."
}

# ---- 4) verify ----
if ($Verify) {
  Info "Verifying on-chain entries=0..."
  $verifyArgv = @(
    "orca-positions-list",
    "--owner", $Owner,
    "--keypair", $Keypair
  )
  try {
    $null = Invoke-ClmmLpCliCapture -RepoRoot $repoRoot -PreferReleaseExe $preferExe -Argv $verifyArgv -StepLabel "orca-positions-list"
  } catch {
    Fail $_.Exception.Message
  }
}

