# Quick close Whirlpool position from local position registry.
# - Resolves: RPC (SOLANA_RPC_URL) + keypair path + owner pubkey + active position PDA.
# - Finds the last active (registry_open without a later registry_close) position for the owner
#   in `data/positions/registry.jsonl` (or CLMM_POSITION_REGISTRY_PATH).
# - Executes: `clmm-lp-cli orca-position-close --position <PDA> --keypair <KEYPAIR> --slippage-bps <N>` (default 500 bps)
#
# Usage (repo root):
#   .\tools\orca_position_close_quick.ps1 -Keypair "C:\secure\devnet-bot\wallet.json" -DryRun
#   .\tools\orca_position_close_quick.ps1
#
# Optional:
#   .\tools\orca_position_close_quick.ps1 -Owner <OWNER_PUBKEY> -Position <POSITION_PDA>
#   .\tools\orca_position_close_quick.ps1 -RegistryPath ".\data\positions\registry.jsonl"

param(
  [string] $Owner = "",
  [string] $Keypair = "",
  [string] $Position = "",
  [string] $RegistryPath = "",
  [UInt16] $SlippageBps = 500,
  [switch] $DryRun,
  [switch] $Verify
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[orca-position-close-quick] " + $msg) }
function Fail([string]$msg) { throw ("[orca-position-close-quick] " + $msg) }

# Resolve repo root
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

# Resolve RPC
if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  $mainnetEnv = Join-Path $PSScriptRoot "mainnet_rpc_env.ps1"
  if (Test-Path -LiteralPath $mainnetEnv) {
    Info ("SOLANA_RPC_URL not set; dot-sourcing " + $mainnetEnv)
    . $mainnetEnv
  }
}
if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  Fail "Missing SOLANA_RPC_URL. Dot-source tools/mainnet_rpc_env.ps1 first or export SOLANA_RPC_URL."
}

. (Join-Path $PSScriptRoot "clmm_rpc_tools_helpers.ps1")
if (Initialize-ClmmToolsRpcEnv) {
  Info "Default CLMM_RPC_DENYLIST=ankr,projectserum (mainnet fallbacks). Set CLMM_RPC_DENYLIST yourself to override."
}

# Resolve keypair file path
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
  Fail ("Keypair file not found: " + $Keypair)
}

# Resolve owner pubkey
if ([string]::IsNullOrWhiteSpace($Owner)) {
  if (Get-Command "solana-keygen" -ErrorAction SilentlyContinue) {
    $Owner = (& solana-keygen pubkey $Keypair | Select-Object -First 1).Trim()
  } else {
    # Fallback: derive pubkey from keypair JSON (array of 64 bytes: secret[32] + pubkey[32]).
    $kpJson = Get-Content -LiteralPath $Keypair -Raw | ConvertFrom-Json
    if ($null -eq $kpJson) { Fail "Could not parse keypair JSON at " + $Keypair }
    $bytes = [byte[]]($kpJson | ForEach-Object { [byte]$_ })
    if ($bytes.Length -lt 64) { Fail ("Keypair JSON too short (expected 64 bytes), len=" + $bytes.Length) }

    $pubkeyBytes = $bytes[32..63]

    function Base58Encode([byte[]] $data) {
      # Base58 encoding without BigInteger (byte-wise base conversion).
      $alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".ToCharArray()
      if ($null -eq $data) { return "" }

      # Count leading zeros.
      $zeroCount = 0
      for ($i = 0; $i -lt $data.Length; $i++) {
        if ($data[$i] -ne 0) { break }
        $zeroCount++
      }

      # digits holds base-58 digits in little-endian (least significant first).
      $digits = New-Object System.Collections.Generic.List[int]

      for ($i = 0; $i -lt $data.Length; $i++) {
        $carry = [int]$data[$i]

        for ($j = 0; $j -lt $digits.Count; $j++) {
          $carry += $digits[$j] * 256
          $digits[$j] = [int]($carry % 58)
          $carry = [int]([Math]::Floor($carry / 58))
        }

        while ($carry -gt 0) {
          $digits.Add([int]($carry % 58))
          $carry = [int]([Math]::Floor($carry / 58))
        }
      }

      # Leading zero bytes become '1'.
      $sb = New-Object System.Text.StringBuilder
      for ($i = 0; $i -lt $zeroCount; $i++) { [void]$sb.Append('1') }

      # Convert digits back to a string (reverse, most significant first).
      for ($i = $digits.Count - 1; $i -ge 0; $i--) {
        [void]$sb.Append($alphabet[$digits[$i]])
      }

      if ($sb.Length -eq 0) { return "" }
      return $sb.ToString()
    }

    $Owner = Base58Encode $pubkeyBytes
  }
}
if ([string]::IsNullOrWhiteSpace($Owner)) {
  Fail "Could not resolve owner pubkey."
}

# Resolve registry path
if ([string]::IsNullOrWhiteSpace($RegistryPath)) {
  if ($env:CLMM_POSITION_REGISTRY_PATH -and -not [string]::IsNullOrWhiteSpace($env:CLMM_POSITION_REGISTRY_PATH)) {
    $RegistryPath = $env:CLMM_POSITION_REGISTRY_PATH
  } else {
    $RegistryPath = Join-Path $repoRoot "data\positions\registry.jsonl"
  }
}
if (-not (Test-Path -LiteralPath $RegistryPath)) {
  Fail ("Registry file not found: " + $RegistryPath)
}

Info ("RPC=" + $env:SOLANA_RPC_URL)
Info ("Keypair=" + $Keypair)
Info ("Owner=" + $Owner)
Info ("Registry=" + $RegistryPath)
Info ("slippage_bps(close)=" + $SlippageBps)

# Parse registry.jsonl and determine active positions for owner
$lastByPosition = @{} # position_pubkey -> last record (as PSCustomObject)

Get-Content -LiteralPath $RegistryPath | ForEach-Object {
  $line = $_.Trim()
  if ([string]::IsNullOrWhiteSpace($line)) { return }
  $obj = $line | ConvertFrom-Json
  $pos = [string]$obj.position_pubkey
  if (-not [string]::IsNullOrWhiteSpace($pos)) {
    $lastByPosition[$pos] = $obj
  }
}

$activeForOwner = @()
foreach ($kvp in $lastByPosition.GetEnumerator()) {
  $pos = $kvp.Key
  $row = $kvp.Value
  if ($row.event -ne "registry_open") { continue }
  if ([string]$row.owner_pubkey -ne $Owner) { continue }
  $activeForOwner += $row
}

if ($activeForOwner.Count -eq 0) {
  Info "No active registry positions found for owner; falling back to orca-positions-list (on-chain)."
  $listOut = & cargo run --bin clmm-lp-cli -- orca-positions-list --owner $Owner --keypair $Keypair 2>&1
  $positions = New-Object System.Collections.Generic.List[string]
  foreach ($line in $listOut) {
    if ($line -match "kind=position position=([^ ]+)") {
      $positions.Add($Matches[1]) | Out-Null
    }
  }
  if ($positions.Count -eq 0) {
    Fail ("No open positions found on-chain for owner " + $Owner)
  }
  if (-not [string]::IsNullOrWhiteSpace($Position)) {
    if ($positions -notcontains $Position) {
      Fail ("Provided -Position is not open for owner per orca-positions-list: " + $Position)
    }
  } else {
    if ($positions.Count -gt 1) {
      Fail ("Multiple open positions found per orca-positions-list; pass -Position to choose. count=" + $positions.Count)
    }
    $Position = $positions[0]
  }

  # In fallback mode we don't reliably know pool_address; that's fine because orca-position-close
  # fetches pool from the position on-chain.
  $selected = [pscustomobject]@{ position_pubkey = $Position; pool_address = ""; owner_pubkey = $Owner; ts_utc = ""; event = "registry_fallback_list" }
} else {
  $selected = $null
  if (-not [string]::IsNullOrWhiteSpace($Position)) {
    foreach ($row in $activeForOwner) {
      if ([string]$row.position_pubkey -eq $Position) { $selected = $row }
    }
    if ($null -eq $selected) {
      Fail ("Position " + $Position + " is not active for owner " + $Owner + " per registry.")
    }
  } else {
    # Choose the newest active open by ts_utc (best-effort; registry is append-only)
    $selected = ($activeForOwner | Sort-Object { [DateTime]$_.ts_utc } -Descending | Select-Object -First 1)
    $Position = [string]$selected.position_pubkey
  }
}

Info ("Selected position=" + $Position)
Info ("Selected pool=" + [string]$selected.pool_address)

$cliArgs = @(
  "run", "--bin", "clmm-lp-cli", "--",
  "orca-position-close",
  "--position", $Position,
  "--keypair", $Keypair,
  "--slippage-bps", ([string]$SlippageBps)
)
if ($DryRun) { $cliArgs += @("--dry-run") }

Info ("Executing (dry-run=" + ($DryRun.IsPresent) + ")")
$output = & cargo @cliArgs 2>&1

$sig = $null
foreach ($line in $output) {
  if ($line -match "signature:\s*([1-9A-HJ-NP-Za-km-z]{80,})") {
    $sig = $Matches[1]
  }
}
if ($sig) {
  Info ("Signature=" + $sig)
}

if ($Verify) {
  Info "Verifying via orca-positions-list..."
  & cargo run --bin clmm-lp-cli -- orca-positions-list --owner $Owner --keypair $Keypair | Out-Host
}

