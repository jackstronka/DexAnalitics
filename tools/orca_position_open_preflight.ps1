# Before `orca-position-open`: verify pool mints + wallet has enough token A/B (raw) + SOL for fees.
# Uses `orca-pool-read` + `solana_account_state.ps1` (JSON-RPC only). Exit 0 = OK, 1 = fail.
#
# wSOL (So111...): spendable = native lamports + wSOL ATA raw (open can wrap SOL).
#
# Usage (repo root):
#   . .\tools\mainnet_rpc_env.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_position_open_preflight.ps1 `
#     -Pool HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF `
#     -Keypair "$env:USERPROFILE\.config\solana\clmm_lp_bot_mainnet.json" `
#     -AmountA 1000000 -AmountB 1000
#
# Dot-source to reuse helpers (standalone block below is skipped):
#   . .\tools\orca_position_open_preflight.ps1
#   Test-OrcaPositionOpenPreflight -Pool ... -RepoRoot ... -Owner ... -AmountA ... -AmountB ... -ReserveSolLamports 15000000

param(
  [Parameter(Mandatory = $true)]
  [string] $Pool,

  [string] $Keypair = "",
  [string] $Owner = "",

  [Parameter(Mandatory = $true)]
  [UInt64] $AmountA,

  [Parameter(Mandatory = $true)]
  [UInt64] $AmountB,

  # Minimum native SOL (lamports) to keep for fees + rent headroom (not counted toward wSOL deposit).
  [UInt64] $ReserveSolLamports = 15000000,

  [switch] $CargoOnly,
  [switch] $Quiet
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "orca_position_preflight_core.ps1")

# --- Standalone entry (skipped when dot-sourced so only functions are loaded) ---
if ($MyInvocation.InvocationName -ne '.') {
  $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
  Set-Location $repoRoot

  if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
    $mainnetEnv = Join-Path $PSScriptRoot "mainnet_rpc_env.ps1"
    if (Test-Path -LiteralPath $mainnetEnv) { . $mainnetEnv }
  }
  if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
    throw "[preflight] Set SOLANA_RPC_URL (dot-source tools/mainnet_rpc_env.ps1)."
  }

  if ([string]::IsNullOrWhiteSpace($Keypair)) {
    if ($env:KEYPAIR_PATH) { $Keypair = $env:KEYPAIR_PATH }
    elseif ($env:SOLANA_KEYPAIR_PATH) { $Keypair = $env:SOLANA_KEYPAIR_PATH }
    else { $Keypair = Join-Path $env:USERPROFILE ".config\solana\clmm_lp_bot_mainnet.json" }
  }
  if (-not (Test-Path -LiteralPath $Keypair)) { throw "Keypair not found: $Keypair" }

  if ([string]::IsNullOrWhiteSpace($Owner)) {
    if (Get-Command "solana-keygen" -ErrorAction SilentlyContinue) {
      $Owner = (& solana-keygen pubkey $Keypair | Select-Object -First 1).Trim()
    } else {
      $kpJson = Get-Content -LiteralPath $Keypair -Raw | ConvertFrom-Json
      $bytes = [byte[]]($kpJson | ForEach-Object { [byte]$_ })
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
      $Owner = $sb.ToString()
    }
  }

  try {
    $null = Test-OrcaPositionOpenPreflight -Pool $Pool -RepoRoot $repoRoot -Owner $Owner `
      -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
      -PreferReleaseExe (-not $CargoOnly.IsPresent) -Quiet:$Quiet.IsPresent
    exit 0
  } catch {
    Write-Host $_.Exception.Message
    exit 1
  }
}
