#Requires -Version 5.1
<#
.SYNOPSIS
  Runs `snapshot-run-curated-all` (refresh JSONL) then `snapshot-backtest-prep` on an interval.
  Schedule this script with Windows Task Scheduler every 30 minutes (or run as a loop).

.DESCRIPTION
  Prepares `data/backtest-snapshot-cache/orca/<POOL>/window_h24.jsonl` etc. so
  `backtest --price-path-source snapshots --prepared-snapshot-window h24` reads a small file quickly.

.PARAMETER RepoRoot
  Repository root (contains Cargo.toml). Default: parent of tools/

.PARAMETER IntervalMinutes
  When -Loop is set, sleep this many minutes between cycles. Default: 30.

.PARAMETER SkipSnapshots
  If set, only run snapshot-backtest-prep (use when another job already collects snapshots).

.PARAMETER Loop
  If set, repeat forever; otherwise run one shot (for Task Scheduler one-shot triggers).

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File tools/run_snapshot_backtest_prep_loop.ps1

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File tools/run_snapshot_backtest_prep_loop.ps1 -Loop -IntervalMinutes 30
#>
param(
    [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
    [int] $IntervalMinutes = 30,
    [switch] $SkipSnapshots,
    [switch] $SkipSnapshotReadiness,
    [switch] $Loop,
    # Optional: set RPC env for snapshot collectors (recommended)
    [string] $SolanaRpcUrl = "",
    [string] $SolanaRpcFallbackUrls = "",
    [string] $ExpectedCluster = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Run-Once {
    Push-Location $RepoRoot
    try {
        if (-not [string]::IsNullOrWhiteSpace($SolanaRpcUrl)) {
            . (Join-Path $RepoRoot "tools/solana_rpc_env.ps1")
            Set-SolanaRpcEnv -SolanaRpcUrl $SolanaRpcUrl -SolanaRpcFallbackUrls $SolanaRpcFallbackUrls -ExpectedCluster $ExpectedCluster
        }

        . (Join-Path $RepoRoot "tools/clmm_rpc_tools_helpers.ps1")
        if (Initialize-ClmmToolsRpcEnv) {
            Write-Host "[$(Get-Date -Format o)] rpc: default CLMM_RPC_DENYLIST for mainnet (ankr,projectserum)."
        }

        if (-not $SkipSnapshots) {
            Write-Host "[$(Get-Date -Format o)] snapshot-run-curated-all..."
            & cargo run -q -p clmm-lp-cli --bin clmm-lp-cli -- snapshot-run-curated-all
            if ($LASTEXITCODE -ne 0) { throw "snapshot-run-curated-all failed: $LASTEXITCODE" }
        }

        Write-Host "[$(Get-Date -Format o)] snapshot-backtest-prep..."
        & cargo run -q -p clmm-lp-cli --bin clmm-lp-cli -- snapshot-backtest-prep
        if ($LASTEXITCODE -ne 0) { throw "snapshot-backtest-prep failed: $LASTEXITCODE" }

        if (-not $SkipSnapshotReadiness) {
            $orcaPools = @(
                "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE", # SOL/USDC
                "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF"  # whETH/SOL
            )

            foreach ($p in $orcaPools) {
                Write-Host "[$(Get-Date -Format o)] snapshot-readiness (orca, pool=$p)..."
                $out = (& cargo run --quiet --bin clmm-lp-cli -- snapshot-readiness --protocol orca --pool-address $p) 2>&1
                $lines = @($out | ForEach-Object { "$_" })

                $tier1 = "UNKNOWN"
                $tier2 = "UNKNOWN"
                foreach ($line in $lines) {
                    if ($line -match "^\s*1\).+:\s+(READY|NOT READY)") { $tier1 = $Matches[1] }
                    if ($line -match "^\s*2\).+:\s+(READY|NOT READY)") { $tier2 = $Matches[1] }
                }

                Write-Host "  readiness: tier1=$tier1 tier2=$tier2"
                if ($tier1 -ne "READY" -or $tier2 -ne "READY") {
                    throw "snapshot-readiness failed for pool=$p (tier1=$tier1 tier2=$tier2). Fix snapshots and re-run."
                }
            }
        }

        $logDir = Join-Path $RepoRoot "data/snapshot_logs"
        if (-not (Test-Path $logDir)) { New-Item -ItemType Directory -Path $logDir | Out-Null }
        $statusPath = Join-Path $logDir "snapshot-backtest-prep-ready.jsonl"
        $status = @{
            ts_utc = (Get-Date).ToUniversalTime().ToString("o")
            snapshots_prepared = (-not $SkipSnapshots)
            snapshot_readiness_checked = (-not $SkipSnapshotReadiness)
        }
        $status | ConvertTo-Json -Depth 8 | Out-File -FilePath $statusPath -Encoding utf8 -Append

        Write-Host "[$(Get-Date -Format o)] done."
    } finally {
        Pop-Location
    }
}

do {
    Run-Once
    if (-not $Loop) { break }
    Write-Host "Sleeping $IntervalMinutes minutes..."
    Start-Sleep -Seconds ($IntervalMinutes * 60)
} while ($true)
