# Example "file-based" mainnet RPC configuration.
#
# Copy to: tools/mainnet_rpc_env.ps1 (keep your real provider URLs out of git)
# Then run (repo root):
#   . .\tools\mainnet_rpc_env.ps1
#
# This sets:
# - SOLANA_RPC_URL
# - SOLANA_RPC_FALLBACK_URLS
# - CLMM_EXPECTED_CLUSTER (mainnet-beta)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "solana_rpc_env.ps1")

Set-SolanaRpcEnv `
  -SolanaRpcUrl "https://api.mainnet-beta.solana.com" `
  -SolanaRpcFallbackUrls "https://solana-api.projectserum.com,https://rpc.ankr.com/solana" `
  -ExpectedCluster "mainnet-beta"

