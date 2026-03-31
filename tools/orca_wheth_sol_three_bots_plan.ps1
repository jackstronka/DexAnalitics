# Plan: ~equal USD per bot on curated **whETH/SOL** (Hktf…), one wallet, three positions, reserve SOL for fees.
#
# Heuristic: each bot targets DeployUsd/NumBots notional; split 50/50 USD between SOL (mint A) and whETH (mint B)
# for choosing --amount-a / --amount-b caps (Orca uses them as max deposit legs; actual mix depends on price + ticks).
#
# Prereq: . .\tools\mainnet_rpc_env.ps1
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_wheth_sol_three_bots_plan.ps1 -Owner <PUBKEY>
#   powershell ... -File .\tools\orca_wheth_sol_three_bots_plan.ps1 -Keypair <path.json> -ReserveUsd 2 -DeployUsd 15 -NumBots 3
#
# Does not send txs — prints gaps + suggested `orca_swap_curated` / `orca_curated_rebalance` lines.

param(
  [string] $Owner = "",
  [string] $Keypair = "",
  [double] $ReserveUsd = 2.0,
  [double] $DeployUsd = 15.0,
  [int] $NumBots = 3,
  [UInt64] $ReserveSolLamports = 2500000,
  [UInt16] $RangeWidthPct = 10,
  [UInt16] $OpenSlippageBps = 100
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$m) { throw ("[wheth-3plan] " + $m) }

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

if (-not $env:SOLANA_RPC_URL) {
  $m = Join-Path $PSScriptRoot "mainnet_rpc_env.ps1"
  if (Test-Path $m) { . $m }
}
if (-not $env:SOLANA_RPC_URL) { Fail "Dot-source tools/mainnet_rpc_env.ps1" }

if ([string]::IsNullOrWhiteSpace($Owner)) {
  if ([string]::IsNullOrWhiteSpace($Keypair)) {
    $Keypair = if ($env:KEYPAIR_PATH) { $env:KEYPAIR_PATH } else { Join-Path $env:USERPROFILE ".config\solana\clmm_lp_bot_mainnet.json" }
  }
  if (-not (Test-Path $Keypair)) { Fail "Pass -Owner or valid -Keypair" }
  if (Get-Command solana-keygen -ErrorAction SilentlyContinue) {
    $Owner = (& solana-keygen pubkey $Keypair | Select-Object -First 1).Trim()
  } else {
    Fail "solana-keygen not found; pass -Owner explicitly"
  }
}

$PoolId = "WHETH_SOL"
$Pool = "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF"
$MintSol = "So11111111111111111111111111111111111111112"
$MintWheth = "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs"

$stateLine = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "solana_account_state.ps1") -Owner $Owner -Json
$st = ($stateLine | ConvertFrom-Json)
$lam = [decimal]$st.native_sol.lamports
$wSolAta = [decimal]0
$wheth = [decimal]0
$usdc = [decimal]0
foreach ($t in $st.spl_token_accounts) {
  if ([string]$t.mint -eq $MintSol) { $wSolAta += [decimal]$t.amount_raw }
  if ([string]$t.mint -eq $MintWheth) { $wheth += [decimal]$t.amount_raw }
  if ([string]$t.mint -eq "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v") { $usdc += [decimal]$t.amount_raw }
}
$solLamportsTotal = $lam + $wSolAta
$solUi = [double]$st.native_sol.sol
if ($wSolAta -gt 0) { $solUi += [double]([decimal]$wSolAta / [decimal]1000000000) }

$cg = Invoke-RestMethod -Uri "https://api.coingecko.com/api/v3/simple/price?ids=solana,ethereum&vs_currencies=usd" -TimeoutSec 25
$solUsd = [double]$cg.solana.usd
$ethUsd = [double]$cg.ethereum.usd
if ($solUsd -le 0 -or $ethUsd -le 0) { Fail "CoinGecko price fetch failed" }

$walletUsdEst = $solUi * $solUsd + ([double]$wheth / 1e8) * $ethUsd + ([double]$usdc / 1e6) * 1.0

$perBotUsd = $DeployUsd / [double]$NumBots
# 50/50 USD per leg per bot (planning only)
$usdSolLeg = $perBotUsd / 2.0
$usdEthLeg = $perBotUsd / 2.0
$lamPerBot = [UInt64][math]::Floor(($usdSolLeg / $solUsd) * 1e9)
$whethRawPerBot = [UInt64][math]::Floor(($usdEthLeg / $ethUsd) * 1e8)

$needSolLamportsTotal = $lamPerBot * [uint64]$NumBots
$needWhethRawTotal = $whethRawPerBot * [uint64]$NumBots

$availSolAfterReserve = $solLamportsTotal - [decimal]$ReserveSolLamports
if ($availSolAfterReserve -lt 0) { $availSolAfterReserve = [decimal]0 }

Write-Host ""
Write-Host "=== orca_wheth_sol_three_bots_plan (no txs) ==="
Write-Host ("Owner: " + $Owner)
Write-Host ("Pool:  " + $Pool + " (" + $PoolId + ")")
Write-Host ("CoinGecko: SOL USD=" + [math]::Round($solUsd, 4) + "  ETH USD=" + [math]::Round($ethUsd, 2))
Write-Host ("Rough wallet USD (SOL+wETH+USDC only): ~" + [math]::Round($walletUsdEst, 2))
Write-Host ""
Write-Host ("Target: ReserveSolLamports=" + $ReserveSolLamports + " (~$" + $ReserveUsd + " SOL at spot) ; DeployUsd=" + $DeployUsd + " ; NumBots=" + $NumBots)
Write-Host ("Per bot notional ~$" + [math]::Round($perBotUsd, 2) + " ; 50/50 USD legs -> AmountA (SOL raw lamports/bot)=" + $lamPerBot + " ; AmountB (whETH raw/bot)=" + $whethRawPerBot)
Write-Host ""
Write-Host ("Balances: SOL_total_lamports=" + [UInt64]$solLamportsTotal + "  whETH_raw=" + [UInt64]$wheth + "  USDC_raw=" + [UInt64]$usdc)
Write-Host ("After reserve, SOL lamports available for 3 opens: " + [UInt64]$availSolAfterReserve + " (need " + $needSolLamportsTotal + " for 3x caps)")
Write-Host ("whETH raw available: " + [UInt64]$wheth + " (need " + $needWhethRawTotal + " for 3x caps)")
$gapSol = [int64]$needSolLamportsTotal - [int64]$availSolAfterReserve
$gapW = [int64]$needWhethRawTotal - [int64]$wheth
if ($gapSol -gt 0) { Write-Host ("GAP SOL lamports (approx): +" + $gapSol + "  (~" + [math]::Round(($gapSol / 1e9) * $solUsd, 2) + " USD) -> swap USDC->SOL or other assets to SOL") }
if ($gapW -gt 0) { Write-Host ("GAP whETH raw: +" + $gapW + "  -> swap SOL->whETH on WHETH_SOL") }
if ($gapSol -le 0 -and $gapW -le 0) { Write-Host "Sufficient SOL (after reserve) and whETH for three equal caps (heuristic)." }

Write-Host ""
Write-Host "=== Suggested opens (same caps each; run 3x after funding). Use -OpenOnly via orca_position_open_then_close_quick or rebalance Action Open. ==="
$kpHint = if ($Keypair) { $Keypair } else { "<KEYPAIR_JSON>" }
Write-Host ("powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_curated_rebalance.ps1 ``")
Write-Host ("  -Action Open -Pair WHETH_SOL -AmountA " + $lamPerBot + " -AmountB " + $whethRawPerBot + " ``")
Write-Host ("  -RangeWidthPct " + $RangeWidthPct + " -SlippageBps " + $OpenSlippageBps + " -Keypair `"" + $kpHint + "`" -ReserveSolLamports " + $ReserveSolLamports)

Write-Host ""
Write-Host "=== After each open, copy `position PDA` then run (dry-run): ==="
Write-Host "cargo run --bin clmm-lp-cli -- orca-bot-run --position <POSITION_PDA> --eval-interval-secs 300 --poll-interval-secs 30"
Write-Host "# With --execute --keypair ... only when ready."

Write-Host ""
Write-Host "=== Swaps (examples; adjust raw from gaps) ==="
Write-Host "# USDC -> SOL (SOL_USDC):"
Write-Host ('.\tools\orca_swap_curated.ps1 -Pair SOL_USDC -From USDC -To SOL -SwapType exact-out -AmountRaw <USDC_RAW> -Keypair "' + $kpHint + '" -Execute')
Write-Host "# SOL -> whETH (WHETH_SOL):"
Write-Host ('.\tools\orca_swap_curated.ps1 -Pair WHETH_SOL -From SOL -To WHETH -SwapType exact-out -AmountRaw <WHETH_RAW_NEEDED> -Keypair "' + $kpHint + '" -Execute')
