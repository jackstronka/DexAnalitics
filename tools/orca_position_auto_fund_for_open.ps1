# Run on-pool exact-out swaps until `orca-position-open` preflight would pass (token legs only).
# Does not send `orca-position-open`. Use before manual open, or use `orca_position_open_then_close_quick.ps1 -AutoFund`.
#
# Requires: enough of the *counter* token to pay each swap; native SOL must already meet -ReserveSolLamports.
#
# Usage (repo root):
#   . .\tools\mainnet_rpc_env.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_position_auto_fund_for_open.ps1 `
#     -Pool HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF `
#     -Keypair "$env:USERPROFILE\.config\solana\clmm_lp_bot_mainnet.json" `
#     -AmountA 1000000 -AmountB 1000

param(
  [Parameter(Mandatory = $true)][string] $Pool,
  [string] $Keypair = "",
  [string] $Owner = "",
  [Parameter(Mandatory = $true)][UInt64] $AmountA,
  [Parameter(Mandatory = $true)][UInt64] $AmountB,
  [UInt64] $ReserveSolLamports = 15000000,
  [UInt32] $MaxRounds = 8,
  [UInt16] $SwapSlippageBps = 150,
  [UInt32] $DeficitBufferBps = 100,
  [switch] $CargoOnly,
  [switch] $Quiet
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  $mainnetEnv = Join-Path $PSScriptRoot "mainnet_rpc_env.ps1"
  if (Test-Path -LiteralPath $mainnetEnv) { . $mainnetEnv }
}
if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  throw "[auto-fund] Set SOLANA_RPC_URL (dot-source tools/mainnet_rpc_env.ps1)."
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
    throw "[auto-fund] Set -Owner or install solana-keygen to derive pubkey from keypair."
  }
}

. (Join-Path $PSScriptRoot "orca_position_preflight_core.ps1")

$preferExe = -not $CargoOnly.IsPresent
try {
  Invoke-OrcaPositionAutoFundFromPool -Pool $Pool -RepoRoot $repoRoot -Owner $Owner -Keypair $Keypair `
    -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
    -PreferReleaseExe $preferExe -MaxRounds $MaxRounds -SwapSlippageBps $SwapSlippageBps `
    -DeficitBufferBps $DeficitBufferBps -Quiet:$Quiet.IsPresent
  $null = Test-OrcaPositionOpenPreflight -Pool $Pool -RepoRoot $repoRoot -Owner $Owner `
    -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
    -PreferReleaseExe $preferExe -Quiet:$Quiet.IsPresent
  exit 0
} catch {
  Write-Host $_.Exception.Message
  exit 1
}
