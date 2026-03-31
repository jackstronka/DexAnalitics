# Curated Orca mainnet (3 pairs): one entry point for swaps, preflight, open, close, fund (cbBTC/USDC), smoke.
# Pool IDs and mints: `tools/orca_curated_mainnet_pools.ps1` (SOL_USDC, WHETH_SOL, CBBTC_USDC).
#
# Usage (repo root):
#   . .\tools\mainnet_rpc_env.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_curated_rebalance.ps1 -Action Help
#   powershell ... -File .\tools\orca_curated_rebalance.ps1 -Action ListPairs
#   powershell ... -File .\tools\orca_curated_rebalance.ps1 -Action Preflight -Pair SOL_USDC -AmountA 1000000 -AmountB 1000
#   powershell ... -File .\tools\orca_curated_rebalance.ps1 -Action Open -Pair CBBTC_USDC -AmountA 2000 -AmountB 200000
#   powershell ... -File .\tools\orca_curated_rebalance.ps1 -Action Close -Position <POSITION_PDA>
#   powershell ... -File .\tools\orca_curated_rebalance.ps1 -Action Swap -Pair SOL_USDC -From SOL -To USDC -SwapType exact-in -AmountRaw 1000000 -Execute
#   powershell ... -File .\tools\orca_curated_rebalance.ps1 -Action FundCbBtc -AmountA 2000 -AmountB 200000 -FundExecute
#   powershell ... -File .\tools\orca_curated_rebalance.ps1 -Action Smoke
#
# Actions:
#   Help      — this overview + pointers to underlying scripts
#   ListPairs — same as `orca_swap_curated.ps1 -ListPairs`
#   Preflight — `orca_position_open_preflight.ps1` for -Pair pool
#   Open      — `orca_position_open_then_close_quick.ps1 -OpenOnly` (live position; no auto-close)
#   Close     — `orca-position-close` (needs -Position PDA from list or explorer)
#   Swap      — forwards to `orca_swap_curated.ps1` (any leg / exact-in / exact-out)
#   FundCbBtc — `orca_fund_cbbtc_usdc_open.ps1` (SOL/USDC + cbBTC/USDC swaps toward open on CBBTC_USDC only)
#   Smoke     — `orca_position_smoke_curated_pools.ps1` (open+close each curated pool; needs balances)

param(
  [ValidateSet("Help", "ListPairs", "Preflight", "Open", "Close", "Swap", "FundCbBtc", "Smoke")]
  [string] $Action = "Help",

  [ValidateSet("SOL_USDC", "WHETH_SOL", "CBBTC_USDC", "")]
  [string] $Pair = "",

  # Close
  [string] $Position = "",

  # Preflight / Open / FundCbBtc
  [UInt64] $AmountA = 0,
  [UInt64] $AmountB = 0,
  [UInt64] $ReserveSolLamports = 15000000,

  # Open / Smoke
  [double] $RangeWidthPct = 10,
  [UInt16] $SlippageBps = 50,
  [UInt16] $CloseSlippageBps = 500,
  [UInt64] $SleepSecs = 5,
  [switch] $AutoFund,
  [UInt32] $AutoFundMaxRounds = 8,
  [UInt16] $FundSwapSlippageBps = 150,
  [UInt32] $FundDeficitBufferBps = 100,
  [switch] $SkipPreflight,
  [switch] $Verify,
  [switch] $Usd,
  [switch] $CargoOnly,

  # Swap (delegates to orca_swap_curated.ps1)
  [string] $From = "",
  [string] $To = "",
  [ValidateSet("exact-in", "exact-out")]
  [string] $SwapType = "exact-in",
  [UInt64] $AmountRaw = 0,
  [UInt16] $SwapSlippageBps = 150,
  [switch] $Execute,

  # FundCbBtc
  [switch] $FundExecute,
  [UInt32] $CbBtcBufferBps = 100,
  [UInt32] $UsdcHeadroomBps = 600,
  [UInt32] $PostSwapTopUpMaxRounds = 6,

  [string] $Keypair = "",
  [string] $Owner = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Info([string]$m) { Write-Host ("[orca-curated-rebalance] " + $m) }
function Fail([string]$m) { throw ("[orca-curated-rebalance] " + $m) }

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

. (Join-Path $PSScriptRoot "orca_curated_mainnet_pools.ps1")

if ([string]::IsNullOrWhiteSpace($Keypair)) {
  if ($env:KEYPAIR_PATH -and -not [string]::IsNullOrWhiteSpace($env:KEYPAIR_PATH)) { $Keypair = $env:KEYPAIR_PATH }
  elseif ($env:SOLANA_KEYPAIR_PATH -and -not [string]::IsNullOrWhiteSpace($env:SOLANA_KEYPAIR_PATH)) { $Keypair = $env:SOLANA_KEYPAIR_PATH }
  else { $Keypair = Join-Path $env:USERPROFILE ".config\solana\clmm_lp_bot_mainnet.json" }
}

function Show-Help {
  Write-Host @"
Curated Orca mainnet - 3 pairs (see tools/orca_curated_mainnet_pools.ps1):
  SOL_USDC    SOL/USDC
  WHETH_SOL   whETH/SOL
  CBBTC_USDC  cbBTC/USDC

Actions (this script):
  -Action ListPairs          List pools, mints, swap examples
  -Action Preflight -Pair ID -AmountA -AmountB
  -Action Open -Pair ID -AmountA -AmountB [-AutoFund] [-SkipPreflight]  (live open only; use -Close separately)
  -Action Close -Position <PDA>  (full close; optional -CloseSlippageBps)
  -Action Swap -Pair ID -From .. -To .. -SwapType exact-in|exact-out -AmountRaw N [-Execute]
  -Action FundCbBtc -AmountA -AmountB [-FundExecute]  (plan or execute swaps toward cbBTC/USDC open)
  -Action Smoke              open+close on each pool (orca_position_smoke_curated_pools.ps1)

Underlying scripts (direct use):
  tools/orca_swap_curated.ps1              - swaps for any curated pair/direction
  tools/orca_swap.ps1                      - low-level; pool + mint + swap type
  tools/orca_position_open_preflight.ps1   - balances vs deposit
  tools/orca_position_open_then_close_quick.ps1 - -OpenOnly adds live position without close
  tools/orca_fund_cbbtc_usdc_open.ps1      - fund wallet for cbBTC/USDC position
  tools/orca_position_smoke_curated_pools.ps1 - regression: all 3 pools

Build CLI: tools/build_clmm_lp_cli.ps1
"@
}

if ($Action -eq "Help") {
  Show-Help
  exit 0
}

if ($Action -eq "ListPairs") {
  $sc = Join-Path $PSScriptRoot "orca_swap_curated.ps1"
  & $sc -ListPairs
  exit $LASTEXITCODE
}

if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  $mainnetEnv = Join-Path $PSScriptRoot "mainnet_rpc_env.ps1"
  if (Test-Path -LiteralPath $mainnetEnv) {
    Info ("SOLANA_RPC_URL not set; dot-sourcing " + $mainnetEnv)
    . $mainnetEnv
  }
}
if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  Fail "Set SOLANA_RPC_URL (tools/mainnet_rpc_env.ps1)."
}

if ($Action -eq "Preflight") {
  if ([string]::IsNullOrWhiteSpace($Pair)) { Fail "Preflight requires -Pair SOL_USDC|WHETH_SOL|CBBTC_USDC" }
  if ($AmountA -eq 0 -or $AmountB -eq 0) { Fail "Preflight requires -AmountA and -AmountB > 0" }
  $pe = Get-OrcaCuratedMainnetPoolById $Pair
  $pf = Join-Path $PSScriptRoot "orca_position_open_preflight.ps1"
  $pfArgs = @{
    Pool               = $pe.Pool
    Keypair            = $Keypair
    Owner              = $Owner
    AmountA            = $AmountA
    AmountB            = $AmountB
    ReserveSolLamports = $ReserveSolLamports
  }
  if ($CargoOnly.IsPresent) { $pfArgs.CargoOnly = $true }
  & $pf @pfArgs
  exit $LASTEXITCODE
}

if ($Action -eq "Open") {
  if ([string]::IsNullOrWhiteSpace($Pair)) { Fail "Open requires -Pair" }
  if ($AmountA -eq 0 -or $AmountB -eq 0) { Fail "Open requires -AmountA and -AmountB > 0" }
  $pe = Get-OrcaCuratedMainnetPoolById $Pair
  $quick = Join-Path $PSScriptRoot "orca_position_open_then_close_quick.ps1"
  $qa = @{
    Pool               = $pe.Pool
    Keypair            = $Keypair
    Owner              = $Owner
    RangeWidthPct      = $RangeWidthPct
    AmountA            = $AmountA
    AmountB            = $AmountB
    SlippageBps        = $SlippageBps
    CloseSlippageBps   = $CloseSlippageBps
    SleepSecs          = $SleepSecs
    ReserveSolLamports = $ReserveSolLamports
    OpenOnly           = $true
  }
  if ($AutoFund.IsPresent) {
    $qa.AutoFund = $true
    $qa.AutoFundMaxRounds = $AutoFundMaxRounds
    $qa.FundSwapSlippageBps = $FundSwapSlippageBps
    $qa.FundDeficitBufferBps = $FundDeficitBufferBps
  }
  if ($CargoOnly.IsPresent) { $qa.CargoOnly = $true }
  if ($SkipPreflight.IsPresent) { $qa.SkipPreflight = $true }
  & $quick @qa
  exit $LASTEXITCODE
}

if ($Action -eq "Close") {
  if ([string]::IsNullOrWhiteSpace($Position)) { Fail "Close requires -Position <Whirlpool position PDA>" }
  . (Join-Path $PSScriptRoot "clmm_rpc_tools_helpers.ps1")
  $null = Initialize-ClmmToolsRpcEnv
  $argv = @(
    "orca-position-close",
    "--position", $Position,
    "--keypair", $Keypair,
    "--slippage-bps", ([string]$CloseSlippageBps)
  )
  Invoke-ClmmLpCliCapture -RepoRoot $repoRoot -PreferReleaseExe (-not $CargoOnly.IsPresent) -Argv $argv -StepLabel "orca-position-close"
  exit 0
}

if ($Action -eq "Swap") {
  if ([string]::IsNullOrWhiteSpace($Pair)) { Fail "Swap requires -Pair" }
  if ([string]::IsNullOrWhiteSpace($From) -or [string]::IsNullOrWhiteSpace($To)) { Fail "Swap requires -From and -To (see -Action ListPairs)" }
  if ($AmountRaw -eq 0) { Fail "Swap requires -AmountRaw > 0" }
  if ([string]::IsNullOrWhiteSpace($SwapType)) { Fail "Swap requires -SwapType exact-in|exact-out" }
  $sc = Join-Path $PSScriptRoot "orca_swap_curated.ps1"
  $sa = @{
    Pair          = $Pair
    From          = $From
    To            = $To
    SwapType      = $SwapType
    AmountRaw     = $AmountRaw
    Keypair       = $Keypair
    Owner         = $Owner
    SlippageBps   = $SwapSlippageBps
  }
  if ($Execute.IsPresent) { $sa.Execute = $true }
  if ($CargoOnly.IsPresent) { $sa.CargoOnly = $true }
  & $sc @sa
  exit $LASTEXITCODE
}

if ($Action -eq "FundCbBtc") {
  if ($AmountA -eq 0 -or $AmountB -eq 0) { Fail "FundCbBtc requires -AmountA and -AmountB > 0 (cbBTC/USDC open targets)" }
  $fs = Join-Path $PSScriptRoot "orca_fund_cbbtc_usdc_open.ps1"
  $fa = @{
    AmountA              = $AmountA
    AmountB              = $AmountB
    ReserveSolLamports   = $ReserveSolLamports
    Keypair              = $Keypair
    Owner                = $Owner
    SlippageBps          = $SwapSlippageBps
    CbBtcBufferBps       = $CbBtcBufferBps
    UsdcHeadroomBps      = $UsdcHeadroomBps
    PostSwapTopUpMaxRounds = $PostSwapTopUpMaxRounds
  }
  if ($FundExecute.IsPresent) { $fa.Execute = $true }
  if ($CargoOnly.IsPresent) { $fa.CargoOnly = $true }
  & $fs @fa
  exit $LASTEXITCODE
}

if ($Action -eq "Smoke") {
  $sm = Join-Path $PSScriptRoot "orca_position_smoke_curated_pools.ps1"
  $smA = if ($AmountA -gt 0) { $AmountA } else { [UInt64]1000000 }
  $smB = if ($AmountB -gt 0) { $AmountB } else { [UInt64]1000 }
  $smArgs = @{
    Keypair              = $Keypair
    SleepSecs            = $SleepSecs
    AmountA              = $smA
    AmountB              = $smB
    ReserveSolLamports   = $ReserveSolLamports
    SlippageBps          = $SlippageBps
    CloseSlippageBps     = $CloseSlippageBps
    RangeWidthPct        = $RangeWidthPct
  }
  if ($AutoFund.IsPresent) {
    $smArgs.AutoFund = $true
    $smArgs.AutoFundMaxRounds = $AutoFundMaxRounds
    $smArgs.FundSwapSlippageBps = $FundSwapSlippageBps
    $smArgs.FundDeficitBufferBps = $FundDeficitBufferBps
  }
  if ($CargoOnly.IsPresent) { $smArgs.CargoOnly = $true }
  if ($SkipPreflight.IsPresent) { $smArgs.SkipPreflight = $true }
  if ($Verify.IsPresent) { $smArgs.Verify = $true }
  if ($Usd.IsPresent) { $smArgs.Usd = $true }
  & $sm @smArgs
  exit $LASTEXITCODE
}

Fail "Unhandled action."
