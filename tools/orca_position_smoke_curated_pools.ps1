# Live mainnet smoke: open -> short sleep -> close for each **curated Orca pool** in STARTUP.md.
# Pool list: `tools/orca_curated_mainnet_pools.ps1` (same as STARTUP.md Orca section).
#
# Prereq: SOLANA_RPC_URL, funded wallet, SPL balances for each pair (SOL+USDC, SOL+whETH, cbBTC+USDC).
# Build release first for speed: .\tools\build_clmm_lp_cli.ps1  (or .\tools\build_clmm_lp_cli_release.ps1)
#
# Usage (repo root):
#   . .\tools\mainnet_rpc_env.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_position_smoke_curated_pools.ps1 -WhatIf
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_position_smoke_curated_pools.ps1 -Verify

param(
  [string] $Keypair = "",
  [UInt64] $SleepSecs = 3,
  [UInt64] $AmountA = 1000000,
  [UInt64] $AmountB = 1000,
  [UInt64] $ReserveSolLamports = 15000000,
  [switch] $AutoFund,
  [UInt32] $AutoFundMaxRounds = 8,
  [UInt16] $FundSwapSlippageBps = 150,
  [UInt32] $FundDeficitBufferBps = 100,
  [UInt16] $SlippageBps = 50,
  [UInt16] $CloseSlippageBps = 500,
  [double] $RangeWidthPct = 10,
  [switch] $CargoOnly,
  [switch] $SkipPreflight,
  [switch] $Verify,
  [switch] $Usd,
  [switch] $WhatIf
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "orca_curated_mainnet_pools.ps1")
$CuratedOrcaPools = @()
foreach ($cp in Get-OrcaCuratedMainnetPoolsAll) {
  $CuratedOrcaPools += @{ Label = $cp.Label; Address = $cp.Pool }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$quick = Join-Path $PSScriptRoot "orca_position_open_then_close_quick.ps1"

function Info([string]$m) { Write-Host ("[orca-smoke-curated] " + $m) }

if ($WhatIf) {
  Info "WhatIf: would run open+close for:"
  foreach ($p in $CuratedOrcaPools) {
    Info ("  - " + $p.Label + " " + $p.Address)
  }
  Info ("Script: " + $quick)
  exit 0
}

$commonArgs = @(
  "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $quick,
  "-SleepSecs", ([string]$SleepSecs),
  "-AmountA", ([string]$AmountA),
  "-AmountB", ([string]$AmountB),
  "-ReserveSolLamports", ([string]$ReserveSolLamports),
  "-SlippageBps", ([string]$SlippageBps),
  "-CloseSlippageBps", ([string]$CloseSlippageBps),
  "-RangeWidthPct", ([string]$RangeWidthPct)
)
if (-not [string]::IsNullOrWhiteSpace($Keypair)) {
  $commonArgs += @("-Keypair", $Keypair)
}
if ($CargoOnly.IsPresent) { $commonArgs += "-CargoOnly" }
if ($AutoFund.IsPresent) {
  $commonArgs += "-AutoFund"
  $commonArgs += @("-AutoFundMaxRounds", ([string]$AutoFundMaxRounds))
  $commonArgs += @("-FundSwapSlippageBps", ([string]$FundSwapSlippageBps))
  $commonArgs += @("-FundDeficitBufferBps", ([string]$FundDeficitBufferBps))
}
if ($SkipPreflight.IsPresent) { $commonArgs += "-SkipPreflight" }
if ($Verify.IsPresent) { $commonArgs += "-Verify" }
if ($Usd.IsPresent) { $commonArgs += "-Usd" }

foreach ($p in $CuratedOrcaPools) {
  Info "========== $($p.Label) =========="
  $poolArgs = $commonArgs + @("-Pool", $p.Address)
  & powershell.exe @poolArgs
  if ($LASTEXITCODE -ne 0) {
    throw ("[orca-smoke-curated] failed for pool " + $p.Address + " exit=" + $LASTEXITCODE)
  }
}

Info "All curated Orca pools completed OK."
