param(
  [string]$KeypairPath = "",
  [string]$SolanaRpcUrl = "https://api.devnet.solana.com",
  [string]$SolanaRpcFallbackUrls = "https://api.devnet.solana.com",
  [string]$DevnetPoolAddress = "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt",
  [int]$DevnetTickLower = -128,
  [int]$DevnetTickUpper = 128,
  [long]$DevnetOpenAmountA = 1000000,
  [long]$DevnetOpenAmountB = 1000,
  [switch]$WalletSetup = $false,
  [string]$ReportPath = ""
)

$ErrorActionPreference = "Stop"
function Info([string]$msg) { Write-Host ("[devnet-smokes] " + $msg) }

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

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
Set-Item -Path Env:SOLANA_RPC_FALLBACK_URLS -Value $SolanaRpcFallbackUrls
Set-Item -Path Env:KEYPAIR_PATH -Value $KeypairPath
Set-Item -Path Env:DEVNET_POOL_ADDRESS -Value $DevnetPoolAddress
Set-Item -Path Env:DEVNET_TICK_LOWER -Value $DevnetTickLower
Set-Item -Path Env:DEVNET_TICK_UPPER -Value $DevnetTickUpper
Set-Item -Path Env:DEVNET_OPEN_AMOUNT_A -Value $DevnetOpenAmountA
Set-Item -Path Env:DEVNET_OPEN_AMOUNT_B -Value $DevnetOpenAmountB

Info ("RPC: " + $env:SOLANA_RPC_URL)
Info ("Keypair: " + $env:KEYPAIR_PATH)
Info ("Pool: " + $env:DEVNET_POOL_ADDRESS + " ticks=[" + $env:DEVNET_TICK_LOWER + "," + $env:DEVNET_TICK_UPPER + "]")
Info ("Open caps: amount_a=" + $env:DEVNET_OPEN_AMOUNT_A + " amount_b=" + $env:DEVNET_OPEN_AMOUNT_B)

if ($WalletSetup) {
  Info "Running devnet_wallet_setup.ps1 (airdrop + optional env hints)..."
  & (Join-Path $PSScriptRoot "devnet_wallet_setup.ps1") -KeypairPath $KeypairPath -SolanaRpcUrl $SolanaRpcUrl -Pool $DevnetPoolAddress
}

& cargo test -p clmm-lp-api devnet_ -- --ignored
$exitCode = $LASTEXITCODE

if (-not [string]::IsNullOrWhiteSpace($ReportPath)) {
  $parent = Split-Path -Parent $ReportPath
  if (-not [string]::IsNullOrWhiteSpace($parent)) {
    $null = New-Item -ItemType Directory -Force -Path $parent
  }
  @{
    timestamp = (Get-Date).ToUniversalTime().ToString("o")
    exit_code = $exitCode
    pool      = $DevnetPoolAddress
    rpc       = $SolanaRpcUrl
  } | ConvertTo-Json | Set-Content -Path $ReportPath -Encoding UTF8
  Info ("Wrote report: " + $ReportPath)
}

if ($exitCode -ne 0) {
  throw ("devnet smokes failed (exit=" + $exitCode + ")")
}

Info "devnet smokes OK"

