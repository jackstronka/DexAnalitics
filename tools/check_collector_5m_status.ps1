# Quick diagnostics for the 5m snapshot collector:
# - last run status JSONL tail
# - loop log tail
# - per Orca curated pool: snapshots_5m.jsonl stats + last ts_utc
param(
    [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Set-Location $RepoRoot

$status = ".\data\snapshot_logs\snapshot-run-curated-all_5m.jsonl"
$loop = ".\data\snapshot_logs\snapshot-loop-5m.log"

if (Test-Path $status) {
    Write-Host "--- status tail (snapshot-run-curated-all_5m.jsonl) ---"
    Get-Content $status -Tail 5
} else {
    Write-Host "MISSING $status"
}

Write-Host ""

if (Test-Path $loop) {
    Write-Host "--- loop log tail (snapshot-loop-5m.log) ---"
    Get-Content $loop -Tail 60
} else {
    Write-Host "MISSING $loop"
}

Write-Host ""

$pools = @(
    "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE", # SOL/USDC
    "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF", # whETH/SOL
    "HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM", # cbBTC/USDC
    "4v8ufj8Hj7UvFgtofQJAtzUud5xomwZfEqfCTHZ4wM72"  # WBTC/cbBTC
)

foreach ($pool in $pools) {
    $path = Join-Path (Join-Path (Join-Path ".\data\pool-snapshots\orca" $pool) "snapshots_5m.jsonl")
    if (!(Test-Path $path)) {
        Write-Host ("MISSING {0} snapshots_5m.jsonl ({1})" -f $pool, $path)
        continue
    }

    $item = Get-Item $path
    $last = Get-Content $path -Tail 1 | ConvertFrom-Json
    Write-Host ("--- {0} snapshots_5m.jsonl ---" -f $pool)
    Write-Host ("  LastWriteTime={0}" -f $item.LastWriteTime.ToString("o"))
    Write-Host ("  LengthBytes={0}" -f $item.Length)
    Write-Host ("  last ts_utc={0} slot={1}" -f $last.ts_utc, $last.slot)
    Write-Host ""
}

