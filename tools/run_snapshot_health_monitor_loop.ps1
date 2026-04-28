# Runs snapshot health-check in a loop (intended for Shawl/NSSM service or a single long-lived terminal).
param(
    [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
    [int] $IntervalSeconds = 60,
    [int] $MaxAgeMinutes10m = 25,
    [int] $MaxAgeMinutes5m = 15,
    [int] $MaxHeartbeatAgeMinutes10m = 22,
    [int] $MaxHeartbeatAgeMinutes5m = 12,
    [int] $ExpectOrcaTarget = 4
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Set-Location $RepoRoot

$logDir = Join-Path $RepoRoot "data/snapshot_logs"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$logFile = Join-Path $logDir "snapshot-health-loop.log"
$mutexName = "Global\clmm-lp-snapshot-health-monitor-loop"
$loopMutex = $null
$hasMutex = $false

function Log([string] $msg) {
    $line = "{0} {1}" -f (Get-Date -Format "o"), $msg
    Add-Content -Path $logFile -Value $line
}

try {
    $loopMutex = New-Object System.Threading.Mutex($false, $mutexName)
    $hasMutex = $loopMutex.WaitOne(0, $false)
    if (-not $hasMutex) {
        Log "lock already held -> start refused (${mutexName})"
        exit 0
    }
} catch {
    Log ("ERROR acquiring lock ${mutexName}: {0}" -f $_)
    throw
}

try {
    Log "loop start; interval=${IntervalSeconds}s; MaxAge10m=${MaxAgeMinutes10m}m MaxAge5m=${MaxAgeMinutes5m}m ExpectOrca=${ExpectOrcaTarget}; lock=${mutexName}"

    while ($true) {
        try {
            $out = & powershell -NoProfile -ExecutionPolicy Bypass -File tools/snapshot_health_check.ps1 `
                -RepoRoot $RepoRoot `
                -MaxAgeMinutes10m $MaxAgeMinutes10m `
                -MaxAgeMinutes5m $MaxAgeMinutes5m `
                -MaxHeartbeatAgeMinutes10m $MaxHeartbeatAgeMinutes10m `
                -MaxHeartbeatAgeMinutes5m $MaxHeartbeatAgeMinutes5m `
                -ExpectOrcaTarget $ExpectOrcaTarget 2>&1
            foreach ($l in @($out)) { Add-Content -Path $logFile -Value $l }
            Log ("exit code: {0}" -f $LASTEXITCODE)
        } catch {
            Log ("ERROR: {0}" -f $_)
        }
        Start-Sleep -Seconds ([Math]::Max(5, $IntervalSeconds))
    }
} finally {
    if ($loopMutex -and $hasMutex) {
        try {
            $loopMutex.ReleaseMutex() | Out-Null
            Log "lock released"
        } catch {
            Log ("ERROR releasing lock: {0}" -f $_)
        }
    }
    if ($loopMutex) {
        $loopMutex.Dispose()
    }
}

