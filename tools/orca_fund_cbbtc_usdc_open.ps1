# Estimate (orca-swap --dry-run quotes) and optionally execute swaps to fund wallet for
# Orca open on **cbBTC/USDC** pool (HxA6...) at given -AmountA (cbBTC raw) / -AmountB (USDC raw).
#
# Plan:
# 1) Preflight: deficits on cbBTC (mint_a) and USDC (mint_b).
# 2) If cbBTC short: quote exact-out cbBTC on same pool -> token_est_in = USDC needed (incl. fee path).
# 3) Total USDC need = USDC deficit + USDC for cbBTC leg; if short vs wallet: quote SOL/USDC exact-out USDC -> SOL (lamports).
# 4) With -Execute: run SOL_USDC then CBBTC_USDC swaps in that order (refresh not automatic; re-run if second leg fails).
#
# Requires: SOLANA_RPC_URL, keypair. Does not open a position (only balances).
#
# Usage:
#   . .\tools\mainnet_rpc_env.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_fund_cbbtc_usdc_open.ps1 `
#     -AmountA 10000 -AmountB 1000000 -Keypair "$env:USERPROFILE\.config\solana\clmm_lp_bot_mainnet.json"
#   ... same + -Execute   # live swaps

param(
  [Parameter(Mandatory = $true)][UInt64] $AmountA,
  [Parameter(Mandatory = $true)][UInt64] $AmountB,
  [UInt64] $ReserveSolLamports = 15000000,
  [string] $Keypair = "",
  [string] $Owner = "",
  [UInt16] $SlippageBps = 150,
  # Extra on cbBTC exact-out amount (deficit buffer).
  [UInt32] $CbBtcBufferBps = 100,
  # Extra on USDC gap before SOL->USDC exact-out (initial plan + each top-up round).
  [UInt32] $UsdcHeadroomBps = 600,
  # After planned swaps, retry SOL->USDC and/or USDC->cbBTC until preflight passes (handles quote vs on-chain drift).
  [UInt32] $PostSwapTopUpMaxRounds = 6,
  [switch] $Execute,
  [switch] $CargoOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Info([string]$m) { Write-Host ("[fund-cbbtc-usdc] " + $m) }
function Fail([string]$m) { throw ("[fund-cbbtc-usdc] " + $m) }

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

$PoolCbBtc = "HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM"
$PoolSolUsdc = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"
$MintCbBtc = "cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij"
$MintUsdc = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
$MintWsol = "So11111111111111111111111111111111111111112"

if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  $m = Join-Path $PSScriptRoot "mainnet_rpc_env.ps1"
  if (Test-Path -LiteralPath $m) { . $m }
}
if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  Fail "Set SOLANA_RPC_URL (tools/mainnet_rpc_env.ps1)."
}

if ([string]::IsNullOrWhiteSpace($Keypair)) {
  if ($env:KEYPAIR_PATH) { $Keypair = $env:KEYPAIR_PATH }
  elseif ($env:SOLANA_KEYPAIR_PATH) { $Keypair = $env:SOLANA_KEYPAIR_PATH }
  else { $Keypair = Join-Path $env:USERPROFILE ".config\solana\clmm_lp_bot_mainnet.json" }
}
if (-not (Test-Path -LiteralPath $Keypair)) { Fail "Keypair not found: $Keypair" }

. (Join-Path $PSScriptRoot "orca_position_preflight_core.ps1")
. (Join-Path $PSScriptRoot "clmm_rpc_tools_helpers.ps1")
$null = Initialize-ClmmToolsRpcEnv

$preferExe = -not $CargoOnly.IsPresent

function ResolveOwnerFromKeypair([string]$kpPath) {
  if (Get-Command "solana-keygen" -ErrorAction SilentlyContinue) {
    return (& solana-keygen pubkey $kpPath | Select-Object -First 1).Trim()
  }
  $kpJson = Get-Content -LiteralPath $kpPath -Raw | ConvertFrom-Json
  $bytes = [byte[]]($kpJson | ForEach-Object { [byte]$_ })
  $pubkeyBytes = $bytes[32..63]
  $alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".ToCharArray()
  $zc = 0
  for ($i = 0; $i -lt $pubkeyBytes.Length; $i++) { if ($pubkeyBytes[$i] -ne 0) { break }; $zc++ }
  $digits = New-Object System.Collections.Generic.List[int]
  foreach ($byte in $pubkeyBytes) {
    $carry = [int]$byte
    for ($j = 0; $j -lt $digits.Count; $j++) {
      $carry += $digits[$j] * 256
      $digits[$j] = [int]($carry % 58)
      $carry = [int]([Math]::Floor($carry / 58))
    }
    while ($carry -gt 0) { $digits.Add([int]($carry % 58)); $carry = [int]([Math]::Floor($carry / 58)) }
  }
  $sb = New-Object System.Text.StringBuilder
  for ($i = 0; $i -lt $zc; $i++) { [void]$sb.Append('1') }
  for ($i = $digits.Count - 1; $i -ge 0; $i--) { [void]$sb.Append($alphabet[$digits[$i]]) }
  return $sb.ToString()
}

if ([string]::IsNullOrWhiteSpace($Owner)) {
  $Owner = ResolveOwnerFromKeypair $Keypair
}

function Parse-QuoteEstimates([string[]]$Lines) {
  $q = ($Lines | Where-Object { "$_" -match "quote:\s*" } | Select-Object -First 1)
  if (-not $q) { return $null }
  $estIn = $null
  $estOut = $null
  if ($q -match "token_est_in:\s*(\d+)") { $estIn = [bigint]$Matches[1] }
  if ($q -match "token_est_out:\s*(\d+)") { $estOut = [bigint]$Matches[1] }
  return @{ Line = "$q"; EstIn = $estIn; EstOut = $estOut }
}

function Invoke-OrcaSwapDryRunQuote {
  param(
    [string]$Pool,
    [string]$SpecifiedMint,
    [string]$SwapType,
    [UInt64]$Amount
  )
  $argv = @(
    "orca-swap",
    "--pool", $Pool,
    "--specified-mint", $SpecifiedMint,
    "--swap-type", $SwapType,
    "--amount", ([string]$Amount),
    "--slippage-bps", ([string]$SlippageBps),
    "--keypair", $Keypair,
    "--dry-run"
  )
  $out = Invoke-ClmmLpCliCapture -RepoRoot $repoRoot -PreferReleaseExe $preferExe -Argv $argv -StepLabel "orca-swap dry-run quote"
  return (Parse-QuoteEstimates ($out | ForEach-Object { "$_" }))
}

function Apply-BpsCeiling([bigint]$x, [UInt32]$bps) {
  if ($x -le 0) { return [UInt64]0 }
  $n = $x * [bigint](10000 + [int]$bps)
  $d = [bigint]10000
  $v = ($n + $d - [bigint]1) / $d
  if ($v -gt [bigint][UInt64]::MaxValue) { Fail "Buffered amount exceeds UInt64." }
  $u = [UInt64]$v
  if ($u -lt 1 -and $x -gt 0) { return [UInt64]1 }
  return $u
}

$r = Get-OrcaPositionOpenPreflightState -Pool $PoolCbBtc -RepoRoot $repoRoot -Owner $Owner `
  -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
  -PreferReleaseExe $preferExe -Quiet:$false -SkipInitRpcEnv:$true

if ($r.Ok) {
  Info "Preflight already PASS for this pool and amounts. No swaps needed."
  exit 0
}

if (-not $r.OkFee) {
  Fail ("Native SOL below reserve: have " + $r.NativeLamports + " lamports, need >= " + $ReserveSolLamports + ". Add SOL first.")
}

$defA = [bigint]$r.NeedA - [bigint]$r.AvailA
$defB = [bigint]$r.NeedB - [bigint]$r.AvailB
if ($defA -lt 0) { $defA = 0 }
if ($defB -lt 0) { $defB = 0 }

Info ("Deficits: cbBTC raw=" + $defA + " USDC raw=" + $defB)

$usdcForCbBtc = [bigint]0
$cbBtcExactOut = [UInt64]0
if ($defA -gt 0) {
  $cbBtcExactOut = Apply-BpsCeiling $defA $CbBtcBufferBps
  Info ("Quote: exact-out cbBTC on cbBTC/USDC, amount_out_raw=" + $cbBtcExactOut + " ...")
  $q1 = Invoke-OrcaSwapDryRunQuote -Pool $PoolCbBtc -SpecifiedMint $MintCbBtc -SwapType "exact-out" -Amount $cbBtcExactOut
  if ($null -eq $q1 -or $null -eq $q1.EstIn) { Fail "Could not parse quote (cbBTC exact-out)." }
  $usdcForCbBtc = $q1.EstIn
  Info ("  -> est USDC in (raw)=" + $usdcForCbBtc + " (~" + ([double]$usdcForCbBtc / 1e6) + " USDC)")
}

$totalUsdcNeed = $defB + $usdcForCbBtc
$usdcAvail = [bigint]$r.AvailB
$usdcGap = $totalUsdcNeed - $usdcAvail
if ($usdcGap -lt 0) { $usdcGap = 0 }

Info ("USDC: need total raw=" + $totalUsdcNeed + " (~" + ([double]$totalUsdcNeed / 1e6) + " USDC), avail raw=" + $usdcAvail + ", gap raw=" + $usdcGap)

$solForUsdc = [bigint]0
$usdcBuyExactOut = [UInt64]0
if ($usdcGap -gt 0) {
  $usdcBuyExactOut = Apply-BpsCeiling $usdcGap $UsdcHeadroomBps
  Info ("Quote: exact-out USDC on SOL/USDC, amount_out_raw=" + $usdcBuyExactOut + " ...")
  $q2 = Invoke-OrcaSwapDryRunQuote -Pool $PoolSolUsdc -SpecifiedMint $MintUsdc -SwapType "exact-out" -Amount $usdcBuyExactOut
  if ($null -eq $q2 -or $null -eq $q2.EstIn) { Fail "Could not parse quote (USDC exact-out)." }
  $solForUsdc = $q2.EstIn
  Info ("  -> est SOL in (lamports)=" + $solForUsdc + " (~" + ([double]$solForUsdc / 1e9) + " SOL)")
}

$solSpendable = [bigint]$r.NativeLamports - [bigint]$ReserveSolLamports
if ($solSpendable -lt 0) { $solSpendable = 0 }
# Rough tx fee headroom: a few swaps (priority fees not modeled).
$feeHeadroom = [bigint]150000

Write-Host ""
Write-Host "=== Summary (estimates from SDK dry-run quotes) ==="
Write-Host ("Open target: cbBTC raw=" + $AmountA + " USDC raw=" + $AmountB)
if ($usdcBuyExactOut -gt 0) {
  Write-Host ("1) SOL/USDC: exact-out USDC raw=" + $usdcBuyExactOut + "  (~out " + ([math]::Round([double]$usdcBuyExactOut / 1e6, 6)) + " USDC)")
  Write-Host ("   est SOL in: " + $solForUsdc + " lamports (~" + ([math]::Round([double]$solForUsdc / 1e9, 6)) + " SOL)")
}
if ($cbBtcExactOut -gt 0) {
  Write-Host ("2) cbBTC/USDC: exact-out cbBTC raw=" + $cbBtcExactOut)
  Write-Host ("   est USDC in: " + $usdcForCbBtc + " raw (~" + ([math]::Round([double]$usdcForCbBtc / 1e6, 6)) + " USDC)")
}
Write-Host "Then re-run preflight / open. Slippage quotes use -SlippageBps; on-chain may differ slightly."
Write-Host ""

if ($usdcBuyExactOut -gt 0 -and $solForUsdc + $feeHeadroom -gt $solSpendable) {
  Fail (
    "SOL not enough for SOL->USDC leg: est_in=" + [string]$solForUsdc + " lamports (~" +
    ([math]::Round([double]$solForUsdc / 1e9, 6)) + " SOL) + fee headroom; spendable (native - reserve)=" +
    [string]$solSpendable + " lamports (~" + ([math]::Round([double]$solSpendable / 1e9, 6)) +
    " SOL). Lower -AmountA/-AmountB or add SOL."
  )
}

if (-not $Execute.IsPresent) {
  Info "Dry plan only (no -Execute). Re-run with -Execute to send swaps."
  exit 0
}

$swapPs1 = Join-Path $PSScriptRoot "orca_swap.ps1"
$common = @{ Owner = $Owner; Keypair = $Keypair; SlippageBps = $SlippageBps; RepoRoot = $repoRoot; Execute = $true }
if ($CargoOnly.IsPresent) { $common.CargoOnly = $true }

if ($usdcBuyExactOut -gt 0) {
  Info "Executing SOL -> USDC (exact-out USDC)..."
  & $swapPs1 @common -Pool $PoolSolUsdc -SpecifiedMint $MintUsdc -SwapType "exact-out" -AmountRaw $usdcBuyExactOut
}

if ($cbBtcExactOut -gt 0) {
  Info "Executing USDC -> cbBTC (exact-out cbBTC)..."
  & $swapPs1 @common -Pool $PoolCbBtc -SpecifiedMint $MintCbBtc -SwapType "exact-out" -AmountRaw $cbBtcExactOut
}

Info "Running post-swap preflight (with optional top-up rounds for quote/on-chain drift)..."
for ($ti = 0; $ti -lt [int]$PostSwapTopUpMaxRounds; $ti++) {
  $r2 = Get-OrcaPositionOpenPreflightState -Pool $PoolCbBtc -RepoRoot $repoRoot -Owner $Owner `
    -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
    -PreferReleaseExe $preferExe -Quiet:$false -SkipInitRpcEnv:$true
  if ($r2.Ok) {
    Info "Post-swap preflight PASS."
    break
  }
  $didTopUp = $false
  if ($r2.OkFee -and $r2.OkA -and -not $r2.OkB) {
    $gapB = [bigint]$r2.NeedB - [bigint]$r2.AvailB
    if ($gapB -gt 0) {
      $topUpUsdc = Apply-BpsCeiling $gapB $UsdcHeadroomBps
      Info ("Top-up round " + ($ti + 1) + ": SOL -> USDC exact-out USDC raw=" + $topUpUsdc + " (gap_raw=" + $gapB + ")...")
      & $swapPs1 @common -Pool $PoolSolUsdc -SpecifiedMint $MintUsdc -SwapType "exact-out" -AmountRaw $topUpUsdc
      $didTopUp = $true
    }
  }
  elseif ($r2.OkFee -and -not $r2.OkA -and $r2.OkB) {
    $gapA = [bigint]$r2.NeedA - [bigint]$r2.AvailA
    if ($gapA -gt 0) {
      $topUpCb = Apply-BpsCeiling $gapA $CbBtcBufferBps
      Info ("Top-up round " + ($ti + 1) + ": USDC -> cbBTC exact-out cbBTC raw=" + $topUpCb + " (gap_raw=" + $gapA + ")...")
      & $swapPs1 @common -Pool $PoolCbBtc -SpecifiedMint $MintCbBtc -SwapType "exact-out" -AmountRaw $topUpCb
      $didTopUp = $true
    }
  }
  if (-not $didTopUp) {
    $null = Test-OrcaPositionOpenPreflight -Pool $PoolCbBtc -RepoRoot $repoRoot -Owner $Owner `
      -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
      -PreferReleaseExe $preferExe -Quiet:$false
  }
}

$rLast = Get-OrcaPositionOpenPreflightState -Pool $PoolCbBtc -RepoRoot $repoRoot -Owner $Owner `
  -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
  -PreferReleaseExe $preferExe -Quiet:$false -SkipInitRpcEnv:$true
if (-not $rLast.Ok) {
  $null = Test-OrcaPositionOpenPreflight -Pool $PoolCbBtc -RepoRoot $repoRoot -Owner $Owner `
    -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
    -PreferReleaseExe $preferExe -Quiet:$false
}
Info "Done."
