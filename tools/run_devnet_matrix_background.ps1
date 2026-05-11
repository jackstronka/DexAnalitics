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
  [string]$OutDir = "",
  [string]$Tag = ""
)

$ErrorActionPreference = "Stop"
function Info([string]$msg) { Write-Host ("[devnet-bg] " + $msg) }

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

$ts = Get-Date -Format "yyyyMMdd-HHmmss"
if ([string]::IsNullOrWhiteSpace($OutDir)) {
  $OutDir = Join-Path $repoRoot ("data\\reports\\devnet-bg\\" + $ts)
}
$null = New-Item -ItemType Directory -Force -Path $OutDir

$tagSafe = $Tag.Trim()
if ([string]::IsNullOrWhiteSpace($tagSafe)) { $tagSafe = "devnet" }
$logPath = Join-Path $OutDir ($tagSafe + "_smokes.log")
$reportPath = Join-Path $OutDir ($tagSafe + "_smokes_report.json")

Info ("OutDir: " + $OutDir)
Info ("Log: " + $logPath)
Info ("Report: " + $reportPath)

# Ensure env is present for the child process (script also takes args explicitly).
$env:SOLANA_RPC_URL = $SolanaRpcUrl
$env:SOLANA_RPC_FALLBACK_URLS = $SolanaRpcFallbackUrls
$env:KEYPAIR_PATH = $KeypairPath
$env:DEVNET_POOL_ADDRESS = $DevnetPoolAddress
$env:DEVNET_TICK_LOWER = $DevnetTickLower
$env:DEVNET_TICK_UPPER = $DevnetTickUpper
$env:DEVNET_OPEN_AMOUNT_A = $DevnetOpenAmountA
$env:DEVNET_OPEN_AMOUNT_B = $DevnetOpenAmountB

$smokesScript = Join-Path $PSScriptRoot "run_devnet_smokes.ps1"
$walletSetupFlag = if ($WalletSetup) { "-WalletSetup" } else { "" }

# Spawn a separate PowerShell that runs the smokes script and logs to file.
$cmd = @"
`$ErrorActionPreference='Stop'
cd '$repoRoot'
& '$smokesScript' -KeypairPath '$KeypairPath' -SolanaRpcUrl '$SolanaRpcUrl' -SolanaRpcFallbackUrls '$SolanaRpcFallbackUrls' -DevnetPoolAddress '$DevnetPoolAddress' -DevnetTickLower $DevnetTickLower -DevnetTickUpper $DevnetTickUpper -DevnetOpenAmountA $DevnetOpenAmountA -DevnetOpenAmountB $DevnetOpenAmountB $walletSetupFlag -ReportPath '$reportPath'
exit `$LASTEXITCODE
"@

$proc = Start-Process -FilePath "powershell" -ArgumentList @(
  "-NoProfile",
  "-ExecutionPolicy","Bypass",
  "-Command", $cmd
) -RedirectStandardOutput $logPath -RedirectStandardError $logPath -PassThru -NoNewWindow

Info ("Started devnet smokes in background. PID=" + $proc.Id)
Info "To check status: Get-Process -Id <PID> (running) or check report/log files."
Info "To follow logs: Get-Content -Path <log> -Wait"
