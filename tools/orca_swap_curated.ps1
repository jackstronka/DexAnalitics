# Swap on curated mainnet Orca pools (3 pairs from STARTUP.md) in any leg direction.
#
# Token order per pool matches `orca-pool-read` (mint_a = SymbolA, mint_b = SymbolB):
#   SOL_USDC:    SOL (wSOL) <-> USDC
#   WHETH_SOL:   SOL (wSOL) <-> WHETH (portal)
#   CBBTC_USDC:  cbBTC <-> USDC
#
# Mapping to `orca-swap --specified-mint`:
#   exact-in:  amount is in **From** token (you spend From).
#   exact-out: amount is in **To** token (you receive To).
#
# Usage (repo root):
#   . .\tools\mainnet_rpc_env.ps1
#   $kp = "$env:USERPROFILE\.config\solana\clmm_lp_bot_mainnet.json"
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_swap_curated.ps1 -ListPairs
#   powershell ... -File .\tools\orca_swap_curated.ps1 -Pair SOL_USDC -From SOL -To USDC -SwapType exact-in -AmountRaw 1000000 -Keypair $kp -Execute
#
# Aliases: WSOL->SOL, ETH/WETH->WHETH, BTC/WBTC->CBBTC (case-insensitive).

param(
  [ValidateSet("SOL_USDC", "WHETH_SOL", "CBBTC_USDC", "")]
  [string] $Pair = "",

  [string] $From = "",
  [string] $To = "",

  [ValidateSet("exact-in", "exact-out")]
  [string] $SwapType = "exact-in",

  [UInt64] $AmountRaw = 0,

  [string] $Keypair = "",
  [string] $Owner = "",

  [switch] $Execute,
  [UInt16] $SlippageBps = 100,
  [switch] $CargoOnly,
  [switch] $ListPairs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[orca-swap-curated] " + $msg) }
function Fail([string]$msg) { throw ("[orca-swap-curated] " + $msg) }

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

. (Join-Path $PSScriptRoot "orca_curated_mainnet_pools.ps1")

if ($ListPairs.IsPresent) {
  Write-Host "Curated Orca mainnet pools (swap any leg via -From / -To + -SwapType):"
  Write-Host ""
  foreach ($p in Get-OrcaCuratedMainnetPoolsAll) {
    Write-Host ("== " + $p.Id + " | " + $p.Label + " ==")
    Write-Host ("  Pool:    " + $p.Pool)
    Write-Host ("  mint_a:  " + $p.MintA + "  (" + $p.SymbolA + ")")
    Write-Host ("  mint_b:  " + $p.MintB + "  (" + $p.SymbolB + ")")
    Write-Host "  Examples:"
    Write-Host ("    Spend " + $p.SymbolA + " -> receive " + $p.SymbolB + ":  -Pair " + $p.Id + " -From " + $p.SymbolA + " -To " + $p.SymbolB + " -SwapType exact-in  -AmountRaw <raw_" + $p.SymbolA + ">")
    Write-Host ("    Spend " + $p.SymbolB + " -> receive " + $p.SymbolA + ":  -Pair " + $p.Id + " -From " + $p.SymbolB + " -To " + $p.SymbolA + " -SwapType exact-in  -AmountRaw <raw_" + $p.SymbolB + ">")
    Write-Host ("    Receive " + $p.SymbolB + " (pay " + $p.SymbolA + "):    -Pair " + $p.Id + " -From " + $p.SymbolA + " -To " + $p.SymbolB + " -SwapType exact-out -AmountRaw <raw_" + $p.SymbolB + ">")
    Write-Host ("    Receive " + $p.SymbolA + " (pay " + $p.SymbolB + "):    -Pair " + $p.Id + " -From " + $p.SymbolB + " -To " + $p.SymbolA + " -SwapType exact-out -AmountRaw <raw_" + $p.SymbolA + ">")
    Write-Host ""
  }
  exit 0
}

if ([string]::IsNullOrWhiteSpace($Pair)) {
  Fail "Pass -Pair SOL_USDC | WHETH_SOL | CBBTC_USDC, or use -ListPairs."
}
if ([string]::IsNullOrWhiteSpace($From) -or [string]::IsNullOrWhiteSpace($To)) {
  Fail "Pass -From and -To using symbols for that pair (see -ListPairs)."
}
if ($AmountRaw -eq 0) {
  Fail "-AmountRaw must be > 0."
}

if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  $mainnetEnv = Join-Path $PSScriptRoot "mainnet_rpc_env.ps1"
  if (Test-Path -LiteralPath $mainnetEnv) {
    Info ("SOLANA_RPC_URL not set; dot-sourcing " + $mainnetEnv)
    . $mainnetEnv
  }
}
if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  Fail "Missing SOLANA_RPC_URL. Dot-source tools/mainnet_rpc_env.ps1 first."
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
  Fail ("Keypair not found: " + $Keypair)
}

function ResolveOwnerFromKeypair([string]$kpPath) {
  if (Get-Command "solana-keygen" -ErrorAction SilentlyContinue) {
    return (& solana-keygen pubkey $kpPath | Select-Object -First 1).Trim()
  }
  $kpJson = Get-Content -LiteralPath $kpPath -Raw | ConvertFrom-Json
  if ($null -eq $kpJson) { throw "Could not parse keypair JSON at $kpPath" }
  $bytes = [byte[]]($kpJson | ForEach-Object { [byte]$_ })
  if ($bytes.Length -lt 64) { throw "Keypair JSON too short (expected 64 bytes)" }
  $pubkeyBytes = $bytes[32..63]
  $alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".ToCharArray()
  $zeroCount = 0
  for ($i = 0; $i -lt $pubkeyBytes.Length; $i++) { if ($pubkeyBytes[$i] -ne 0) { break }; $zeroCount++ }
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
  for ($i = 0; $i -lt $zeroCount; $i++) { [void]$sb.Append('1') }
  for ($i = $digits.Count - 1; $i -ge 0; $i--) { [void]$sb.Append($alphabet[$digits[$i]]) }
  return $sb.ToString()
}

if ([string]::IsNullOrWhiteSpace($Owner)) {
  $Owner = ResolveOwnerFromKeypair $Keypair
}

$pe = Get-OrcaCuratedMainnetPoolById $Pair
$fSym = Normalize-OrcaCuratedTokenSymbol $From
$tSym = Normalize-OrcaCuratedTokenSymbol $To
if ($fSym -eq $tSym) {
  Fail "-From and -To must name the two different tokens in the pair."
}

$fromResolved = Resolve-OrcaCuratedMintForSymbol $pe $fSym
$toResolved = Resolve-OrcaCuratedMintForSymbol $pe $tSym

# Ensure From/To span both legs (second resolve already validates membership).
$specifiedMint = $null
if ($SwapType -eq "exact-in") {
  $specifiedMint = $fromResolved.Mint
  Info ("Swap " + $SwapType + ": spend " + $fromResolved.Symbol + " amount_raw=" + $AmountRaw + " -> receive " + $toResolved.Symbol + " (pool " + $pe.Pool + ")")
} else {
  $specifiedMint = $toResolved.Mint
  Info ("Swap " + $SwapType + ": receive " + $toResolved.Symbol + " amount_raw=" + $AmountRaw + " (pay " + $fromResolved.Symbol + ") (pool " + $pe.Pool + ")")
}

$swapScript = Join-Path $PSScriptRoot "orca_swap.ps1"
$swapArgs = @{
  Owner          = $Owner
  Keypair        = $Keypair
  Pool           = $pe.Pool
  SpecifiedMint  = $specifiedMint
  SwapType       = $SwapType
  AmountRaw      = $AmountRaw
  SlippageBps    = $SlippageBps
  RepoRoot       = $repoRoot
}
if ($Execute.IsPresent) { $swapArgs.Execute = $true }
if ($CargoOnly.IsPresent) { $swapArgs.CargoOnly = $true }

& $swapScript @swapArgs
