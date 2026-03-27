param(
  [Parameter(Mandatory = $true)]
  [string]$Position,
  [ValidateSet("dry-run", "limited-live")]
  [string]$Mode = "dry-run",
  [int]$EvalIntervalSecs = 300,
  [int]$PollIntervalSecs = 30,
  [string]$Notes = "",
  [string]$Signatures = ""
)

$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[bot-postrun] " + $msg) }

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$reportsDir = Join-Path $repoRoot "data/reports"
New-Item -ItemType Directory -Path $reportsDir -Force | Out-Null

$sigList = @()
if (-not [string]::IsNullOrWhiteSpace($Signatures)) {
  $sigList = $Signatures.Split(",") | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne "" }
}

$keypairSource = "none"
if ($env:KEYPAIR_PATH -and -not [string]::IsNullOrWhiteSpace($env:KEYPAIR_PATH)) {
  $keypairSource = "KEYPAIR_PATH"
} elseif ($env:SOLANA_KEYPAIR_PATH -and -not [string]::IsNullOrWhiteSpace($env:SOLANA_KEYPAIR_PATH)) {
  $keypairSource = "SOLANA_KEYPAIR_PATH"
} elseif ($env:SOLANA_KEYPAIR -and -not [string]::IsNullOrWhiteSpace($env:SOLANA_KEYPAIR)) {
  $keypairSource = "SOLANA_KEYPAIR"
}

$report = [pscustomobject]@{
  ts_utc = (Get-Date).ToUniversalTime().ToString("o")
  mode = $Mode
  position = $Position
  runtime_config = @{
    eval_interval_secs = $EvalIntervalSecs
    poll_interval_secs = $PollIntervalSecs
  }
  env = @{
    solana_rpc_url = $env:SOLANA_RPC_URL
    solana_rpc_fallback_urls = $env:SOLANA_RPC_FALLBACK_URLS
    keypair_source = $keypairSource
  }
  tx = @{
    signatures = $sigList
    signatures_count = $sigList.Count
  }
  notes = $Notes
}

$stamp = Get-Date -Format "yyyyMMdd_HHmmss"
$outPath = Join-Path $reportsDir ("bot_postrun_" + $stamp + ".json")
$report | ConvertTo-Json -Depth 8 | Out-File -FilePath $outPath -Encoding utf8

Info ("Saved report: " + $outPath)
Write-Host ("report: " + $outPath)
