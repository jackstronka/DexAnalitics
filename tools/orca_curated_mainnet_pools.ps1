# Curated Orca Whirlpool mainnet pools (single source of truth for tools).
# Dot-source from tools/*.ps1 — defines $script:OrcaCuratedMainnetPools and helpers.
#
# token_mint_a / token_mint_b order matches `clmm-lp-cli orca-pool-read` for each pool.
# SOL is always native mint (wSOL) So111... for CLI swaps.

Set-StrictMode -Version Latest

$script:OrcaCuratedWsolMint = "So11111111111111111111111111111111111111112"

$script:OrcaCuratedMainnetPools = @(
  @{
    Id      = "SOL_USDC"
    Label   = "SOL/USDC 0.04%"
    Pool    = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"
    MintA   = $script:OrcaCuratedWsolMint
    MintB   = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
    SymbolA = "SOL"
    SymbolB = "USDC"
  },
  @{
    Id      = "WHETH_SOL"
    Label   = "whETH/SOL 0.05%"
    Pool    = "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF"
    MintA   = $script:OrcaCuratedWsolMint
    MintB   = "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs"
    SymbolA = "SOL"
    SymbolB = "WHETH"
  },
  @{
    Id      = "CBBTC_USDC"
    Label   = "cbBTC/USDC 0.04%"
    Pool    = "HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM"
    MintA   = "cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij"
    MintB   = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
    SymbolA = "CBBTC"
    SymbolB = "USDC"
  },
  @{
    Id      = "CBBTC_WBTC"
    Label   = "cbBTC/WBTC 0.01%"
    Pool    = "4v8ufj8Hj7UvFgtofQJAtzUud5xomwZfEqfCTHZ4wM72"
    MintA   = "cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij"
    MintB   = "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh"
    SymbolA = "CBBTC"
    SymbolB = "WBTC"
  }
)

function Get-OrcaCuratedMainnetPoolsAll {
  return $script:OrcaCuratedMainnetPools
}

function Get-OrcaCuratedMainnetPoolById {
  param([Parameter(Mandatory)][string]$Id)
  $want = $Id.Trim().ToUpperInvariant()
  foreach ($p in $script:OrcaCuratedMainnetPools) {
    if ($p.Id.ToUpperInvariant() -eq $want) { return $p }
  }
  throw ("Unknown curated pool Id='" + $Id + "'. Use -ListPairs on orca_swap_curated.ps1.")
}

function Normalize-OrcaCuratedTokenSymbol {
  param([Parameter(Mandatory)][string]$Symbol)
  $u = $Symbol.Trim().ToUpperInvariant()
  if ($u -eq "WSOL") { return "SOL" }
  if ($u -eq "ETH" -or $u -eq "WETH") { return "WHETH" }
  return $u
}

function Resolve-OrcaCuratedMintForSymbol {
  param(
    [Parameter(Mandatory)][hashtable]$PoolEntry,
    [Parameter(Mandatory)][string]$Symbol
  )
  $raw = $Symbol.Trim().ToUpperInvariant()
  $s = Normalize-OrcaCuratedTokenSymbol $Symbol
  # Legacy alias: cbBTC/USDC tooling historically accepted WBTC/BTC as names for the cbBTC mint.
  if ($PoolEntry.Id -eq "CBBTC_USDC" -and ($raw -eq "WBTC" -or $raw -eq "BTC")) {
    $s = "CBBTC"
  }
  if ($s -eq $PoolEntry.SymbolA) { return @{ Mint = [string]$PoolEntry.MintA; Symbol = $s } }
  if ($s -eq $PoolEntry.SymbolB) { return @{ Mint = [string]$PoolEntry.MintB; Symbol = $s } }
  throw ("Token '" + $Symbol + "' is not part of pair " + $PoolEntry.Id + " (" + $PoolEntry.SymbolA + " <-> " + $PoolEntry.SymbolB + ").")
}
