param(
  [Parameter(Mandatory = $true)]
  [string]$KeypairWinPath,

  # Orca devnet test pool used in SDK examples (SOL/devUSDC).
  [string]$Pool = "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt",

  # Orca devUSDC mint (Nebula tutorial token).
  [string]$DevUsdcMint = "BRjpCHtyQLNCo8gqRUr8jtdAj5AjPYQaoqbvcZiHok1k",

  # Quote input for price discovery: 1 SOL in lamports.
  [long]$QuoteLamports = 1000000000,

  # Quote input for reverse direction (devUSDC -> SOL): 1 devUSDC in base units (6 decimals).
  [long]$QuoteDevUsdc = 1000000,

  # Swap slippage tolerance.
  [int]$SlippageBps = 200,

  # If true: when devUSDC is overweight, also swap devUSDC -> SOL to get back to ~50/50.
  [switch]$AllowDevUsdcToSol = $true,

  # If true: don't send swap tx, just print computed plan.
  [switch]$DryRun = $false,

  # Optional: rebalance a specific position lifecycle (close -> wallet rebalance -> open).
  [string]$Position = "",
  [string]$ReopenPool = "",
  [double]$ReopenRangeWidthPct = 10.0,
  [int]$ReopenSleepSecs = 0,
  [long]$OpenAmountA = 1000000, # 0.001 SOL (lamports)
  [long]$OpenAmountB = 1000,    # 0.001 devUSDC (6 decimals)
  [int]$OpenSlippageBps = 100
)

$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[rebalance-half] " + $msg) }

if (-not (Test-Path $KeypairWinPath)) {
  throw ("Keypair file does not exist: " + $KeypairWinPath)
}

function ToWslPath([string]$winPath) {
  # Convert "C:\foo\bar" -> "/mnt/c/foo/bar"
  $p = $winPath -replace "\\", "/"
  if ($p -match "^([A-Za-z]):/(.*)$") {
    $drive = $Matches[1].ToLower()
    $rest = $Matches[2]
    return "/mnt/$drive/$rest"
  }
  throw ("Unsupported path format for WSL conversion: " + $winPath)
}

function ParseFirstNumber([string]$text) {
  $m = [regex]::Match($text, "([0-9]+(\.[0-9]+)?)")
  if (-not $m.Success) { throw ("Could not parse number from: " + $text) }
  return [decimal]$m.Groups[1].Value
}

function ParseTokenEstOut([string[]]$lines) {
  # Example:
  # quote: ExactIn(ExactInSwapQuote { token_in: 1000000000, token_est_out: 22255403, ... })
  $line = ($lines | Where-Object { $_ -match "token_est_out:" } | Select-Object -Last 1)
  if (-not $line) { throw "Could not find token_est_out in orca-swap output" }
  $m = [regex]::Match($line, "token_est_out:\s*([0-9]+)")
  if (-not $m.Success) { throw ("Could not parse token_est_out from: " + $line) }
  return [decimal]$m.Groups[1].Value
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$keypairWsl = ToWslPath $KeypairWinPath

Info ("Keypair: " + $KeypairWinPath)
Info ("Pool: " + $Pool)
Info ("devUSDC mint: " + $DevUsdcMint)

# WSL preflight: we rely on solana + spl-token inside WSL to read balances (same env you used).
$hasSolana = (& wsl bash -lc "command -v solana >/dev/null 2>&1; echo $?" ).Trim()
$hasSpl = (& wsl bash -lc "command -v spl-token >/dev/null 2>&1; echo $?" ).Trim()
if ($hasSolana -ne "0" -or $hasSpl -ne "0") {
  throw @"
[rebalance-half] Missing required commands inside WSL: solana and/or spl-token.

This script reads balances via WSL (same workflow you used in terminal). Please install Solana CLI in WSL,
or run the script in the environment where `solana` and `spl-token` are available.

Quick check (WSL):
  solana --version
  spl-token --version
"@
}

# Ensure WSL CLI talks to devnet.
& wsl solana config set --url https://api.devnet.solana.com | Out-Null

$pubkey = (& wsl solana address -k $keypairWsl).Trim()
if ([string]::IsNullOrWhiteSpace($pubkey)) { throw "Failed to get wallet pubkey via WSL solana" }
Info ("Wallet: " + $pubkey)

$solLine = (& wsl solana balance $pubkey).Trim()
$sol = ParseFirstNumber $solLine
Info ("SOL balance: " + $solLine)

$usdcLine = (& wsl spl-token balance $DevUsdcMint --owner $pubkey).Trim()
$devUsdc = ParseFirstNumber $usdcLine
Info ("devUSDC balance: " + $usdcLine)

# Optional position lifecycle: close first (to bring funds into wallet).
if (-not [string]::IsNullOrWhiteSpace($Position)) {
  if ([string]::IsNullOrWhiteSpace($ReopenPool)) {
    throw "[rebalance-half] When -Position is set, also provide -ReopenPool (whirlpool address)."
  }
  Info ("Position mode: closing position " + $Position)
  if ($DryRun) {
    Info "DryRun: would close position (skipped)."
  } else {
    & cargo run --bin clmm-lp-cli -- orca-position-close `
      --position $Position `
      --keypair $KeypairWinPath
    if ($LASTEXITCODE -ne 0) { throw "[rebalance-half] orca-position-close failed" }
  }
}

# Price discovery: how much devUSDC we get for QuoteLamports.
$quoteOut = & cargo run --bin clmm-lp-cli -- orca-swap `
  --pool $Pool `
  --specified-mint So11111111111111111111111111111111111111112 `
  --swap-type exact-in `
  --amount $QuoteLamports `
  --slippage-bps $SlippageBps `
  --dry-run `
  --keypair $KeypairWinPath 2>&1

$tokenEstOut = ParseTokenEstOut $quoteOut
$quoteSol = [decimal]$QuoteLamports / 1000000000.0
$rateUsdcPerSol = ($tokenEstOut / 1000000.0) / $quoteSol # devUSDC has 6 decimals
Info ("Rate estimate: 1 SOL ~= " + $rateUsdcPerSol + " devUSDC (from quote)")

$valueSolInUsdc = $sol * $rateUsdcPerSol
$totalUsdcValue = $valueSolInUsdc + $devUsdc
$targetEach = $totalUsdcValue / 2
$delta = $valueSolInUsdc - $targetEach # positive => too much SOL

Info ("Value SOL in devUSDC ~= " + $valueSolInUsdc)
Info ("Total value ~= " + $totalUsdcValue + " devUSDC, target each ~= " + $targetEach)

if ($delta -le 0) {
  if (-not $AllowDevUsdcToSol) {
    Info ("Already SOL-underweight or balanced (delta=" + $delta + "). No SOL->devUSDC swap needed.")
    exit 0
  }

  $needUsdc = (-1) * $delta
  Info ("devUSDC overweight by ~= " + $needUsdc + " (value units). Planning devUSDC->SOL swap.")

  $quoteOut2 = & cargo run --bin clmm-lp-cli -- orca-swap `
    --pool $Pool `
    --specified-mint $DevUsdcMint `
    --swap-type exact-in `
    --amount $QuoteDevUsdc `
    --slippage-bps $SlippageBps `
    --dry-run `
    --keypair $KeypairWinPath 2>&1

  $estLamportsOut = ParseTokenEstOut $quoteOut2
  $rateSolPerUsdc = (([decimal]$estLamportsOut) / 1000000000.0) / (([decimal]$QuoteDevUsdc) / 1000000.0)
  Info ("Rate estimate: 1 devUSDC ~= " + $rateSolPerUsdc + " SOL (from reverse quote)")

  $usdcToSwap = [decimal]$needUsdc
  $microToSwap = [long][math]::Floor(([double]$usdcToSwap) * 1000000.0)
  if ($microToSwap -lt 1) {
    Info "Computed microToSwap < 1; skipping."
  } else {
    Info ("Plan: swap devUSDC->SOL amount ~= " + $usdcToSwap + " devUSDC (" + $microToSwap + " micro units)")
    if ($DryRun) {
      Info "DryRun: not sending swap transaction."
    } else {
      & cargo run --bin clmm-lp-cli -- orca-swap `
        --pool $Pool `
        --specified-mint $DevUsdcMint `
        --swap-type exact-in `
        --amount $microToSwap `
        --slippage-bps $SlippageBps `
        --keypair $KeypairWinPath
      if ($LASTEXITCODE -ne 0) { throw "[rebalance-half] devUSDC->SOL swap failed" }
    }
  }

  if (-not [string]::IsNullOrWhiteSpace($Position)) {
    Info ("Position mode: reopening on pool " + $ReopenPool)
    if ($DryRun) {
      Info "DryRun: would reopen position (skipped)."
      exit 0
    }
    if ($ReopenSleepSecs -gt 0) { Start-Sleep -Seconds $ReopenSleepSecs }
    & cargo run --bin clmm-lp-cli -- orca-position-open `
      --pool $ReopenPool `
      --range-width-pct $ReopenRangeWidthPct `
      --amount-a $OpenAmountA `
      --amount-b $OpenAmountB `
      --slippage-bps $OpenSlippageBps `
      --keypair $KeypairWinPath
    exit 0
  }

  exit 0
}

$solToSwap = $delta / $rateUsdcPerSol
$lamportsToSwap = [long][math]::Floor(([double]$solToSwap) * 1000000000.0)

if ($lamportsToSwap -lt 1) {
  Info "Computed lamportsToSwap < 1; skipping."
  exit 0
}

Info ("Plan: swap SOL->devUSDC amount ~= " + $solToSwap + " SOL (" + $lamportsToSwap + " lamports)")

if ($DryRun) {
  Info "DryRun: not sending swap transaction."
  exit 0
}

& cargo run --bin clmm-lp-cli -- orca-swap `
  --pool $Pool `
  --specified-mint So11111111111111111111111111111111111111112 `
  --swap-type exact-in `
  --amount $lamportsToSwap `
  --slippage-bps $SlippageBps `
  --keypair $KeypairWinPath

Info "Re-reading balances after swap..."
$sol2 = ParseFirstNumber ((& wsl solana balance $pubkey).Trim())
$devUsdc2 = ParseFirstNumber ((& wsl spl-token balance $DevUsdcMint --owner $pubkey).Trim())
Info ("SOL now: " + $sol2 + " | devUSDC now: " + $devUsdc2)

if (-not [string]::IsNullOrWhiteSpace($Position)) {
  Info ("Position mode: reopening on pool " + $ReopenPool)
  if ($DryRun) {
    Info "DryRun: would reopen position (skipped)."
    exit 0
  }
  if ($ReopenSleepSecs -gt 0) { Start-Sleep -Seconds $ReopenSleepSecs }
  & cargo run --bin clmm-lp-cli -- orca-position-open `
    --pool $ReopenPool `
    --range-width-pct $ReopenRangeWidthPct `
    --amount-a $OpenAmountA `
    --amount-b $OpenAmountB `
    --slippage-bps $OpenSlippageBps `
    --keypair $KeypairWinPath
}

