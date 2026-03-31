# Compares backtest results for Orca curated pools using snapshot JSONL at 5m vs 10m cadence.
# Window: last full hour in UTC (e.g. 10:00:00Z..11:00:00Z).
# Ranges: ±1%, ±2%, ±3% around entry price (first price in the window).
#
# Notes:
# - Requires snapshot files already collected (snapshots.jsonl and snapshots_5m.jsonl).
# - Uses CLI output parsing (prettytable) to extract Entry Price / Net PnL / Fees / Time in Range.
#
param(
    [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
    [string] $Configuration = "release",
    [double] $Capital = 1000.0,
    # Optional: speed up by using snapshot-backtest-prep cache (still intersected with the chosen time window).
    # Example: -PreparedSnapshotWindow h24
    [string] $PreparedSnapshotWindow = "",
    # Optional: run snapshot-backtest-prep for both 10m and 5m before backtests (requires local snapshot JSONL files).
    [switch] $RunSnapshotBacktestPrep,
    # Windows for snapshot-backtest-prep (used only when -RunSnapshotBacktestPrep is set).
    [string] $PrepWindowsHours = "24,48,96",
    [string] $PrepWindowsDays = "7,30",
    # Default expectations: 5m should have ~12 rows/hour; we accept >=8.
    # 10m should have ~6 rows/hour; we accept >=4.
    [int] $MinRowsInWindow5m = 8,
    [int] $MinRowsInWindow10m = 4,
    [switch] $Quiet
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Exe = Join-Path $RepoRoot "target\$Configuration\clmm-lp-cli.exe"
if (-not (Test-Path $Exe)) {
    throw "Binary not found: $Exe (build with cargo build --release --bin clmm-lp-cli)"
}

function Utc-RoundToHour([datetime] $dt) {
    $u = $dt.ToUniversalTime()
    return [datetime]::SpecifyKind($u.Date.AddHours($u.Hour), [DateTimeKind]::Utc)
}

$nowUtc = (Get-Date).ToUniversalTime()
$endUtc = Utc-RoundToHour $nowUtc
$startUtc = $endUtc.AddHours(-1)
$startStr = $startUtc.ToString("yyyy-MM-ddTHH:mm:ssZ")
$endStr = $endUtc.ToString("yyyy-MM-ddTHH:mm:ssZ")

$pairs = @(
    @{
        name = "SOL/USDC"
        pool = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"
        symbolA = "SOL"
        symbolB = "USDC"
    },
    @{
        name = "whETH/SOL"
        pool = "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF"
        symbolA = "whETH"
        symbolB = "SOL"
    },
    @{
        name = "cbBTC/USDC"
        pool = "HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM"
        symbolA = "cbBTC"
        symbolB = "USDC"
    }
)

$widthPcts = @(0.01, 0.02, 0.03)
$variants = @(
    @{ label = "5m"; suffix = "5m" },
    @{ label = "10m"; suffix = $null }
)

function Get-SnapshotPath([hashtable] $pair, [string] $suffix) {
    $file = if ($suffix) { "snapshots_${suffix}.jsonl" } else { "snapshots.jsonl" }
    return Join-Path (Join-Path (Join-Path $RepoRoot "data/pool-snapshots/orca") $pair.pool) $file
}

function Get-OrcaPoolMints([string] $pool) {
    # Use the canonical 10m file to discover the pool mints (works even if 5m is new/missing).
    $path = Join-Path (Join-Path (Join-Path $RepoRoot "data/pool-snapshots/orca") $pool) "snapshots.jsonl"
    if (!(Test-Path $path)) { throw "Missing snapshot file for mint discovery: $path" }
    $last = Get-Content $path -Tail 1 | ConvertFrom-Json
    if (-not $last.token_mint_a -or -not $last.token_mint_b) {
        throw "Could not read token_mint_a/token_mint_b from $path (tail=1)"
    }
    return @{ mintA = [string]$last.token_mint_a; mintB = [string]$last.token_mint_b }
}

function Count-RowsInWindow([string] $path, [datetime] $startUtc, [datetime] $endUtc) {
    if (-not (Test-Path $path)) { return 0 }
    $cnt = 0
    Get-Content $path | ForEach-Object {
        try {
            $v = $_ | ConvertFrom-Json
            $ts = [datetime]::Parse($v.ts_utc).ToUniversalTime()
            if ($ts -ge $startUtc -and $ts -lt $endUtc) { $cnt++ }
        } catch {}
    }
    return $cnt
}

function Invoke-Backtest(
    [hashtable] $pair,
    [string] $suffix,
    [double] $lower,
    [double] $upper
) {
    $m = Get-OrcaPoolMints $pair.pool
    $args = @(
        "backtest",
        "--symbol-a", $pair.symbolA,
        "--mint-a", $m.mintA,
        "--symbol-b", $pair.symbolB,
        "--mint-b", $m.mintB,
        "--capital", "$Capital",
        "--start-date", $startStr,
        "--end-date", $endStr,
        "--lower", ("{0:R}" -f $lower),
        "--upper", ("{0:R}" -f $upper),
        "--strategy", "static",
        "--price-path-source", "snapshots",
        "--snapshot-protocol", "orca",
        "--snapshot-pool-address", $pair.pool,
        "--fee-source", "snapshots"
    )

    if ($suffix) {
        $args += @("--snapshot-jsonl-suffix", $suffix)
    }

    if ($PreparedSnapshotWindow -and $PreparedSnapshotWindow.Trim()) {
        $args += @("--prepared-snapshot-window", $PreparedSnapshotWindow.Trim())
    }

    if (-not $Quiet) {
        $s = if ($suffix) { $suffix } else { "10m" }
        $psw = if ($PreparedSnapshotWindow -and $PreparedSnapshotWindow.Trim()) { $PreparedSnapshotWindow.Trim() } else { "-" }
        Write-Host "[$(Get-Date -Format o)] backtest $($pair.name) suffix=$s window=$startStr..$endStr lower=$lower upper=$upper prepared_window=$psw"
    }

    $out = & $Exe @args 2>&1
    return @($out | ForEach-Object { "$_" })
}

function Invoke-SnapshotBacktestPrep([string] $suffix, [string[]] $pools) {
    $args = @(
        "snapshot-backtest-prep",
        "--pools", ($pools -join ","),
        "--windows-hours", $PrepWindowsHours,
        "--windows-days", $PrepWindowsDays
    )
    if ($suffix) {
        $args += @("--snapshots-suffix", $suffix)
    }
    if (-not $Quiet) {
        $s = if ($suffix) { $suffix } else { "10m" }
        Write-Host "[$(Get-Date -Format o)] snapshot-backtest-prep suffix=$s windows_hours=$PrepWindowsHours windows_days=$PrepWindowsDays"
    }
    $null = & $Exe @args 2>&1
}

function Parse-EntryPrice([string[]] $lines) {
    foreach ($l in $lines) {
        if ($l -match 'Entry Price.*\$(\d+(\.\d+)?)') {
            return [double]$Matches[1]
        }
    }
    throw "Could not parse Entry Price from output"
}

function Parse-MetricUsd([string[]] $lines, [string] $label) {
    foreach ($l in $lines) {
        if ($l -match [regex]::Escape($label)) {
            if ($l -match '\$([+-]?\d+(\.\d+)?)') {
                return [double]$Matches[1]
            }
        }
    }
    return $null
}

function Parse-TimeInRangePct([string[]] $lines) {
    foreach ($l in $lines) {
        if ($l -match 'Time in Range.*(\d+(\.\d+)?)%') {
            return [double]$Matches[1]
        }
    }
    return $null
}

$tsTag = (Get-Date).ToUniversalTime().ToString("yyyyMMdd_HHmmss")
$outDir = Join-Path $RepoRoot "data/reports"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$csvPath = Join-Path $outDir "compare_orca_snapshots_5m_vs_10m_last_full_hour_$tsTag.csv"

$rows = New-Object System.Collections.Generic.List[object]

$poolList = @($pairs | ForEach-Object { [string]$_.pool })
if ($RunSnapshotBacktestPrep) {
    Invoke-SnapshotBacktestPrep -suffix $null -pools $poolList
    Invoke-SnapshotBacktestPrep -suffix "5m" -pools $poolList
}

foreach ($pair in $pairs) {
    foreach ($variant in $variants) {
        # Preflight: require a sensible number of rows in the window per cadence.
        $snapPath = Get-SnapshotPath -pair $pair -suffix $variant.suffix
        if (-not (Test-Path $snapPath)) {
            throw "Missing snapshot file: $snapPath. Run: .\\target\\release\\clmm-lp-cli.exe snapshot-run-curated-all --snapshots-suffix $($variant.label)"
        }
        $rowsInWin = Count-RowsInWindow -path $snapPath -startUtc $startUtc -endUtc $endUtc
        $minNeed = if ($variant.label -eq "5m") { $MinRowsInWindow5m } else { $MinRowsInWindow10m }
        if ($rowsInWin -lt $minNeed) {
            throw "Not enough snapshot rows in last full hour for $($pair.name) variant=$($variant.label): rows_in_window=$rowsInWin (need >= $minNeed)."
        }

        # 1) Determine entry price for this variant/window using a very wide range.
        $wideLower = 0.0000001
        $wideUpper = 1.0e15
        $probe = Invoke-Backtest -pair $pair -suffix $variant.suffix -lower $wideLower -upper $wideUpper
        $entry = Parse-EntryPrice $probe

        foreach ($w in $widthPcts) {
            $lower = $entry * (1.0 - $w)
            $upper = $entry * (1.0 + $w)
            $out = Invoke-Backtest -pair $pair -suffix $variant.suffix -lower $lower -upper $upper

            $netPnl = Parse-MetricUsd $out "Net PnL"
            $fees = Parse-MetricUsd $out "Fees Earned"
            $tir = Parse-TimeInRangePct $out

            $rows.Add([pscustomobject]@{
                ts_utc = (Get-Date).ToUniversalTime().ToString("o")
                window_start_utc = $startStr
                window_end_utc = $endStr
                pair = $pair.name
                pool = $pair.pool
                variant = $variant.label
                snapshot_suffix = $variant.suffix
                entry_price_usd = $entry
                width_pct = ($w * 100.0)
                lower = $lower
                upper = $upper
                net_pnl_usd = $netPnl
                fees_usd = $fees
                time_in_range_pct = $tir
            }) | Out-Null
        }
    }
}

# Write CSV
$rows | Export-Csv -NoTypeInformation -Encoding utf8 -Path $csvPath

# Quick diff view (5m vs 10m) by pair+width
if (-not $Quiet) {
    Write-Host ""
    Write-Host "Saved CSV: $csvPath"
    Write-Host ""
    $grouped = $rows | Group-Object pair, width_pct
    foreach ($g in $grouped) {
        $items = $g.Group
        $r5 = $items | Where-Object { $_.variant -eq "5m" } | Select-Object -First 1
        $r10 = $items | Where-Object { $_.variant -eq "10m" } | Select-Object -First 1
        if ($null -eq $r5 -or $null -eq $r10) { continue }

        $dpnl = $r5.net_pnl_usd - $r10.net_pnl_usd
        $dfees = $r5.fees_usd - $r10.fees_usd
        $dtir = $r5.time_in_range_pct - $r10.time_in_range_pct

        Write-Host ("{0} width={1}%  dPnL=${2:N2}  dFees=${3:N2}  dTIR={4:N1}pp" -f $r5.pair, $r5.width_pct, $dpnl, $dfees, $dtir)
    }
}

