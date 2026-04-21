# Snapshot collectors health check (10m + 5m).
#
# Reads the CLI status JSONL files written by `snapshot-run-curated-all`:
# - data/snapshot_logs/snapshot-run-curated-all.jsonl
# - data/snapshot_logs/snapshot-run-curated-all_5m.jsonl
#
# Optional loop heartbeats (scripts/windows/run-snapshot-loop*.ps1):
# - data/snapshot_logs/snapshot-loop-heartbeat-10m.json
# - data/snapshot_logs/snapshot-loop-heartbeat-5m.json
#
# Emits a JSONL health record under data/snapshot_logs/snapshot-health.jsonl and exits:
# - 0 when healthy
# - 1 when unhealthy (for services/alerts)
param(
    [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,

    # Max allowed age of the last OK run.
    [int] $MaxAgeMinutes10m = 25,
    [int] $MaxAgeMinutes5m = 15,

    # Expect 3 Orca curated pools.
    [int] $ExpectOrcaTarget = 4,

    # If the loop log contains a recent ERROR line, consider unhealthy.
    [int] $RecentErrorLookbackLines = 120,

    # Loop heartbeat files (written each iteration by scripts/windows/run-snapshot-loop*.ps1).
    # If a file exists but its ts_utc is older than this, NOT OK (detects dead/stuck loop process).
    # If the file is absent, checks are skipped (backward compatible until loops are upgraded).
    [int] $MaxHeartbeatAgeMinutes10m = 22,
    [int] $MaxHeartbeatAgeMinutes5m = 12,

    # Optional: write agent-readable alert artifacts when NOT OK.
    # The agent can watch `latest.json` or scan timestamped history.
    [string] $AgentAlertDir = ".\\data\\agent-alerts\\snapshot-health",

    [switch] $Quiet
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Set-Location $RepoRoot

function Read-LastJsonl([string] $path) {
    if (!(Test-Path $path)) { return $null }
    $line = (Get-Content $path -Tail 1)
    if (-not $line) { return $null }
    try { return ($line | ConvertFrom-Json) } catch { return $null }
}

function Read-LastOkJsonl([string] $path, [int] $tailLines) {
    if (!(Test-Path $path)) { return $null }
    $lines = Get-Content $path -Tail $tailLines
    for ($i = $lines.Count - 1; $i -ge 0; $i--) {
        $line = $lines[$i]
        if (-not $line) { continue }
        try {
            $v = $line | ConvertFrom-Json
            if ($v.ok -eq $true) { return $v }
        } catch {}
    }
    return $null
}

function AgeMinutesUtc([string] $tsUtc) {
    if (-not $tsUtc) { return $null }
    try {
        $ts = [datetime]::Parse($tsUtc).ToUniversalTime()
        return ([datetime]::UtcNow - $ts).TotalMinutes
    } catch {
        return $null
    }
}

function HasRecentErrorSince([string] $logPath, [int] $tailLines, [string] $sinceTsUtc) {
    if (!(Test-Path $logPath)) { return $false }
    $tail = Get-Content $logPath -Tail $tailLines
    $since = $null
    if ($sinceTsUtc) {
        try { $since = [datetime]::Parse($sinceTsUtc).ToUniversalTime() } catch { $since = $null }
    }
    foreach ($l in $tail) {
        if ($since) {
            # Our loop logs are `Get-Date -Format o` prefix + message.
            $m = [regex]::Match($l, '^\s*(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2}))\s+')
            if ($m.Success) {
                try {
                    $ts = [datetime]::Parse($m.Groups[1].Value).ToUniversalTime()
                    if ($ts -lt $since) { continue }
                } catch {
                    # If we can't parse the timestamp, fall through and still consider the line.
                }
            }
        }
        if ($l -match "\bERROR\b") { return $true }
    }
    return $false
}

$status10Path = ".\data\snapshot_logs\snapshot-run-curated-all.jsonl"
$status5Path = ".\data\snapshot_logs\snapshot-run-curated-all_5m.jsonl"
$loop10Log = ".\data\snapshot_logs\snapshot-loop.log"
$loop5Log = ".\data\snapshot_logs\snapshot-loop-5m.log"
$heartbeat10Path = ".\data\snapshot_logs\snapshot-loop-heartbeat-10m.json"
$heartbeat5Path = ".\data\snapshot_logs\snapshot-loop-heartbeat-5m.json"

$s10 = Read-LastJsonl $status10Path
$s5 = Read-LastJsonl $status5Path
$s10ok = Read-LastOkJsonl $status10Path 50
$s5ok = Read-LastOkJsonl $status5Path 50

$age10 = if ($s10ok) { AgeMinutesUtc $s10ok.ts_utc } else { $null }
$age5 = if ($s5ok) { AgeMinutesUtc $s5ok.ts_utc } else { $null }

$since10 = if ($s10ok) { $s10ok.ts_utc } else { $null }
$since5 = if ($s5ok) { $s5ok.ts_utc } else { $null }
$err10 = HasRecentErrorSince $loop10Log $RecentErrorLookbackLines $since10
$err5 = HasRecentErrorSince $loop5Log $RecentErrorLookbackLines $since5

function Read-HeartbeatFile([string] $path) {
    if (!(Test-Path $path)) { return $null }
    try { return (Get-Content $path -Raw | ConvertFrom-Json) } catch { return $null }
}

$hb10 = Read-HeartbeatFile $heartbeat10Path
$hb5 = Read-HeartbeatFile $heartbeat5Path
$ageHb10 = if ($hb10 -and $hb10.ts_utc) { AgeMinutesUtc ([string]$hb10.ts_utc) } else { $null }
$ageHb5 = if ($hb5 -and $hb5.ts_utc) { AgeMinutesUtc ([string]$hb5.ts_utc) } else { $null }

$issues = @()

if (-not $s10) { $issues += "missing_or_unparseable_status_10m" }
if (-not $s5) { $issues += "missing_or_unparseable_status_5m" }
if (-not $s10ok) { $issues += "no_ok_run_found_10m_tail50" }
if (-not $s5ok) { $issues += "no_ok_run_found_5m_tail50" }

if ($null -eq $age10) { if ($s10) { $issues += "age_10m_unparseable" } }
elseif ($age10 -gt $MaxAgeMinutes10m) { $issues += ("age_10m_gt_" + $MaxAgeMinutes10m) }

if ($null -eq $age5) { if ($s5) { $issues += "age_5m_unparseable" } }
elseif ($age5 -gt $MaxAgeMinutes5m) { $issues += ("age_5m_gt_" + $MaxAgeMinutes5m) }

if ($s10ok -and $ExpectOrcaTarget -gt 0) {
    if ($s10ok.orca.target -ne $ExpectOrcaTarget) { $issues += "orca_target_10m_unexpected" }
    if ($s10ok.orca.success -ne $ExpectOrcaTarget) { $issues += "orca_success_10m_not_full" }
}
if ($s5ok -and $ExpectOrcaTarget -gt 0) {
    if ($s5ok.orca.target -ne $ExpectOrcaTarget) { $issues += "orca_target_5m_unexpected" }
    if ($s5ok.orca.success -ne $ExpectOrcaTarget) { $issues += "orca_success_5m_not_full" }
}

if ($err10) { $issues += "recent_error_in_snapshot_loop_10m_log" }
if ($err5) { $issues += "recent_error_in_snapshot_loop_5m_log" }

if (Test-Path $heartbeat10Path) {
    if ($null -eq $ageHb10) { $issues += "heartbeat_10m_ts_unparseable" }
    elseif ($ageHb10 -gt $MaxHeartbeatAgeMinutes10m) { $issues += ("heartbeat_10m_stale_gt_" + $MaxHeartbeatAgeMinutes10m) }
}
if (Test-Path $heartbeat5Path) {
    if ($null -eq $ageHb5) { $issues += "heartbeat_5m_ts_unparseable" }
    elseif ($ageHb5 -gt $MaxHeartbeatAgeMinutes5m) { $issues += ("heartbeat_5m_stale_gt_" + $MaxHeartbeatAgeMinutes5m) }
}

$ok = ($issues.Count -eq 0)

$health = [pscustomobject]@{
    ts_utc = ([datetime]::UtcNow.ToString("o"))
    ok = $ok
    issues = $issues
    status = @{
        m10 = @{
            path = $status10Path
            ts_utc_last = if ($s10) { $s10.ts_utc } else { $null }
            ts_utc_last_ok = if ($s10ok) { $s10ok.ts_utc } else { $null }
            age_minutes = if ($null -ne $age10) { [math]::Round($age10, 2) } else { $null }
            ok_last = if ($s10) { $s10.ok } else { $null }
            ok_last_ok = if ($s10ok) { $s10ok.ok } else { $null }
            orca_last = if ($s10) { $s10.orca } else { $null }
            orca_last_ok = if ($s10ok) { $s10ok.orca } else { $null }
        }
        m5 = @{
            path = $status5Path
            ts_utc_last = if ($s5) { $s5.ts_utc } else { $null }
            ts_utc_last_ok = if ($s5ok) { $s5ok.ts_utc } else { $null }
            age_minutes = if ($null -ne $age5) { [math]::Round($age5, 2) } else { $null }
            ok_last = if ($s5) { $s5.ok } else { $null }
            ok_last_ok = if ($s5ok) { $s5ok.ok } else { $null }
            orca_last = if ($s5) { $s5.orca } else { $null }
            orca_last_ok = if ($s5ok) { $s5ok.orca } else { $null }
        }
    }
    logs = @{
        loop10 = @{ path = $loop10Log; recent_error = $err10 }
        loop5 = @{ path = $loop5Log; recent_error = $err5 }
    }
    heartbeats = @{
        m10 = @{
            path         = $heartbeat10Path
            present      = (Test-Path $heartbeat10Path)
            ts_utc       = if ($hb10) { $hb10.ts_utc } else { $null }
            age_minutes  = if ($null -ne $ageHb10) { [math]::Round($ageHb10, 2) } else { $null }
            max_age_min  = $MaxHeartbeatAgeMinutes10m
        }
        m5 = @{
            path         = $heartbeat5Path
            present      = (Test-Path $heartbeat5Path)
            ts_utc       = if ($hb5) { $hb5.ts_utc } else { $null }
            age_minutes  = if ($null -ne $ageHb5) { [math]::Round($ageHb5, 2) } else { $null }
            max_age_min  = $MaxHeartbeatAgeMinutes5m
        }
    }
}

$outDir = ".\data\snapshot_logs"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$outPath = Join-Path $outDir "snapshot-health.jsonl"
($health | ConvertTo-Json -Depth 8 -Compress) | Out-File -FilePath $outPath -Encoding utf8 -Append

# Agent reporting (edge-triggered): write JSON only when state/issue set changes.
#
# Keeps a tiny state file under the agent dir to avoid spamming every minute.
$agentDirAbs = if ([System.IO.Path]::IsPathRooted($AgentAlertDir)) {
    $AgentAlertDir
} else {
    Join-Path $RepoRoot $AgentAlertDir
}
New-Item -ItemType Directory -Force -Path $agentDirAbs | Out-Null
$statePath = Join-Path $agentDirAbs "state.json"
$prev = $null
if (Test-Path $statePath) {
    try { $prev = Get-Content $statePath -Raw | ConvertFrom-Json } catch { $prev = $null }
}
$prevOk = if ($prev) { [bool]$prev.ok } else { $true }
$prevIssues = if ($prev -and $prev.issues) { [string]$prev.issues } else { "" }
$curIssues = ($issues -join ",")

$shouldWrite = $false
if (-not $ok) {
    if ($prevOk -eq $true) { $shouldWrite = $true } # OK -> NOT OK
    elseif ($prevIssues -ne $curIssues) { $shouldWrite = $true } # issue set changed
} else {
    if ($prevOk -eq $false) { $shouldWrite = $true } # NOT OK -> OK (recovery)
}

if ($shouldWrite) {
    $agentDirAbs = if ([System.IO.Path]::IsPathRooted($AgentAlertDir)) {
        $AgentAlertDir
    } else {
        Join-Path $RepoRoot $AgentAlertDir
    }
    $ts = [datetime]::UtcNow.ToString("yyyyMMdd_HHmmss")
    $latest = Join-Path $agentDirAbs "latest.json"
    $suffix = if ($ok) { "_RECOVERY" } else { "" }
    $hist = Join-Path $agentDirAbs (("snapshot_health_" + $ts) + $suffix + ".json")
    $jsonPretty = ($health | ConvertTo-Json -Depth 10)
    $jsonPretty | Out-File -FilePath $latest -Encoding utf8
    $jsonPretty | Out-File -FilePath $hist -Encoding utf8
}

# Persist state for edge-triggering.
@{ ok = $ok; issues = $curIssues; ts_utc = $health.ts_utc } | ConvertTo-Json -Depth 5 | Out-File -FilePath $statePath -Encoding utf8

if (-not $Quiet) {
    if ($ok) {
        Write-Host ("OK snapshot health: 10m age={0}m 5m age={1}m" -f $health.status.m10.age_minutes, $health.status.m5.age_minutes)
    } else {
        Write-Host ("NOT OK snapshot health: issues={0}" -f ($issues -join ", "))
    }
    Write-Host ("health jsonl appended: {0}" -f $outPath)
    if (-not $ok) {
        Write-Host ("agent alert dir: {0}" -f $AgentAlertDir)
    }
}

if ($ok) { exit 0 } else { exit 1 }

