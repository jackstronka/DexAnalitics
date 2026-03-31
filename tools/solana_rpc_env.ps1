# Shared helper for consistent Solana RPC environment setup across scripts.
#
# Usage (dot-source):
#   . "$PSScriptRoot/solana_rpc_env.ps1"
#   Set-SolanaRpcEnv -SolanaRpcUrl "https://..." -SolanaRpcFallbackUrls "https://a,https://b" -ExpectedCluster "mainnet"
#
# Notes:
# - Keep clusters consistent across primary + fallbacks.
# - Many public endpoints throttle/disable tx history; for swaps-enrich prefer a dedicated endpoint.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Set-SolanaRpcEnv {
    param(
        [Parameter(Mandatory = $true)]
        [string] $SolanaRpcUrl,
        [string] $SolanaRpcFallbackUrls = "",
        # Optional safety guard (used by Rust code too). Typical: "mainnet" or "devnet".
        [string] $ExpectedCluster = ""
    )

    if ([string]::IsNullOrWhiteSpace($SolanaRpcUrl)) {
        throw "Set-SolanaRpcEnv: SolanaRpcUrl is empty"
    }

    Set-Item -Path Env:SOLANA_RPC_URL -Value $SolanaRpcUrl.Trim()
    Set-Item -Path Env:SOLANA_RPC_FALLBACK_URLS -Value ($SolanaRpcFallbackUrls.Trim())

    if (-not [string]::IsNullOrWhiteSpace($ExpectedCluster)) {
        Set-Item -Path Env:CLMM_EXPECTED_CLUSTER -Value $ExpectedCluster.Trim()
    }

    Show-SolanaRpcEnv
}

function Show-SolanaRpcEnv {
    $primary = $env:SOLANA_RPC_URL
    $fallbacks = $env:SOLANA_RPC_FALLBACK_URLS
    $cluster = $env:CLMM_EXPECTED_CLUSTER

    Write-Host ("[rpc-env] SOLANA_RPC_URL=" + $primary)
    if (-not [string]::IsNullOrWhiteSpace($fallbacks)) {
        Write-Host ("[rpc-env] SOLANA_RPC_FALLBACK_URLS=" + $fallbacks)
    } else {
        Write-Host "[rpc-env] SOLANA_RPC_FALLBACK_URLS=(empty)"
    }
    if (-not [string]::IsNullOrWhiteSpace($cluster)) {
        Write-Host ("[rpc-env] CLMM_EXPECTED_CLUSTER=" + $cluster)
    } else {
        Write-Host "[rpc-env] CLMM_EXPECTED_CLUSTER=(unset)"
    }
}

