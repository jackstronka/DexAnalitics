param(
  [string]$KeypairPath = "",
  [string]$SolanaRpcUrl = "https://api.devnet.solana.com",
  [string]$Pool = "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt",
  [string]$DevUsdcMint = "BRjpCHtyQLNCo8gqRUr8jtdAj5AjPYQaoqbvcZiHok1k",
  [switch]$DryRun = $false,
  # Optional: run 50/50 rebalance (requires WSL + solana/spl-token; see devnet_rebalance_wallet_half.ps1).
  [switch]$RunHalfRebalance = $false
)

$ErrorActionPreference = "Stop"
function Info([string]$msg) { Write-Host ("[devnet-wallet-setup] " + $msg) }

if ([string]::IsNullOrWhiteSpace($KeypairPath)) {
  if ($env:KEYPAIR_PATH) { $KeypairPath = $env:KEYPAIR_PATH }
  elseif ($env:SOLANA_KEYPAIR_PATH) { $KeypairPath = $env:SOLANA_KEYPAIR_PATH }
}
if ([string]::IsNullOrWhiteSpace($KeypairPath)) {
  throw "Provide -KeypairPath or set KEYPAIR_PATH / SOLANA_KEYPAIR_PATH"
}
if (-not (Test-Path $KeypairPath)) {
  throw ("Keypair file does not exist: " + $KeypairPath)
}

Set-Item -Path Env:SOLANA_RPC_URL -Value $SolanaRpcUrl
Set-Item -Path Env:KEYPAIR_PATH -Value $KeypairPath
Set-Item -Path Env:DEVNET_POOL_ADDRESS -Value $Pool

Info ("RPC: " + $env:SOLANA_RPC_URL)
Info ("Keypair: " + $KeypairPath)
Info ("Pool: " + $Pool)

if ($DryRun) {
  Info "DryRun: would run airdrop request + 50/50 rebalance (skipped)"
  exit 0
}

# Best-effort SOL airdrop (devnet faucet; may rate-limit).
Info "Requesting devnet airdrop (best-effort)..."
& solana airdrop 2 $KeypairPath --url $SolanaRpcUrl 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) {
  Info "Airdrop failed or skipped (rate limit); continue if wallet already funded."
}

if ($RunHalfRebalance) {
  Info "Rebalancing toward ~50/50 SOL vs devUSDC (see tools/devnet_rebalance_wallet_half.ps1)..."
  & (Join-Path $PSScriptRoot "devnet_rebalance_wallet_half.ps1") `
    -KeypairWinPath $KeypairPath `
    -Pool $Pool `
    -DevUsdcMint $DevUsdcMint
}

Info "Wallet setup done. Optional: set CLMM_STABLE_MINT_FOR_SOL_TOPUP=$DevUsdcMint for devnet operational-SOL top-up if needed."
