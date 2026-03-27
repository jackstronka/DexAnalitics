param(
  [switch]$RequireKeypair = $false,
  [int]$RpcTimeoutSec = 10
)

$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[bot-preflight] " + $msg) }
function Fail([string]$msg) { throw ("[bot-preflight] " + $msg) }

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

Info "Checking required tools"
$null = Get-Command cargo -ErrorAction Stop

Info "Checking SOLANA_RPC_URL"
if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  Fail "Missing SOLANA_RPC_URL"
}

if ($RequireKeypair) {
  Info "Checking keypair source"
  $keypairPath = $null

  if ($env:KEYPAIR_PATH -and (Test-Path $env:KEYPAIR_PATH)) {
    $keypairPath = $env:KEYPAIR_PATH
  } elseif ($env:SOLANA_KEYPAIR_PATH -and (Test-Path $env:SOLANA_KEYPAIR_PATH)) {
    $keypairPath = $env:SOLANA_KEYPAIR_PATH
  }

  $hasInlineKeypair = $env:SOLANA_KEYPAIR -and -not [string]::IsNullOrWhiteSpace($env:SOLANA_KEYPAIR)
  if (-not $keypairPath -and -not $hasInlineKeypair) {
    Fail "Missing signing key source. Use KEYPAIR_PATH/SOLANA_KEYPAIR_PATH or SOLANA_KEYPAIR."
  }

  if ($keypairPath) {
    Info ("Using keypair file: " + $keypairPath)
  } else {
    Info "Using SOLANA_KEYPAIR from environment"
  }
}

Info "Checking repo layout"
if (-not (Test-Path (Join-Path $repoRoot "Cargo.toml"))) {
  Fail "Cargo.toml not found in repository root"
}

Info "Checking RPC reachability"
$rpcBody = @{
  jsonrpc = "2.0"
  id = 1
  method = "getHealth"
  params = @()
} | ConvertTo-Json -Compress

try {
  $resp = Invoke-RestMethod -Uri $env:SOLANA_RPC_URL -Method Post -Body $rpcBody -ContentType "application/json" -TimeoutSec $RpcTimeoutSec
  if ($null -eq $resp) {
    Fail "RPC returned empty response"
  }
  Info "RPC reachable"
} catch {
  Fail ("RPC check failed: " + $_.Exception.Message)
}

Info "Running cargo --version"
& cargo --version | Out-Null

Info "Preflight OK"
