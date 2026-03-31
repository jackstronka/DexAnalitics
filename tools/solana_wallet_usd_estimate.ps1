# Estimate total wallet value in USD (read-only RPC + optional CoinGecko spot prices).
# Native SOL + SPL; aggregates multiple token accounts per mint.
#
# Pricing (defaults, overridable):
#   - USDC / USDT-style stables: 1 USD (mint allowlist)
#   - SOL, cbBTC (-> BTC), whETH portal (-> ETH): CoinGecko simple/price (free tier)
#   - Unknown mints: 0 USD with a warning (extend -MintPriceUsd or edit script)
#
# Usage (repo root):
#   . .\tools\mainnet_rpc_env.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\solana_wallet_usd_estimate.ps1 -Owner 8s9BcTUTXmWmZVPDrkoMNKsU6n1dRsihySv1bSteSvMQ
#   powershell ... -File .\tools\solana_wallet_usd_estimate.ps1 -Keypair "$env:USERPROFILE\.config\solana\clmm_lp_bot_mainnet.json"
#
# Optional: -Json (one line) for automation; -SkipPriceFetch (only on-chain UI amounts, no USD)

param(
  [string] $Owner = "",
  [string] $Keypair = "",
  [switch] $Json,
  [switch] $SkipPriceFetch,
  [int] $PriceTimeoutSec = 20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$m) { throw ("[wallet-usd] " + $m) }
function W([string]$m) { if (-not $Json) { Write-Host ("[wallet-usd] " + $m) } }

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$statePs1 = Join-Path $PSScriptRoot "solana_account_state.ps1"

if ([string]::IsNullOrWhiteSpace($Owner)) {
  if ([string]::IsNullOrWhiteSpace($Keypair)) {
    if ($env:KEYPAIR_PATH) { $Keypair = $env:KEYPAIR_PATH }
    elseif ($env:SOLANA_KEYPAIR_PATH) { $Keypair = $env:SOLANA_KEYPAIR_PATH }
    else { $Keypair = Join-Path $env:USERPROFILE ".config\solana\clmm_lp_bot_mainnet.json" }
  }
  if (-not (Test-Path -LiteralPath $Keypair)) { Fail "Keypair not found: $Keypair (pass -Owner or -Keypair)" }
  if (Get-Command "solana-keygen" -ErrorAction SilentlyContinue) {
    $Owner = (& solana-keygen pubkey $Keypair | Select-Object -First 1).Trim()
  } else {
    $kpJson = Get-Content -LiteralPath $Keypair -Raw | ConvertFrom-Json
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
    $Owner = $sb.ToString()
  }
}

if ([string]::IsNullOrWhiteSpace($Owner)) { Fail "Could not resolve -Owner" }

# Curated + common stables (same mints as tools/orca_curated_mainnet_pools.ps1 usage)
$mintStableUsd = @{
  "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" = 1.0   # USDC
  "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" = 1.0   # USDT
}
$mintToCoinGeckoId = @{
  "So11111111111111111111111111111111111111112" = "solana"
  "cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij" = "bitcoin"
  "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs" = "ethereum"
}
$wsolMint = "So11111111111111111111111111111111111111112"

$line = & powershell -NoProfile -ExecutionPolicy Bypass -File $statePs1 -Owner $Owner -Json 2>$null
if (-not $line -or $line.Trim().Length -lt 2) { Fail "solana_account_state.ps1 returned empty" }
$state = ($line | ConvertFrom-Json)

$cg = @{}
if (-not $SkipPriceFetch) {
  try {
    $url = "https://api.coingecko.com/api/v3/simple/price?ids=solana,bitcoin,ethereum&vs_currencies=usd"
    $resp = Invoke-RestMethod -Uri $url -Method Get -TimeoutSec $PriceTimeoutSec
    if ($resp.solana.usd) { $cg["solana"] = [double]$resp.solana.usd }
    if ($resp.bitcoin.usd) { $cg["bitcoin"] = [double]$resp.bitcoin.usd }
    if ($resp.ethereum.usd) { $cg["ethereum"] = [double]$resp.ethereum.usd }
  } catch {
    W ("CoinGecko fetch failed: " + $_.Exception.Message + " (use -SkipPriceFetch or retry)")
  }
}

function Price-ForMint([string]$mint) {
  if ($mintStableUsd.ContainsKey($mint)) { return @{ Usd = [double]$mintStableUsd[$mint]; Label = "stable(1)" } }
  if ($mintToCoinGeckoId.ContainsKey($mint)) {
    $id = $mintToCoinGeckoId[$mint]
    if ($cg.ContainsKey($id)) { return @{ Usd = [double]$cg[$id]; Label = $id } }
    return @{ Usd = 0.0; Label = "no_price" }
  }
  return @{ Usd = 0.0; Label = "unknown_mint" }
}

# Aggregate SPL by mint
$byMint = @{}
foreach ($a in $state.spl_token_accounts) {
  $m = [string]$a.mint
  $ui = $a.ui_amount
  if ($null -eq $ui) {
    try { $ui = [double]$a.ui_amount_string } catch { $ui = 0.0 }
  }
  if (-not $byMint.ContainsKey($m)) { $byMint[$m] = 0.0 }
  $byMint[$m] = [double]$byMint[$m] + [double]$ui
}

# Native SOL + wSOL ATAs (same mint) = one SOL line (avoid double count)
$wsolUiFromAta = 0.0
if ($byMint.ContainsKey($wsolMint)) {
  $wsolUiFromAta = [double]$byMint[$wsolMint]
  $null = $byMint.Remove($wsolMint)
}
$solUi = [double]$state.native_sol.sol + $wsolUiFromAta
$solP = Price-ForMint $wsolMint
$solUsd = if ($solP.Usd -gt 0) { $solUi * $solP.Usd } else { 0.0 }

$rows = New-Object System.Collections.Generic.List[hashtable]
[void]$rows.Add(@{ Kind = "sol_total"; Symbol = "SOL"; Mint = $wsolMint; Ui = $solUi; PriceUsd = $solP.Usd; Usd = $solUsd; Note = ($solP.Label + ";native+wSOL_ATA") })

$total = $solUsd
foreach ($kv in $byMint.GetEnumerator()) {
  $m = $kv.Key
  $ui = [double]$kv.Value
  $p = Price-ForMint $m
  $u = $ui * $p.Usd
  $sym = if ($m -eq "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v") { "USDC" }
  elseif ($m -eq "cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij") { "cbBTC~" }
  elseif ($m -eq "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs") { "whETH~" }
  elseif ($m -eq "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB") { "USDT" }
  else { "SPL" }
  $note = $p.Label
  if ($p.Usd -eq 0 -and $ui -gt 0) { $note = "unpriced_mint" }
  [void]$rows.Add(@{ Kind = "spl"; Symbol = $sym; Mint = $m; Ui = $ui; PriceUsd = $p.Usd; Usd = $u; Note = $note })
  $total += $u
}

$out = [ordered]@{
  schema_version = 1
  fetched_at_utc = [string]$state.fetched_at_utc
  owner          = [string]$state.owner
  rpc_url_used   = [string]$state.rpc_url_used
  total_usd_estimate = [math]::Round($total, 2)
  disclaimer     = "Estimate only: CoinGecko spot, USDC=1 USD, cbBTC priced as BTC, whETH as ETH; unpriced mints=0."
  rows           = @($rows)
}

if ($Json) {
  Write-Output ($out | ConvertTo-Json -Depth 8 -Compress)
} else {
  W ("Owner: " + $out.owner)
  W ("RPC:   " + $out.rpc_url_used)
  Write-Host ""
  Write-Host ("Kind" + "`t" + "Symbol" + "`t" + "UiAmount" + "`t" + "PxUSD" + "`t" + "USD" + "`t" + "Mint (short)" + "`t" + "Note")
  foreach ($r in $rows) {
    $mshort = [string]$r.Mint
    if ($mshort.Length -gt 12) { $mshort = $mshort.Substring(0, 6) + "…" + $mshort.Substring($mshort.Length - 4) }
    Write-Host ($r.Kind + "`t" + $r.Symbol + "`t" + [math]::Round([double]$r.Ui, 8) + "`t" + [math]::Round([double]$r.PriceUsd, 4) + "`t" + [math]::Round([double]$r.Usd, 4) + "`t" + $mshort + "`t" + $r.Note)
  }
  Write-Host ""
  Write-Host ("=== Total USD (estimate): " + $out.total_usd_estimate + " ===")
  Write-Host $out.disclaimer
}
